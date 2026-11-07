// SPDX-License-Identifier: GPL-3.0-only
//! Session scheduler of the engine.
//!
//! The scheduler turns the load objective, the load phases, and the arrival
//! process of one worker rate into session starts. [`LoadProfile`] holds the
//! phases and gives the target fraction at every time, [`Schedule`] produces
//! the session starts, and a worker drains them with [`Schedule::drain`] and an
//! injected [`Clock`]. The same seed, stream, and configuration always give the
//! same scheduled times, so a replay reproduces every session start. A
//! [`RateHandle`] changes the target of a running schedule, see [`rate`].
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod clock;
pub mod profile;
pub mod rate;
pub mod schedule;

pub use clock::{Clock, ManualClock};
pub use profile::{LoadProfile, Phase, Shape};
pub use rate::{RateControl, RateHandle};
pub use schedule::{
    stream_id, Arrival, CapPolicy, Drain, Objective, Schedule, ScheduleConfig, SessionStart,
};

/// Errors that the scheduler reports for an invalid configuration.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SchedError {
    /// The load profile has no phase.
    #[error("load profile: no phases")]
    NoPhases,
    /// A phase has a zero duration.
    #[error("phase {index} '{name}': duration must be positive")]
    ZeroDuration {
        /// The position of the phase in the profile.
        index: usize,
        /// The phase name.
        name: String,
    },
    /// A phase fraction is negative or not finite.
    #[error("phase {index} '{name}': {field} {value} must be finite and not negative")]
    InvalidFraction {
        /// The position of the phase in the profile.
        index: usize,
        /// The phase name.
        name: String,
        /// The field, `start_fraction` or `end_fraction`.
        field: &'static str,
        /// The rejected value.
        value: f64,
    },
    /// A step phase has zero steps.
    #[error("phase {index} '{name}': a step phase needs at least one step")]
    ZeroSteps {
        /// The position of the phase in the profile.
        index: usize,
        /// The phase name.
        name: String,
    },
    /// A shape name is not one of the contract names.
    #[error("unknown phase shape '{0}'")]
    UnknownShape(String),
    /// The objective target is zero, negative, or not finite.
    #[error("objective: target {0} must be finite and positive")]
    InvalidTarget(f64),
    /// The concurrency cap is zero.
    #[error("concurrency cap must be positive")]
    ZeroCap,
    /// A rate update is negative, not finite, or too large for its kind.
    #[error("rate update: {0} must be finite and not negative")]
    InvalidRate(f64),
    /// A rate update names a scenario that no worker rate serves.
    #[error("rate update: unknown scenario '{0}'")]
    UnknownScenario(String),
    /// A rate update names one scenario twice.
    #[error("rate update: scenario '{0}' appears twice")]
    DuplicateScenario(String),
    /// A rate update gives sessions per second to a concurrency objective, or
    /// the reverse.
    #[error("rate update: the kind of the target does not match the objective")]
    KindMismatch,
}
