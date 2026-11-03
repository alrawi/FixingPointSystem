// SPDX-License-Identifier: GPL-3.0-only
//! Load phases and the target fraction over time.
//!
//! A [`LoadProfile`] is the ordered list of load phases of a plan. Each phase
//! scales the steady target of a worker rate from its start fraction to its end
//! fraction with its [`Shape`]. The profile answers three questions for the
//! schedule, which are the fraction at a time, the integral of the fraction up
//! to a time, and the inverse of that integral. Times are seconds from the run
//! start as `f64`, and the public conversions to [`Duration`] live in
//! [`crate::schedule`].

use std::time::Duration;

use crate::SchedError;

/// How the fraction moves within one phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// The fraction stays at the start fraction for the whole phase.
    Hold,
    /// The fraction moves linearly from the start fraction to the end fraction.
    Linear,
    /// The fraction moves from the start fraction to the end fraction in equal
    /// steps. Step `i` of `n`, counting from 0, holds the fraction
    /// `start + (end - start) * i / n`, so the phase begins at the start
    /// fraction and the end fraction takes effect where the phase ends.
    Step {
        /// The number of equal steps, at least 1.
        steps: u32,
    },
}

impl Shape {
    /// Parses a shape name of the test configuration contract, which is
    /// `hold`, `linear`, or `step`. A step shape parsed from a name carries the
    /// given number of steps.
    pub fn from_contract_name(name: &str, steps: u32) -> Result<Self, SchedError> {
        match name {
            "hold" => Ok(Shape::Hold),
            "linear" => Ok(Shape::Linear),
            "step" => Ok(Shape::Step { steps }),
            other => Err(SchedError::UnknownShape(other.to_owned())),
        }
    }
}

/// One load phase, as the `LoadPhase` message of the agent plan defines it.
#[derive(Debug, Clone, PartialEq)]
pub struct Phase {
    /// The phase name from the test configuration.
    pub name: String,
    /// The length of the phase.
    pub duration: Duration,
    /// How the fraction moves within the phase.
    pub shape: Shape,
    /// The fraction of the steady target at the phase start.
    pub start_fraction: f64,
    /// The fraction of the steady target at the phase end. A hold phase
    /// ignores it.
    pub end_fraction: f64,
}

impl Phase {
    /// Returns a phase with the given name, duration, shape, and fractions.
    pub fn new(
        name: impl Into<String>,
        duration: Duration,
        shape: Shape,
        start_fraction: f64,
        end_fraction: f64,
    ) -> Self {
        Self {
            name: name.into(),
            duration,
            shape,
            start_fraction,
            end_fraction,
        }
    }
}

/// The validated phases of one plan with precomputed boundaries.
#[derive(Debug, Clone)]
pub struct LoadProfile {
    phases: Vec<Phase>,
    /// Start time of each phase in seconds, plus the total duration at the end.
    starts: Vec<f64>,
    /// Integral of the fraction up to each phase start, plus the total at the end.
    mass: Vec<f64>,
}

impl LoadProfile {
    /// Validates the phases and precomputes their boundaries.
    pub fn new(phases: Vec<Phase>) -> Result<Self, SchedError> {
        if phases.is_empty() {
            return Err(SchedError::NoPhases);
        }
        let mut starts = Vec::with_capacity(phases.len() + 1);
        let mut mass = Vec::with_capacity(phases.len() + 1);
        let (mut t, mut m) = (0.0, 0.0);
        for (index, p) in phases.iter().enumerate() {
            if p.duration.is_zero() {
                return Err(SchedError::ZeroDuration {
                    index,
                    name: p.name.clone(),
                });
            }
            for (field, value) in [
                ("start_fraction", p.start_fraction),
                ("end_fraction", p.end_fraction),
            ] {
                if !value.is_finite() || value < 0.0 {
                    return Err(SchedError::InvalidFraction {
                        index,
                        name: p.name.clone(),
                        field,
                        value,
                    });
                }
            }
            if p.shape == (Shape::Step { steps: 0 }) {
                return Err(SchedError::ZeroSteps {
                    index,
                    name: p.name.clone(),
                });
            }
            starts.push(t);
            mass.push(m);
            let d = p.duration.as_secs_f64();
            t += d;
            m += local_mass(p, d);
        }
        starts.push(t);
        mass.push(m);
        Ok(Self {
            phases,
            starts,
            mass,
        })
    }

