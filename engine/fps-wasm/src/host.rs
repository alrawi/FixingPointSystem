// SPDX-License-Identifier: GPL-3.0-only
//! The engine calls of the extension world, as a Rust trait.
//!
//! [`Host`] is the `engine` interface of `contracts/extension/fps-extension.wit`
//! in the form that `contracts/extension/README.md` gives it. The runtime
//! implements it once per session, and the session owns the value for its
//! whole life, so the guest never reaches the network, time, or randomness
//! except through it.

use std::fmt;

/// A timer handle that [`Host::set_timer`] returns and the `on_timer` hook receives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TimerId(pub u32);

/// Why the engine refused an engine call. The cases match `engine-error`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EngineError {
    /// The session no longer accepts writes.
    Closed,
    /// The call would exceed a limit, such as the timer count.
    Limit,
    /// The call is not valid in the current hook, such as send during render.
    NotAllowed,
}

/// Log levels of [`Host::log`]. The cases match `log-level`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum LogLevel {
    /// Finest detail.
    Trace,
    /// Debugging detail.
    Debug,
    /// Normal events.
    Info,
    /// Recoverable problems.
    Warn,
    /// Failures.
    Error,
}

/// Engine calls, bound to the session whose hook is running.
pub trait Host: Send {
    /// Writes bytes to the session transport after any bytes already queued.
    fn send(&mut self, data: &[u8]) -> Result<(), EngineError>;
    /// Starts a one shot timer that fires `on_timer` after `delay_ms` of session time.
    fn set_timer(&mut self, delay_ms: u32) -> Result<TimerId, EngineError>;
    /// Cancels a timer. Cancelling a fired or unknown timer does nothing.
    fn cancel_timer(&mut self, timer: TimerId);
    /// Emits a structured log record.
    fn log(&mut self, level: LogLevel, message: &str, fields: &[(&str, &str)]);
    /// Returns the next 64 bit value of the session generator.
    fn random_u64(&mut self) -> u64;
    /// Fills `out` from the session generator.
    fn random_bytes(&mut self, out: &mut [u8]);
}

/// The hook that is running, which decides which engine calls are valid.
///
/// The table is the one in `contracts/extension/README.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// `functions`, which the host calls once per load. No engine call is valid.
    Catalog,
    /// `on-open`.
    Open,
    /// `on-data`.
    Data,
    /// `on-timer`.
    Timer,
    /// `render`.
    Render,
    /// `on-close`.
    Close,
}

impl Phase {
    /// Returns true when `send` and `set-timer` are valid.
    pub(crate) fn may_write(self) -> bool {
        matches!(self, Phase::Open | Phase::Data | Phase::Timer)
    }

    /// Returns true when `cancel-timer` acts. It does nothing elsewhere.
    pub(crate) fn may_cancel(self) -> bool {
        matches!(
            self,
            Phase::Open | Phase::Data | Phase::Timer | Phase::Render
        )
    }

    /// Returns true when `random-u64` and `random-bytes` are valid. They have
    /// no error case, so a call elsewhere traps the guest.
    pub(crate) fn may_draw(self) -> bool {
        !matches!(self, Phase::Close | Phase::Catalog)
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Phase::Catalog => "functions",
            Phase::Open => "on-open",
            Phase::Data => "on-data",
            Phase::Timer => "on-timer",
            Phase::Render => "render",
            Phase::Close => "on-close",
        })
    }
}

/// The host of the catalog call, which has no session.
pub(crate) struct NoHost;

impl Host for NoHost {
    fn send(&mut self, _data: &[u8]) -> Result<(), EngineError> {
        Err(EngineError::NotAllowed)
    }
    fn set_timer(&mut self, _delay_ms: u32) -> Result<TimerId, EngineError> {
        Err(EngineError::NotAllowed)
    }
    fn cancel_timer(&mut self, _timer: TimerId) {}
    fn log(&mut self, _level: LogLevel, _message: &str, _fields: &[(&str, &str)]) {}
    fn random_u64(&mut self) -> u64 {
        0
    }
    fn random_bytes(&mut self, out: &mut [u8]) {
        out.fill(0);
    }
}
