// SPDX-License-Identifier: GPL-3.0-only
//! The seeded session sampler.
//!
//! The sampler decides from the run seed and the session id alone, so a replay
//! of a run samples the same sessions, and no state is shared between workers.

use crate::CaptureError;

/// How the sampler picks sessions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SamplePolicy {
    /// No session is captured. A capture started with this policy does no work at all.
    Off,
    /// Every session is captured with this probability, a fraction from 0 to 1.
    ///
    /// The decision is a hash of the seed and the session id, so the number of
    /// captured sessions varies around `rate * sessions` like a binomial sample.
    Rate(f64),
    /// One session in every `n` is captured, which makes the count exact.
    EveryNth(u64),
}

/// The sampling decision of one capture, see [`SamplePolicy`].
#[derive(Debug, Clone, Copy)]
pub struct Sampler {
    mode: Mode,
}

#[derive(Debug, Clone, Copy)]
enum Mode {
    Off,
    All,
    /// Captures when the hash of the seed and the id is below the threshold.
    Threshold {
        seed: u64,
        threshold: u64,
    },
    /// Captures when `(id + offset) % n == 0`.
    Nth {
        n: u64,
        offset: u64,
    },
}

/// SplitMix64 finalizer, which spreads nearby inputs over the whole range.
pub(crate) fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl Sampler {
    /// Builds the sampler for a policy and the run seed.
    ///
    /// A rate must lie in `0.0..=1.0` and `n` must be positive.
    pub fn new(policy: SamplePolicy, seed: u64) -> Result<Self, CaptureError> {
        let mode = match policy {
            SamplePolicy::Off => Mode::Off,
            SamplePolicy::Rate(rate) => {
                if !(0.0..=1.0).contains(&rate) {
                    return Err(CaptureError::InvalidConfig(format!(
                        "sample rate {rate} is outside 0 to 1"
                    )));
                }
                if rate == 0.0 {
                    Mode::Off
                } else if rate == 1.0 {
                    Mode::All
                } else {
                    // 2^64 as a float. The cast saturates, and rate < 1 keeps it below u64::MAX.
                    let threshold = (rate * 18_446_744_073_709_551_616.0) as u64;
                    Mode::Threshold { seed, threshold }
                }
            }
            SamplePolicy::EveryNth(0) => {
                return Err(CaptureError::InvalidConfig(
                    "every nth sampling needs n of at least 1".into(),
                ))
            }
            SamplePolicy::EveryNth(1) => Mode::All,
            SamplePolicy::EveryNth(n) => Mode::Nth {
                n,
                offset: splitmix64(seed) % n,
            },
        };
        Ok(Self { mode })
    }

    /// Returns true when the sampler never selects a session.
    #[inline]
    pub fn is_off(&self) -> bool {
        matches!(self.mode, Mode::Off)
    }

    /// Returns true when the session with this id is captured.
    #[inline]
    pub fn sample(&self, session_id: u64) -> bool {
        match self.mode {
            Mode::Off => false,
            Mode::All => true,
            Mode::Threshold { seed, threshold } => {
                splitmix64(seed ^ splitmix64(session_id)) < threshold
            }
            Mode::Nth { n, offset } => session_id.wrapping_add(offset) % n == 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(policy: SamplePolicy, seed: u64, sessions: u64) -> u64 {
        let s = Sampler::new(policy, seed).unwrap();
        (0..sessions).filter(|id| s.sample(*id)).count() as u64
    }

    #[test]
    fn one_percent_of_ten_thousand_lands_in_the_acceptance_band() {
        for seed in 1..=50 {
            let n = count(SamplePolicy::Rate(0.01), seed, 10_000);
            assert!((70..=130).contains(&n), "seed {seed} sampled {n}");
        }
    }

    #[test]
    fn every_nth_is_exact_for_any_seed() {
        for seed in [1, 2, 99, u64::MAX] {
            assert_eq!(count(SamplePolicy::EveryNth(100), seed, 10_000), 100);
        }
    }

    #[test]
    fn decisions_repeat_and_depend_on_the_seed() {
        let a = Sampler::new(SamplePolicy::Rate(0.1), 7).unwrap();
        let b = Sampler::new(SamplePolicy::Rate(0.1), 7).unwrap();
        let c = Sampler::new(SamplePolicy::Rate(0.1), 8).unwrap();
        let pick = |s: &Sampler| (0..1_000).filter(|i| s.sample(*i)).collect::<Vec<_>>();
        assert_eq!(pick(&a), pick(&b));
        assert_ne!(pick(&a), pick(&c));
    }

    #[test]
    fn extremes_and_invalid_values() {
        assert!(Sampler::new(SamplePolicy::Off, 1).unwrap().is_off());
        assert!(Sampler::new(SamplePolicy::Rate(0.0), 1).unwrap().is_off());
        assert_eq!(count(SamplePolicy::Rate(1.0), 1, 50), 50);
        assert_eq!(count(SamplePolicy::EveryNth(1), 1, 50), 50);
        assert!(Sampler::new(SamplePolicy::Rate(1.5), 1).is_err());
        assert!(Sampler::new(SamplePolicy::Rate(f64::NAN), 1).is_err());
        assert!(Sampler::new(SamplePolicy::EveryNth(0), 1).is_err());
    }
}
