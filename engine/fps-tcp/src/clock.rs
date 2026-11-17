// SPDX-License-Identifier: GPL-3.0-only
//! Clocks for the stacks of the harness.
//!
//! A stack reads its time from a [`Clock`] that its constructor receives, so a
//! test fixes the time with [`ManualClock`] and only the benchmark binary reads
//! the wall clock, through [`MonotonicClock`], at the edge of the process.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

/// A source of monotonic time in microseconds since an arbitrary start.
pub trait Clock: Send + Sync {
    /// Returns the current time in microseconds.
    fn now_micros(&self) -> i64;
}

/// The operating system monotonic clock, counted from its creation.
#[derive(Debug)]
pub struct MonotonicClock {
    start: Instant,
}

impl MonotonicClock {
    /// Starts a clock at zero.
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonotonicClock {
    fn now_micros(&self) -> i64 {
        i64::try_from(self.start.elapsed().as_micros()).unwrap_or(i64::MAX)
    }
}

/// A clock that moves only when a test advances it.
#[derive(Debug, Default)]
pub struct ManualClock {
    micros: AtomicI64,
}

impl ManualClock {
    /// Starts a clock at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the clock forward by `micros` microseconds.
    pub fn advance_micros(&self, micros: i64) {
        self.micros.fetch_add(micros, Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn now_micros(&self) -> i64 {
        self.micros.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_moves_only_when_advanced() {
        let clock = ManualClock::new();
        assert_eq!(clock.now_micros(), 0);
        clock.advance_micros(1500);
        assert_eq!(clock.now_micros(), 1500);
    }

    #[test]
    fn monotonic_clock_does_not_go_back() {
        let clock = MonotonicClock::new();
        let a = clock.now_micros();
        let b = clock.now_micros();
        assert!(b >= a);
    }
}
