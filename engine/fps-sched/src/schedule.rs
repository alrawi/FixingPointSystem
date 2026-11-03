// SPDX-License-Identifier: GPL-3.0-only
//! Session starts from the objective, the phases, and the arrival process.
//!
//! A [`Schedule`] serves one worker rate, which is one mix entry on one
//! worker. A rate objective places session starts on the time axis with the
//! time change method. The schedule draws arrivals of a unit rate process in
//! operational time and maps each one back through the integral of the target
//! rate, so a linear ramp, a step, and a hold all keep the exact expected
//! count. A concurrency objective starts a session whenever the open sessions
//! fall below the target of the current phase. A worker calls
//! [`Schedule::drain`] with its clock and its open session count, starts every
//! session that the drain yields, and sleeps until [`Schedule::next_due`].

use std::time::Duration;

use rand::Rng;
use rand_pcg::Pcg64Mcg;

use crate::clock::Clock;
use crate::profile::LoadProfile;
use crate::SchedError;

/// Slack for the floor of the concurrency target, so that a fraction that
/// rounding puts just below an integer still reaches it.
const TARGET_EPS: f64 = 1e-6;

/// The steady state target of one worker rate, as `WorkerRate` defines it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Objective {
    /// Open sessions at this rate per second at fraction 1.
    SessionsPerSecond(f64),
    /// Keep this many sessions open at fraction 1.
    ConcurrentSessions(u32),
}

/// The arrival process of a rate objective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Arrival {
    /// Evenly spaced starts. Start `k` lies where the expected count reaches
    /// `k + 0.5`, so each phase holds its expected count rounded.
    Constant,
    /// A nonhomogeneous Poisson process whose intensity is the target rate.
    /// Inter arrival times at a constant rate are exponential.
    #[default]
    Poisson,
}

/// What a rate objective does with an arrival that finds the concurrency cap
/// reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CapPolicy {
    /// Hold the arrival until a session closes. The start keeps its scheduled
    /// time, so the caller can measure the delay.
    #[default]
    Defer,
    /// Drop the arrival and count it in [`Schedule::skipped`].
    Skip,
}

/// The configuration of one schedule.
#[derive(Debug, Clone)]
pub struct ScheduleConfig {
    /// The steady state target.
    pub objective: Objective,
    /// The arrival process, which only a rate objective uses.
    pub arrival: Arrival,
    /// The load phases.
    pub profile: LoadProfile,
    /// The run seed of the plan.
    pub seed: u64,
    /// The generator stream, unique per worker rate, see [`stream_id`].
    pub stream: u64,
    /// The largest number of open sessions, or `None` for no cap.
    pub max_concurrent: Option<u32>,
    /// The handling of arrivals at the cap.
    pub cap_policy: CapPolicy,
}

impl ScheduleConfig {
    /// Returns a configuration with Poisson arrivals, no cap, and stream 0.
    pub fn new(objective: Objective, profile: LoadProfile, seed: u64) -> Self {
        Self {
            objective,
            arrival: Arrival::default(),
            profile,
            seed,
            stream: 0,
            max_concurrent: None,
            cap_policy: CapPolicy::default(),
        }
    }
}

/// Returns the generator stream of mix entry `mix_index` on worker
/// `worker_id`, so every worker rate of a plan draws from its own stream.
pub fn stream_id(worker_id: u32, mix_index: u32) -> u64 {
    u64::from(worker_id) << 32 | u64::from(mix_index)
}

/// Returns the 128 bit generator state of one stream of one seed.
///
/// `Pcg64Mcg` forces the two lowest state bits to 1, so a state built as
/// `seed << 64 | stream` would give streams 0 to 3 the same sequence. The
/// stream therefore sits above those bits, which keeps streams below 2^62
/// distinct, and the high half mixes seed and stream with SplitMix64.
fn generator_state(seed: u64, stream: u64) -> u128 {
    let high = splitmix64(seed ^ splitmix64(stream));
    u128::from(high) << 64 | u128::from(stream) << 2
}

/// The SplitMix64 finalizer, a bijection on 64 bit values.
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// One session start that the schedule releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionStart {
    /// The session id, counting from 0 within the schedule.
    pub session_id: u64,
    /// The time since the run start at which the session was due. A deferred
    /// start keeps this time, and a concurrency start uses the drain time.
    pub scheduled_at: Duration,
    /// The index of the phase that holds `scheduled_at`.
    pub phase: usize,
}

/// The session scheduler of one worker rate.
#[derive(Debug)]
pub struct Schedule {
    cfg: ScheduleConfig,
    rng: Pcg64Mcg,
    next_session_id: u64,
    /// Arrivals drawn so far by a rate objective.
    drawn: u64,
    /// Operational time of the last Poisson arrival.
    op_time: f64,
    /// Scheduled time in seconds of the next rate arrival, if one remains.
    pending: Option<f64>,
    skipped: u64,
}

impl Schedule {
    /// Validates the configuration and returns the schedule at the run start.
    pub fn new(cfg: ScheduleConfig) -> Result<Self, SchedError> {
        match cfg.objective {
            Objective::SessionsPerSecond(r) if !(r.is_finite() && r > 0.0) => {
                return Err(SchedError::InvalidTarget(r));
            }
            Objective::ConcurrentSessions(0) => return Err(SchedError::InvalidTarget(0.0)),
            _ => {}
        }
        if cfg.max_concurrent == Some(0) {
            return Err(SchedError::ZeroCap);
        }
        let rng = Pcg64Mcg::new(generator_state(cfg.seed, cfg.stream));
        let mut sched = Self {
            cfg,
            rng,
            next_session_id: 0,
            drawn: 0,
            op_time: 0.0,
            pending: None,
            skipped: 0,
        };
        sched.advance();
        Ok(sched)
    }

