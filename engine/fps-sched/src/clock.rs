// SPDX-License-Identifier: GPL-3.0-only
//! The injected clock that a worker drains its schedule with.
//!
//! The scheduler never reads the wall clock. A worker passes a [`Clock`] that
//! reports the time since the run start, which the runtime of ENG-10 backs with
//! its timer and which tests back with a [`ManualClock`].

use std::cell::Cell;
use std::time::Duration;

/// A source of the time elapsed since the run start.
pub trait Clock {
    /// Returns the time elapsed since the run start.
    fn elapsed(&self) -> Duration;
}

impl<C: Clock + ?Sized> Clock for &C {
    fn elapsed(&self) -> Duration {
        (**self).elapsed()
    }
}

/// A clock that moves only when the caller sets or advances it.
#[derive(Debug, Default)]
pub struct ManualClock {
    now: Cell<Duration>,
}

impl ManualClock {
    /// Returns a clock that stands at `start`.
    pub fn new(start: Duration) -> Self {
        Self {
            now: Cell::new(start),
        }
    }

    /// Moves the clock to `now`.
    pub fn set(&self, now: Duration) {
        self.now.set(now);
    }

    /// Moves the clock forward by `by`, saturating at the largest duration.
    pub fn advance(&self, by: Duration) {
        self.now.set(self.now.get().saturating_add(by));
    }
}

impl Clock for ManualClock {
    fn elapsed(&self) -> Duration {
        self.now.get()
    }
}
