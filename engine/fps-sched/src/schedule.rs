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
use crate::rate::RateHandle;
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
    /// Scheduled time in seconds of the next rate arrival, if one remains. It
    /// is `None` when the arrival lies beyond the profile and while the rate is 0.
    pending: Option<f64>,
    /// The operational time of the next rate arrival.
    pending_op: f64,
    /// The operational time and the profile mass at the last rate change. An
    /// arrival at operational time `op` lies where the mass reaches
    /// `mass_base + (op - op_base) / rate`, so the expected count stays
    /// continuous across a change.
    op_base: f64,
    mass_base: f64,
    handle: RateHandle,
    /// The bits of the target that the schedule applied last.
    applied: u64,
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
        let handle = RateHandle::new(cfg.objective);
        let applied = handle.bits();
        let mut sched = Self {
            cfg,
            rng,
            next_session_id: 0,
            drawn: 0,
            op_time: 0.0,
            pending: None,
            pending_op: 0.0,
            op_base: 0.0,
            mass_base: 0.0,
            handle,
            applied,
            skipped: 0,
        };
        sched.advance();
        Ok(sched)
    }

    /// Returns a handle that changes the target of this schedule while it runs.
    ///
    /// The handle is `Send + Sync` and cheap to clone, so a control thread can
    /// keep a clone while the worker owns the schedule. The schedule loads the
    /// handle at every [`Schedule::drain`] and applies a new target at that
    /// tick, with these rules.
    ///
    /// - The arrival process stays continuous. The expected count of arrivals
    ///   at the tick is the same before and after the change, and every later
    ///   arrival follows the new rate. The change causes no burst that catches
    ///   up and no gap. A Poisson process stays Poisson, because only the map
    ///   from operational time to time changes.
    /// - Arrivals that were already due at the tick under the old rate are
    ///   released first, and the new rate applies from the first tick at which
    ///   none is due. Under [`CapPolicy::Defer`] at the cap this means that a
    ///   change waits until the deferred arrivals have started.
    /// - A rate of 0 pauses the schedule. It yields no start and does not
    ///   finish. The expected count stands still, so the resume with a positive
    ///   rate does not owe the paused time. A concurrency target of 0 yields no
    ///   start and leaves open sessions to the worker.
    /// - The profile ends at a fixed time whatever the rate, and the schedule
    ///   finishes when no arrival remains before that time.
    /// - [`Schedule::next_due`] returns a time that is not after the clock while
    ///   a change waits, so a worker that re-reads it ticks at once. A worker
    ///   that sleeps with no due time, which is the case while the schedule is
    ///   paused or at the cap, must wake on its own to see a change, for
    ///   example every 100 ms.
    pub fn rate_handle(&self) -> RateHandle {
        self.handle.clone()
    }

    /// Returns the configuration. Its objective holds the target that the
    /// schedule applied last, which may be a rate of 0 after a pause.
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
            Objective::SessionsPerSecond(rate) => {
                self.pending.is_none() && rate > 0.0 && !self.change_waits()
            }
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
        self.apply_change(now);
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
                let due = self.pending.map(|t| self.to_duration(t));
                if self.change_waits() {
                    let now = clock.elapsed();
                    return Some(due.map_or(now, |d| d.min(now)));
                }
                due
            }
            Objective::ConcurrentSessions(n) => {
                if self.change_waits() {
                    return Some(clock.elapsed());
                }
                if at_cap || n == 0 {
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
        if !matches!(self.cfg.objective, Objective::SessionsPerSecond(_)) {
            return;
        }
        self.pending_op = match self.cfg.arrival {
            Arrival::Constant => self.drawn as f64 + 0.5,
            Arrival::Poisson => {
                let u: f64 = self.rng.gen();
                // u lies in [0, 1), so 1 - u lies in (0, 1] and the log is finite.
                self.op_time += -(1.0 - u).ln();
                self.op_time
            }
        };
        self.drawn += 1;
        self.pending = self.pending_time();
    }

    /// Maps the operational time of the pending arrival to a time in seconds
    /// under the current rate, or `None` beyond the profile and at rate 0.
    fn pending_time(&self) -> Option<f64> {
        let Objective::SessionsPerSecond(rate) = self.cfg.objective else {
            return None;
        };
        if rate <= 0.0 {
            return None;
        }
        let mass = self.mass_base + (self.pending_op - self.op_base) / rate;
        self.cfg.profile.time_at_mass(mass)
    }

    /// Returns true when the handle holds a target that the schedule has not
    /// applied yet.
    fn change_waits(&self) -> bool {
        self.handle.bits() != self.applied
    }

    /// Applies a new target from the handle at the clock reading `now`.
    ///
    /// A rate schedule rebases its operational time at `now`. The expected
    /// count there is `op_base + (mass_at(now) - mass_base) * rate`, and the
    /// pending arrival keeps its operational time, so it moves to where the new
    /// rate reaches it. An arrival that is already due keeps its time, and the
    /// change waits for the tick at which none is due.
    fn apply_change(&mut self, now: Duration) {
        let bits = self.handle.bits();
        if bits == self.applied {
            return;
        }
        let new = f64::from_bits(bits);
        match self.cfg.objective {
            Objective::SessionsPerSecond(old) => {
                if self.pending.is_some_and(|t| self.to_duration(t) <= now) {
                    return;
                }
                let mass_now = self
                    .cfg
                    .profile
                    .mass_at(now.as_secs_f64())
                    .max(self.mass_base);
                self.op_base += (mass_now - self.mass_base) * old;
                self.mass_base = mass_now;
                self.cfg.objective = Objective::SessionsPerSecond(new);
                self.pending = self.pending_time();
            }
            Objective::ConcurrentSessions(_) => {
                self.cfg.objective = Objective::ConcurrentSessions(new as u32);
            }
        }
        self.applied = bits;
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
        s.apply_change(self.now);
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