    /// Returns the phases in order.
    pub fn phases(&self) -> &[Phase] {
        &self.phases
    }

    /// Returns the sum of all phase durations in seconds.
    pub fn total_secs(&self) -> f64 {
        self.starts[self.phases.len()]
    }

    /// Returns the start time of phase `index` in seconds, or the total
    /// duration for an index past the last phase.
    pub fn phase_start_secs(&self, index: usize) -> f64 {
        self.starts[index.min(self.phases.len())]
    }

    /// Returns the integral of the fraction over the whole profile in seconds.
    pub fn total_mass(&self) -> f64 {
        self.mass[self.phases.len()]
    }

    /// Returns the index of the phase that contains time `t`. A phase covers
    /// its start and excludes its end. Times before 0 belong to the first phase,
    /// and times at or after the end belong to no phase.
    pub fn phase_index_at(&self, t: f64) -> Option<usize> {
        if t >= self.total_secs() {
            return None;
        }
        // The first boundary after t, minus one, is the phase that holds t.
        let after = self.starts.partition_point(|&s| s <= t);
        Some(after.saturating_sub(1).min(self.phases.len() - 1))
    }

    /// Returns the phase that contains time `t`, or `None` after the end.
    pub fn phase_at(&self, t: f64) -> Option<&Phase> {
        self.phase_index_at(t).map(|i| &self.phases[i])
    }

    /// Returns the target fraction at time `t`, which is 0 after the end.
    pub fn fraction_at(&self, t: f64) -> f64 {
        match self.phase_index_at(t) {
            Some(i) => local_fraction(&self.phases[i], (t - self.starts[i]).max(0.0)),
            None => 0.0,
        }
    }

    /// Returns the integral of the fraction from 0 to `t` in seconds.
    pub fn mass_at(&self, t: f64) -> f64 {
        match self.phase_index_at(t) {
            Some(i) => {
                let u = (t - self.starts[i]).max(0.0);
                self.mass[i] + local_mass(&self.phases[i], u)
            }
            None => self.total_mass(),
        }
    }

    /// Returns the earliest time at which the integral of the fraction reaches
    /// `m`, or `None` when the whole profile holds less than `m`. A value of
    /// `m` at or below 0 maps to time 0.
    pub fn time_at_mass(&self, m: f64) -> Option<f64> {
        if m <= 0.0 {
            return Some(0.0);
        }
        if m > self.total_mass() {
            return None;
        }
        // The first phase whose end mass reaches m holds the time. Its start
        // mass lies below m, so the phase has a positive mass.
        let i = self.mass[1..].partition_point(|&end| end < m);
        let i = i.min(self.phases.len() - 1);
        let p = &self.phases[i];
        let d = p.duration.as_secs_f64();
        let u = local_time_at_mass(p, d, m - self.mass[i]);
        Some(self.starts[i] + u.clamp(0.0, d))
    }

    /// Returns the earliest time at or after `from` at which the fraction is at
    /// least `level` and that lies strictly inside a phase, or `None` when the
    /// fraction stays below `level` until the end.
    pub fn first_time_at_or_above(&self, level: f64, from: f64) -> Option<f64> {
        let first = self.phase_index_at(from.max(0.0))?;
        for i in first..self.phases.len() {
            let p = &self.phases[i];
            let d = p.duration.as_secs_f64();
            let u0 = (from - self.starts[i]).max(0.0);
            if let Some(u) = local_first_at_or_above(p, d, level, u0) {
                return Some(self.starts[i] + u);
            }
        }
        None
    }
}

