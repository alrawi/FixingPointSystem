// SPDX-License-Identifier: GPL-3.0-only
//! The clock seam of the template function `now()`.
//!
//! Scenario contract v0.6 says that the engine reads the clock through the
//! same seam as its random source. [`RenderCtx`](crate::template::RenderCtx)
//! carries a [`Clock`] next to the generator. A production run uses
//! [`SystemClock`], which reads the wall clock of the host in UTC. A test, a
//! golden, or a replay that needs equal bytes uses [`PinnedClock`], which
//! returns one fixed instant and may advance by a fixed step per call.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A source of the current instant.
pub trait Clock: Send + Sync {
    /// Returns the current instant as milliseconds since
    /// 1970-01-01T00:00:00Z. Each call of `now()` in a template reads the
    /// clock once.
    fn now_ms(&self) -> i64;
}

/// The wall clock of the host.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
            Err(e) => i64::try_from(e.duration().as_millis()).map_or(i64::MIN, |ms| -ms),
        }
    }
}

/// A clock that tests pin to a fixed instant.
///
/// The first call returns `start_ms`, and every later call returns the
/// previous value plus `step_ms`, so a step of 0 gives one fixed instant.
#[derive(Debug)]
pub struct PinnedClock {
    next: AtomicI64,
    step_ms: i64,
}

impl PinnedClock {
    /// Pins the clock to `start_ms` milliseconds since the epoch.
    pub fn new(start_ms: i64) -> Self {
        PinnedClock {
            next: AtomicI64::new(start_ms),
            step_ms: 0,
        }
    }

    /// Advances the clock by `step_ms` milliseconds after every call.
    #[must_use]
    pub fn with_step_ms(mut self, step_ms: i64) -> Self {
        self.step_ms = step_ms;
        self
    }
}

impl Clock for PinnedClock {
    fn now_ms(&self) -> i64 {
        self.next
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |t| {
                Some(t.saturating_add(self.step_ms))
            })
            .unwrap_or_default()
    }
}

const DAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Formats `ms` milliseconds since the epoch as the IMF fixdate of RFC 9110
/// section 5.6.7, for example `Tue, 10 Nov 2026 06:15:04 GMT`. The result has
/// 29 bytes. It returns `None` when the year does not fit four digits.
pub fn http_date(ms: i64) -> Option<String> {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil date from a day count, after Howard Hinnant's algorithm.
    let z = days.checked_add(719_468)?;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = era.checked_mul(400)?.checked_add(yoe)? + i64::from(month <= 2);
    if !(1..=9999).contains(&year) {
        return None;
    }
    // 1970-01-01 was a Thursday, which is index 3 of a week that starts on Monday.
    let weekday = usize::try_from((days + 3).rem_euclid(7)).ok()?;
    let month_name = MONTH_NAMES.get(usize::try_from(month - 1).ok()?)?;
    Some(format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        DAY_NAMES.get(weekday)?,
        day,
        month_name,
        year,
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_date_matches_the_rfc_and_contract_examples() {
        assert_eq!(
            http_date(784_111_777_000).as_deref(),
            Some("Sun, 06 Nov 1994 08:49:37 GMT")
        );
        assert_eq!(
            http_date(1_794_291_304_000).as_deref(),
            Some("Tue, 10 Nov 2026 06:15:04 GMT")
        );
    }

    #[test]
    fn http_date_handles_the_epoch_leap_days_and_the_year_range() {
        assert_eq!(
            http_date(0).as_deref(),
            Some("Thu, 01 Jan 1970 00:00:00 GMT")
        );
        // One millisecond before the epoch is still 1969.
        assert_eq!(
            http_date(-1).as_deref(),
            Some("Wed, 31 Dec 1969 23:59:59 GMT")
        );
        assert_eq!(
            http_date(951_782_400_000).as_deref(),
            Some("Tue, 29 Feb 2000 00:00:00 GMT")
        );
        // 1900 is not a leap year, so day 60 of the year is 1 March.
        assert_eq!(
            http_date(-2_203_891_200_000).as_deref(),
            Some("Thu, 01 Mar 1900 00:00:00 GMT")
        );
        // The last second of 9999, and the first second after it.
        assert_eq!(
            http_date(253_402_300_799_000).as_deref(),
            Some("Fri, 31 Dec 9999 23:59:59 GMT")
        );
        assert_eq!(http_date(253_402_300_800_000), None);
        assert_eq!(http_date(-62_135_596_800_001), None);
        assert_eq!(http_date(i64::MAX), None);
        assert_eq!(http_date(i64::MIN), None);
    }

    #[test]
    fn http_date_is_29_bytes_from_2001_to_2286() {
        // 2001-09-09T01:46:40Z is 10^9 s, and 2286-11-20T17:46:39Z is 10^10 - 1 s.
        for ms in [1_000_000_000_000_i64, 9_999_999_999_000] {
            assert_eq!(http_date(ms).map(|s| s.len()), Some(29));
        }
    }

    #[test]
    fn pinned_clock_returns_one_instant_or_steps() {
        let fixed = PinnedClock::new(5);
        assert_eq!((fixed.now_ms(), fixed.now_ms(), fixed.now_ms()), (5, 5, 5));
        let stepped = PinnedClock::new(1000).with_step_ms(250);
        assert_eq!(
            (stepped.now_ms(), stepped.now_ms(), stepped.now_ms()),
            (1000, 1250, 1500)
        );
    }

    #[test]
    fn system_clock_is_after_the_year_2025() {
        assert!(SystemClock.now_ms() > 1_735_689_600_000);
    }
}
