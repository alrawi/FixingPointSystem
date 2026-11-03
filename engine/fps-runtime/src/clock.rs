// SPDX-License-Identifier: GPL-3.0-only
//! The Tokio backed clock of the scheduler.

use std::time::Duration;

use fps_sched::Clock;
use tokio::time::Instant;

/// A [`Clock`] that reads the Tokio timer of the worker.
///
/// The clock reports the time since its start instant. It follows a paused
/// Tokio clock in tests, so a test advances the scheduler with
/// `tokio::time::advance`.
#[derive(Debug, Clone, Copy)]
pub struct TokioClock {
    start: Instant,
}

impl TokioClock {
    /// Returns a clock whose run starts now.
    pub fn start_now() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    /// Returns a clock whose run starts at `start`.
    pub fn starting_at(start: Instant) -> Self {
        Self { start }
    }

    /// Returns the start instant.
    pub fn start(&self) -> Instant {
        self.start
    }

    /// Returns the Tokio instant that lies `offset` after the start.
    pub fn instant_at(&self, offset: Duration) -> Instant {
        self.start.checked_add(offset).unwrap_or(self.start)
    }
}

impl Clock for TokioClock {
    fn elapsed(&self) -> Duration {
        Instant::now().saturating_duration_since(self.start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn follows_the_paused_tokio_clock() {
        let clock = TokioClock::start_now();
        assert_eq!(clock.elapsed(), Duration::ZERO);
        tokio::time::advance(Duration::from_millis(1500)).await;
        assert_eq!(clock.elapsed(), Duration::from_millis(1500));
        assert_eq!(
            clock.instant_at(Duration::from_secs(2)),
            clock.start() + Duration::from_secs(2)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn drives_a_constant_rate_schedule() {
        use fps_sched::{Arrival, LoadProfile, Objective, Phase, Schedule, ScheduleConfig, Shape};
        let profile = LoadProfile::new(vec![Phase::new(
            "steady",
            Duration::from_secs(2),
            Shape::Hold,
            1.0,
            1.0,
        )])
        .unwrap();
        let mut cfg = ScheduleConfig::new(Objective::SessionsPerSecond(10.0), profile, 1);
        cfg.arrival = Arrival::Constant;
        let mut sched = Schedule::new(cfg).unwrap();
        let clock = TokioClock::start_now();
        let mut started = 0;
        while !sched.is_finished(&clock) {
            started += sched.drain(&clock, 0).count();
            match sched.next_due(&clock, 0) {
                Some(due) => tokio::time::sleep_until(clock.instant_at(due)).await,
                None => break,
            }
        }
        started += sched.drain(&clock, 0).count();
        assert_eq!(started, 20);
    }
}