/// The fraction of phase `p` at local time `u`.
fn local_fraction(p: &Phase, u: f64) -> f64 {
    let (s, e) = (p.start_fraction, p.end_fraction);
    let d = p.duration.as_secs_f64();
    match p.shape {
        Shape::Hold => s,
        Shape::Linear => s + (e - s) * (u / d).min(1.0),
        Shape::Step { steps } => {
            let n = f64::from(steps);
            let i = step_index(u, d, steps);
            s + (e - s) * f64::from(i) / n
        }
    }
}

/// The step of a step phase that holds local time `u`.
fn step_index(u: f64, d: f64, steps: u32) -> u32 {
    let n = f64::from(steps);
    let i = (u / d * n).floor();
    if i <= 0.0 {
        0
    } else if i >= n {
        steps - 1
    } else {
        // i lies in 1..steps, so the cast keeps its value.
        i as u32
    }
}

/// The integral of the fraction of phase `p` from 0 to local time `u`.
fn local_mass(p: &Phase, u: f64) -> f64 {
    let (s, e) = (p.start_fraction, p.end_fraction);
    let d = p.duration.as_secs_f64();
    let u = u.clamp(0.0, d);
    match p.shape {
        Shape::Hold => s * u,
        Shape::Linear => s * u + (e - s) * u * u / (2.0 * d),
        Shape::Step { steps } => {
            let w = d / f64::from(steps);
            let k = step_index(u, d, steps);
            let level = s + (e - s) * f64::from(k) / f64::from(steps);
            step_mass_before(s, e, steps, w, k) + level * (u - f64::from(k) * w)
        }
    }
}

/// The integral of a step phase over its first `k` complete steps.
fn step_mass_before(s: f64, e: f64, steps: u32, w: f64, k: u32) -> f64 {
    let k = f64::from(k);
    w * (k * s + (e - s) / f64::from(steps) * k * (k - 1.0) / 2.0)
}