    /// Returns the configuration.
    pub fn config(&self) -> &ScheduleConfig {
        &self.cfg
    }

    /// Returns the number of arrivals that [`CapPolicy::Skip`] dropped.
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// Returns the name of the phase at the clock time, or `None` after the end.
    pub fn phase_name(&self, clock: &impl Clock) -> Option<&str> {
        let t = clock.elapsed().as_secs_f64();
        self.cfg.profile.phase_at(t).map(|p| p.name.as_str())
    }

    /// Returns true when the schedule will release no further session.
    pub fn is_finished(&self, clock: &impl Clock) -> bool {
        match self.cfg.objective {
            Objective::SessionsPerSecond(_) => self.pending.is_none(),
            Objective::ConcurrentSessions(_) => {
                clock.elapsed().as_secs_f64() >= self.cfg.profile.total_secs()
            }
        }
    }

    /// Returns an iterator over the sessions to start now, given `open`
    /// sessions that the worker holds open. The iterator reads the clock once
    /// and counts every start it yields as open.
    pub fn drain<'a>(&'a mut self, clock: &impl Clock, open: u32) -> Drain<'a> {
        let now = clock.elapsed();
        Drain {
            sched: self,
            now,
            now_secs: now.as_secs_f64(),
            open,
        }
    }

    /// Returns the earliest time at which a drain with `open` open sessions
    /// may yield a start. `None` means that only a closing session or nothing
    /// at all can release another start. A time before the clock means that a
    /// start is overdue.
    pub fn next_due(&self, clock: &impl Clock, open: u32) -> Option<Duration> {
        let at_cap = self.cfg.max_concurrent.is_some_and(|cap| open >= cap);
        match self.cfg.objective {
            Objective::SessionsPerSecond(_) => {
                if at_cap && self.cfg.cap_policy == CapPolicy::Defer {
                    return None;
                }
                self.pending.map(|t| self.to_duration(t))
            }
            Objective::ConcurrentSessions(n) => {
                if at_cap {
                    return None;
                }
                let level = (f64::from(open) + 1.0 - TARGET_EPS / 2.0) / f64::from(n);
                let now = clock.elapsed().as_secs_f64();
                self.cfg
                    .profile
                    .first_time_at_or_above(level, now)
                    .map(|t| self.to_duration(t))
            }
        }
    }

    /// Draws the next rate arrival into `pending`.
    fn advance(&mut self) {
        let Objective::SessionsPerSecond(rate) = self.cfg.objective else {
            return;
        };
        let op = match self.cfg.arrival {
            Arrival::Constant => self.drawn as f64 + 0.5,
            Arrival::Poisson => {
                let u: f64 = self.rng.gen();
                // u lies in [0, 1), so 1 - u lies in (0, 1] and the log is finite.
                self.op_time += -(1.0 - u).ln();
                self.op_time
            }
        };
        self.drawn += 1;
        self.pending = self.cfg.profile.time_at_mass(op / rate);
    }

    /// Converts seconds from the run start into a duration within the profile.
    fn to_duration(&self, t: f64) -> Duration {
        let total = self.cfg.profile.total_secs();
        Duration::try_from_secs_f64(t.clamp(0.0, total)).unwrap_or(Duration::MAX)
    }

    fn release(&mut self, at: Duration, t: f64) -> SessionStart {
        let id = self.next_session_id;
        self.next_session_id += 1;
        let last = self.cfg.profile.phases().len() - 1;
        SessionStart {
            session_id: id,
            scheduled_at: at,
            phase: self.cfg.profile.phase_index_at(t).unwrap_or(last),
        }
    }
}

/// The sessions to start at one clock reading, from [`Schedule::drain`].
#[derive(Debug)]
pub struct Drain<'a> {
    sched: &'a mut Schedule,
    now: Duration,
    now_secs: f64,
    open: u32,
}

impl Iterator for Drain<'_> {
    type Item = SessionStart;

    fn next(&mut self) -> Option<SessionStart> {
        let s = &mut *self.sched;
        let max = s.cfg.max_concurrent;
        let at_cap = |open: u32| max.is_some_and(|cap| open >= cap);
        match s.cfg.objective {
            Objective::SessionsPerSecond(_) => loop {
                let t = s.pending?;
                let at = s.to_duration(t);
                if at > self.now {
                    return None;
                }
                if at_cap(self.open) {
                    match s.cfg.cap_policy {
                        CapPolicy::Defer => return None,
                        CapPolicy::Skip => {
                            s.skipped += 1;
                            s.advance();
                            continue;
                        }
                    }
                }
                s.advance();
                self.open = self.open.saturating_add(1);
                return Some(s.release(at, t));
            },
            Objective::ConcurrentSessions(n) => {
                let frac = s.cfg.profile.fraction_at(self.now_secs);
                let mut target = (f64::from(n) * frac + TARGET_EPS).floor();
                if let Some(cap) = s.cfg.max_concurrent {
                    target = target.min(f64::from(cap));
                }
                if self.now_secs >= s.cfg.profile.total_secs() || f64::from(self.open) >= target {
                    return None;
                }
                self.open = self.open.saturating_add(1);
                Some(s.release(self.now, self.now_secs))
            }
        }
    }
}