/// The local time at which the integral of phase `p` reaches `m`, for a
/// value of `m` above 0 and at most the phase mass.
fn local_time_at_mass(p: &Phase, d: f64, m: f64) -> f64 {
    let (s, e) = (p.start_fraction, p.end_fraction);
    match p.shape {
        Shape::Hold => m / s,
        Shape::Linear => {
            // Solve a u^2 + s u = m with a = (e - s) / 2d in the form that stays
            // stable when a is close to 0.
            let a = (e - s) / (2.0 * d);
            let disc = (s * s + 4.0 * a * m).max(0.0);
            2.0 * m / (s + disc.sqrt())
        }
        Shape::Step { steps } => {
            let w = d / f64::from(steps);
            // Find the first step whose end mass reaches m. Every earlier step
            // ends below m, so the found step has a positive level.
            let (mut lo, mut hi) = (0u32, steps - 1);
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                if step_mass_before(s, e, steps, w, mid + 1) >= m {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            let level = s + (e - s) * f64::from(lo) / f64::from(steps);
            let before = step_mass_before(s, e, steps, w, lo);
            f64::from(lo) * w + (m - before) / level
        }
    }
}

/// The earliest local time at or after `u0` and before `d` at which the
/// fraction of phase `p` is at least `level`.
fn local_first_at_or_above(p: &Phase, d: f64, level: f64, u0: f64) -> Option<f64> {
    if u0 >= d {
        return None;
    }
    if local_fraction(p, u0) >= level {
        return Some(u0);
    }
    let (s, e) = (p.start_fraction, p.end_fraction);
    match p.shape {
        Shape::Hold => None,
        Shape::Linear => {
            if e <= s {
                return None;
            }
            let u = (level - s) * d / (e - s);
            (u < d).then_some(u.max(u0))
        }
        Shape::Step { steps } => {
            let w = d / f64::from(steps);
            let first = step_index(u0, d, steps) + 1;
            (first..steps)
                .find(|&i| s + (e - s) * f64::from(i) / f64::from(steps) >= level)
                .map(|i| f64::from(i) * w)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    fn ramp_steady_down() -> LoadProfile {
        LoadProfile::new(vec![
            Phase::new("ramp_up", secs(10), Shape::Linear, 0.0, 1.0),
            Phase::new("steady", secs(10), Shape::Hold, 1.0, 1.0),
            Phase::new("ramp_down", secs(10), Shape::Linear, 1.0, 0.0),
        ])
        .unwrap()
    }

    #[test]
    fn fractions_follow_the_shapes() {
        let p = ramp_steady_down();
        assert_eq!(p.fraction_at(0.0), 0.0);
        assert!((p.fraction_at(5.0) - 0.5).abs() < 1e-12);
        assert_eq!(p.fraction_at(10.0), 1.0);
        assert_eq!(p.fraction_at(15.0), 1.0);
        assert!((p.fraction_at(25.0) - 0.5).abs() < 1e-12);
        assert_eq!(p.fraction_at(30.0), 0.0);
        assert_eq!(p.phase_at(9.999).unwrap().name, "ramp_up");
        assert_eq!(p.phase_at(10.0).unwrap().name, "steady");
        assert_eq!(p.phase_at(20.0).unwrap().name, "ramp_down");
        assert!(p.phase_at(30.0).is_none());
    }

    #[test]
    fn mass_and_inverse_agree() {
        let p = ramp_steady_down();
        assert!((p.total_mass() - 20.0).abs() < 1e-12);
        assert!((p.mass_at(5.0) - 1.25).abs() < 1e-12);
        for m in [0.1, 1.25, 5.0, 7.5, 12.0, 15.0, 19.999] {
            let t = p.time_at_mass(m).unwrap();
            assert!((p.mass_at(t) - m).abs() < 1e-9, "m {m} t {t}");
        }
        assert!(p.time_at_mass(20.001).is_none());
    }

    #[test]
    fn step_levels_start_at_the_start_fraction() {
        let p = LoadProfile::new(vec![Phase::new(
            "down",
            secs(5),
            Shape::Step { steps: 5 },
            1.0,
            0.0,
        )])
        .unwrap();
        let levels: Vec<f64> = (0..5).map(|i| p.fraction_at(f64::from(i) + 0.5)).collect();
        let want = [1.0, 0.8, 0.6, 0.4, 0.2];
        for (got, want) in levels.iter().zip(want) {
            assert!((got - want).abs() < 1e-12, "{levels:?}");
        }
        assert!((p.total_mass() - 3.0).abs() < 1e-12);
        let t = p.time_at_mass(2.7).unwrap();
        assert!((t - 3.75).abs() < 1e-9, "{t}");
    }

    #[test]
    fn first_time_at_or_above_skips_flat_phases() {
        let p = ramp_steady_down();
        assert!((p.first_time_at_or_above(0.5, 0.0).unwrap() - 5.0).abs() < 1e-12);
        assert_eq!(p.first_time_at_or_above(0.5, 12.0), Some(12.0));
        assert!(p.first_time_at_or_above(0.9, 22.0).is_none());
        assert!(p.first_time_at_or_above(1.5, 0.0).is_none());
    }

    #[test]
    fn invalid_phases_are_rejected() {
        assert_eq!(LoadProfile::new(vec![]).unwrap_err(), SchedError::NoPhases);
        let zero = Phase::new("z", Duration::ZERO, Shape::Hold, 1.0, 1.0);
        assert!(matches!(
            LoadProfile::new(vec![zero]),
            Err(SchedError::ZeroDuration { index: 0, .. })
        ));
        let neg = Phase::new("n", secs(1), Shape::Linear, -0.1, 1.0);
        assert!(matches!(
            LoadProfile::new(vec![neg]),
            Err(SchedError::InvalidFraction {
                field: "start_fraction",
                ..
            })
        ));
        let steps = Phase::new("s", secs(1), Shape::Step { steps: 0 }, 0.0, 1.0);
        assert!(matches!(
            LoadProfile::new(vec![steps]),
            Err(SchedError::ZeroSteps { .. })
        ));
    }
}
