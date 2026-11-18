// SPDX-License-Identifier: GPL-3.0-only
//! Time based loss detection, tail loss probes, and sender pacing (TRN-34).
//!
//! The pieces here are plain data and pure functions, like the rest of the
//! recovery code, and the state machine of [`crate::host`] calls them:
//!
//! * [`TxLog`] remembers when each flight of new data left, so that an
//!   acknowledgment gives a round trip sample with the resolution of the
//!   clock and a segment has a send time to compare with.
//! * [`Rack`] is RACK (RFC 8985 section 6): a segment is lost when a segment
//!   that was sent later has been delivered and a round trip plus a
//!   reordering window have passed since it was sent. It replaces the count
//!   of three duplicate acknowledgments, which a tail of fewer segments never
//!   reaches, and which a drop of a retransmission needs again.
//! * [`pto_us`] is the probe timeout of the tail loss probe (RFC 8985 section
//!   7): a timer shorter than the retransmission timeout that sends one probe
//!   segment, so that a lost tail is found by the acknowledgment of the probe
//!   and not by the timeout.
//! * [`pacing_gap_us`] is the spacing of sender pacing: the segments of a
//!   window leave over a round trip and not in one burst, which is what
//!   overflows a short queue.
//!
//! Everything lives in the [`crate::recovery::Recovery`] box, so an idle
//! socket pays nothing for any of it.

use std::collections::VecDeque;

use super::host::{seq_le, seq_lt};

/// The switches of the mechanisms of this module, copied from the
/// configuration into the recovery state of a connection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Features {
    /// Sender pacing.
    pub pacing: bool,
    /// HyStart for the CUBIC slow start.
    pub hystart: bool,
    /// Time based loss detection (RACK).
    pub rack: bool,
    /// Tail loss probes.
    pub tlp: bool,
}

impl Features {
    /// Whether the connection has to remember send times.
    pub fn needs_log(self) -> bool {
        self.rack || self.hystart || self.tlp
    }
}

/// The send times of the data that is in flight, in order of sequence.
///
/// One entry covers the data that left in one call of the output code, which
/// takes one reading of the clock, so a burst of segments is one entry. The
/// times are the microsecond clock truncated to 32 bits and compared as
/// differences, which is right for spans of up to 35 minutes.
#[derive(Debug, Default)]
pub struct TxLog {
    /// The first sequence number that an entry covers.
    start: u32,
    /// `(end, time)` of each flight, the end excluded.
    flights: VecDeque<(u32, u32)>,
}

/// The truncated clock.
pub fn t32(now_us: i64) -> u32 {
    now_us as u32
}

/// The microseconds from `earlier` to `later`, signed.
pub fn since(later: u32, earlier: u32) -> i32 {
    later.wrapping_sub(earlier) as i32
}

impl TxLog {
    /// True when nothing is logged.
    pub fn is_empty(&self) -> bool {
        self.flights.is_empty()
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.flights.len()
    }

    /// Forgets everything, after a timeout sent the flight again.
    pub fn clear(&mut self) {
        self.flights.clear();
    }

    /// Notes that the data `[start, end)` left at `now_us` for the first
    /// time. A flight that follows the last one at the same reading of the
    /// clock joins it.
    pub fn push(&mut self, start: u32, end: u32, now_us: i64) {
        let t = t32(now_us);
        match self.flights.back_mut() {
            Some(last) if last.0 == start && last.1 == t => {
                last.0 = end;
            }
            Some(last) if last.0 == start => self.flights.push_back((end, t)),
            Some(_) => {
                // A gap: what is logged says nothing about the new data.
                self.flights.clear();
                self.start = start;
                self.flights.push_back((end, t));
            }
            None => {
                self.start = start;
                self.flights.push_back((end, t));
            }
        }
    }

    /// The send time of the byte at `seq`, if the log covers it.
    pub fn time_of(&self, seq: u32) -> Option<u32> {
        if self.flights.is_empty() || seq_lt(seq, self.start) {
            return None;
        }
        let at = self.flights.partition_point(|&(end, _)| seq_le(end, seq));
        self.flights.get(at).map(|&(_, t)| t)
    }

    /// The end of the newest flight, which is the highest sequence number
    /// that the log knows.
    pub fn end(&self) -> Option<u32> {
        self.flights.back().map(|&(end, _)| end)
    }

    /// Drops what `snd_una` passed.
    pub fn advance(&mut self, snd_una: u32) {
        while let Some(&(end, _)) = self.flights.front() {
            if seq_le(end, snd_una) {
                self.flights.pop_front();
                self.start = end;
            } else {
                break;
            }
        }
        if !self.flights.is_empty() && seq_lt(self.start, snd_una) {
            self.start = snd_una;
        }
    }

    /// The end of the last flight that left at or before `deadline` (a
    /// truncated clock reading) and strictly before `newer`, or `None` when
    /// the first flight is already too young. The times of the log never
    /// decrease, so the flights that qualify are a prefix.
    pub fn old_end(&self, deadline: u32, newer: u32) -> Option<u32> {
        let at = self
            .flights
            .partition_point(|&(_, t)| since(deadline, t) >= 0 && since(newer, t) > 0);
        at.checked_sub(1)
            .and_then(|i| self.flights.get(i))
            .map(|&(end, _)| end)
    }

    /// The first flight that ends beyond `after` (the first flight when
    /// `after` is `None`), as its end and time.
    pub fn first_after(&self, after: Option<u32>) -> Option<(u32, u32)> {
        let at = match after {
            Some(a) => self.flights.partition_point(|&(end, _)| seq_le(end, a)),
            None => 0,
        };
        self.flights.get(at).copied()
    }
}

/// The state of RACK (RFC 8985 section 6.2): the most recently sent segment
/// that is known to be delivered, and the round trip of that delivery.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rack {
    /// The send time of the most recently sent delivered segment (`RACK.xmit_ts`).
    ts: u32,
    /// The end of that segment (`RACK.end_seq`).
    end: u32,
    /// The round trip of that segment, in microseconds (`RACK.rtt`).
    rtt_us: u32,
    /// Whether any segment was delivered since the state was made.
    valid: bool,
    /// The smallest round trip seen, in microseconds (`RACK.min_rtt`).
    min_rtt_us: u32,
}

/// The reordering window never falls below this, in microseconds, so that a
/// link with a round trip of a few microseconds does not call every
/// out of order segment lost.
pub const MIN_REO_WND_US: u32 = 100;

impl Rack {
    /// Takes a round trip sample for the minimum.
    pub fn note_rtt(&mut self, rtt_us: u32) {
        let rtt = rtt_us.max(1);
        if self.min_rtt_us == 0 || rtt < self.min_rtt_us {
            self.min_rtt_us = rtt;
        }
    }

    /// The smallest round trip seen, if any.
    pub fn min_rtt_us(&self) -> Option<u32> {
        (self.min_rtt_us != 0).then_some(self.min_rtt_us)
    }

    /// A segment that left at `ts`, ending at `end`, was delivered at `now`.
    /// Counts only when it left after the one that is recorded
    /// (RFC 8985 section 6.2, step 2).
    pub fn delivered(&mut self, ts: u32, end: u32, now_us: i64) {
        let rtt = since(t32(now_us), ts).max(1) as u32;
        self.note_rtt(rtt);
        let newer =
            !self.valid || since(ts, self.ts) > 0 || (ts == self.ts && seq_lt(self.end, end));
        if newer {
            self.valid = true;
            self.ts = ts;
            self.end = end;
            self.rtt_us = rtt;
        }
    }

    /// Whether a delivery was recorded.
    pub fn valid(&self) -> bool {
        self.valid
    }

    /// The send time of the delivered segment.
    pub fn ts(&self) -> u32 {
        self.ts
    }

    /// The reordering window of RFC 8985 section 6.2: a quarter of the
    /// smallest round trip (the first value of the RFC, which a stack
    /// without D-SACK keeps), at least [`MIN_REO_WND_US`].
    pub fn reo_wnd_us(&self) -> u32 {
        (self.min_rtt_us / 4).max(MIN_REO_WND_US)
    }

    /// How long after its send time a segment older than the delivered one is
    /// lost: `RACK.rtt + reo_wnd`.
    pub fn lost_after_us(&self) -> u32 {
        self.rtt_us.saturating_add(self.reo_wnd_us())
    }

    /// The end of the data that RACK calls lost at `now_us`: every segment
    /// that left before the delivered one and at least [`Rack::lost_after_us`]
    /// ago. A hole that starts below it is lost. A segment that left at the
    /// same reading of the clock counts as older when it lies below the
    /// delivered one, because the stack sends in order of sequence
    /// (RFC 8985 section 6.2).
    pub fn lost_to(&self, log: &TxLog, now_us: i64) -> Option<u32> {
        if !self.valid {
            return None;
        }
        let deadline = t32(now_us).wrapping_sub(self.lost_after_us());
        let older = log.old_end(deadline, self.ts);
        let same = (since(deadline, self.ts) >= 0).then_some(self.end);
        match (older, same) {
            (Some(a), Some(b)) => Some(if seq_lt(a, b) { b } else { a }),
            (a, b) => a.or(b),
        }
    }

    /// When the oldest segment that is older than the delivered one and not
    /// lost yet (`after` is the end of what [`Rack::lost_to`] returned) turns
    /// lost, as a time on the clock of `now_us`, or `None` when no segment
    /// is waiting for the window.
    pub fn reorder_deadline(&self, log: &TxLog, now_us: i64, after: Option<u32>) -> Option<i64> {
        if !self.valid {
            return None;
        }
        let (_, t) = log.first_after(after)?;
        let older = since(self.ts, t);
        if older < 0 || (older == 0 && after.is_some_and(|a| !seq_lt(a, self.end))) {
            return None;
        }
        let at = t.wrapping_add(self.lost_after_us());
        let wait = since(at, t32(now_us));
        Some(now_us + i64::from(wait.max(0)))
    }
}

/// The smallest probe timeout, in microseconds (RFC 8985 section 7.2 lets an
/// implementation choose it; Linux uses 10 ms).
pub const MIN_PTO_US: i64 = 10_000;

/// The worst case delay of the acknowledgment of a single segment, which the
/// probe timeout adds when only one segment is outstanding (`WCDelAckT` of
/// RFC 8985 section 7.2).
pub const WC_DELACK_US: i64 = 200_000;

/// The probe timeout of RFC 8985 section 7.2: twice the smoothed round trip
/// time, and with one segment outstanding enough to cover a delayed ACK,
/// never below [`MIN_PTO_US`] and never above the retransmission timeout.
pub fn pto_us(srtt_us: i64, flight_segments: u32, rto_us: i64) -> i64 {
    let mut pto = 2 * srtt_us;
    if flight_segments <= 1 {
        pto = pto.max(srtt_us + srtt_us / 2 + WC_DELACK_US);
    }
    pto.max(MIN_PTO_US).min(rto_us)
}

/// How much faster than the window per round trip the pacing rate is: twice
/// in slow start and 1.25 times in congestion avoidance (the ratios of the
/// Linux kernel, `tcp_pacing_ss_ratio` and `tcp_pacing_ca_ratio`), in percent.
pub const PACING_SS_PERCENT: u64 = 200;
/// See [`PACING_SS_PERCENT`].
pub const PACING_CA_PERCENT: u64 = 125;

/// The time that a segment of `len` bytes takes at the pacing rate, in
/// microseconds, for a window of `cwnd` bytes and a round trip of `srtt_us`.
/// The rate is `percent / 100 * cwnd / srtt`.
pub fn pacing_gap_us(len: u32, cwnd: u32, srtt_us: i64, slow_start: bool) -> i64 {
    let percent = if slow_start {
        PACING_SS_PERCENT
    } else {
        PACING_CA_PERCENT
    };
    let rate = u64::from(cwnd.max(1)) * percent;
    let gap = u64::from(len) * u64::try_from(srtt_us.max(0)).unwrap_or(0) * 100 / rate;
    i64::try_from(gap).unwrap_or(i64::MAX)
}

/// How far behind the clock the pacing time may fall before it is moved to
/// the clock, in microseconds. It is the burst that a late wake up may send
/// to catch up, and it bounds the burst after an idle time.
pub const PACING_LAG_US: i64 = 1_000;

/// How far ahead of the clock the pacing time may be and a segment still
/// leaves, in microseconds: a wake up sends about this much time of data at
/// once. The Linux kernel sizes a burst to a millisecond of the pacing rate
/// (`sk_pacing_shift` of 10) for the same reason: a timer cannot fire for
/// every segment, and a driver that sleeps in steps of a millisecond would
/// cap the rate at a few segments per step without it.
pub const PACING_QUANTUM_US: i64 = 1_000;

#[cfg(test)]
impl Rack {
    fn rtt_us_for_test(&self) -> u32 {
        self.rtt_us
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_at_one_reading_of_the_clock_is_one_entry() {
        let mut log = TxLog::default();
        log.push(1000, 2000, 50);
        log.push(2000, 3000, 50);
        log.push(3000, 4000, 80);
        assert_eq!(log.len(), 2);
        assert_eq!(log.time_of(1000), Some(50));
        assert_eq!(log.time_of(2999), Some(50));
        assert_eq!(log.time_of(3000), Some(80));
        assert_eq!(log.time_of(3999), Some(80));
        assert_eq!(log.time_of(4000), None);
        assert_eq!(log.time_of(999), None);
        assert_eq!(log.end(), Some(4000));
    }

    #[test]
    fn advancing_drops_whole_flights_and_keeps_the_one_that_is_cut() {
        let mut log = TxLog::default();
        log.push(0, 1000, 10);
        log.push(1000, 2000, 20);
        log.push(2000, 3000, 30);
        log.advance(1500);
        assert_eq!(log.len(), 2);
        assert_eq!(log.time_of(1500), Some(20));
        assert_eq!(log.time_of(1499), None);
        log.advance(3000);
        assert!(log.is_empty());
    }

    #[test]
    fn a_gap_in_the_data_starts_the_log_over() {
        let mut log = TxLog::default();
        log.push(0, 1000, 10);
        log.push(5000, 6000, 20);
        assert_eq!(log.len(), 1);
        assert_eq!(log.time_of(0), None);
        assert_eq!(log.time_of(5500), Some(20));
    }

    #[test]
    fn the_log_works_across_the_sequence_wrap_and_the_clock_wrap() {
        let mut log = TxLog::default();
        let base = u32::MAX - 1500;
        log.push(base, base.wrapping_add(1000), i64::from(u32::MAX) - 5);
        log.push(
            base.wrapping_add(1000),
            base.wrapping_add(2000),
            i64::from(u32::MAX) + 20,
        );
        assert_eq!(log.time_of(base.wrapping_add(10)), Some(u32::MAX - 5));
        assert_eq!(log.time_of(base.wrapping_add(1500)), Some(19));
        // The first flight is older than 12 microseconds before the wrap and
        // the second is not.
        assert_eq!(
            log.old_end(u32::MAX, u32::MAX.wrapping_add(100)),
            Some(base.wrapping_add(1000))
        );
    }

    #[test]
    fn old_end_takes_the_flights_before_the_delivered_one_that_are_old_enough() {
        let mut log = TxLog::default();
        log.push(0, 1000, 100);
        log.push(1000, 2000, 200);
        log.push(2000, 3000, 300);
        // Delivered one left at 300: the first two are older than it. At a
        // deadline of 150 only the first is old enough.
        assert_eq!(log.old_end(150, 300), Some(1000));
        assert_eq!(log.old_end(250, 300), Some(2000));
        // A flight that left at the same time as the delivered one is not
        // older than it.
        assert_eq!(log.old_end(1000, 300), Some(2000));
        assert_eq!(log.old_end(50, 300), None);
    }

    #[test]
    fn a_delivery_counts_only_when_it_left_after_the_recorded_one() {
        let mut r = Rack::default();
        assert!(!r.valid());
        r.delivered(1000, 5000, 11_000);
        assert_eq!((r.ts(), r.lost_after_us()), (1000, 12_500));
        // An older segment delivered later does not move it.
        r.delivered(500, 4000, 12_000);
        assert_eq!(r.ts(), 1000);
        // A newer one does, with its own round trip.
        r.delivered(2000, 6000, 12_500);
        assert_eq!(r.ts(), 2000);
        assert_eq!(r.min_rtt_us(), Some(10_000));
    }

    #[test]
    fn the_reordering_window_is_a_quarter_of_the_smallest_round_trip_with_a_floor() {
        let mut r = Rack::default();
        r.note_rtt(8_000);
        assert_eq!(r.reo_wnd_us(), 2_000);
        r.note_rtt(100);
        assert_eq!(r.reo_wnd_us(), MIN_REO_WND_US);
        r.note_rtt(40_000);
        assert_eq!(r.min_rtt_us(), Some(100));
    }

    #[test]
    fn a_segment_is_lost_when_a_later_one_was_delivered_and_a_round_trip_and_the_window_passed() {
        let mut log = TxLog::default();
        log.push(0, 1000, 1_000);
        log.push(1000, 2000, 1_100);
        log.push(2000, 3000, 1_200);
        let mut r = Rack::default();
        // The segment of the third flight was delivered after 10 ms.
        r.delivered(1_200, 3000, 11_200);
        assert_eq!(r.rtt_us_for_test(), 10_000);
        // The window is a quarter of the smallest round trip, 2,500.
        // The first flight is lost 12,500 after it left, at 13,500.
        assert_eq!(r.lost_to(&log, 13_000), None);
        assert_eq!(r.lost_to(&log, 13_500), Some(1000));
        assert_eq!(r.lost_to(&log, 13_600), Some(2000));
        // The delivered flight is lost only below the delivered segment, and
        // only when the window has passed since it left.
        assert_eq!(r.lost_to(&log, 13_700), Some(3000));
        assert_eq!(r.lost_to(&log, 100_000), Some(3000));
    }

    #[test]
    fn the_reorder_timer_waits_for_the_oldest_segment() {
        let mut log = TxLog::default();
        log.push(0, 1000, 1_000);
        log.push(1000, 2000, 1_100);
        let mut r = Rack::default();
        assert_eq!(r.reorder_deadline(&log, 5_000, None), None);
        r.delivered(1_100, 2000, 11_100);
        // The first flight is lost at 1,000 + 10,000 + 2,500.
        assert_eq!(r.reorder_deadline(&log, 11_200, None), Some(13_500));
        // Past it, the timer is due at once.
        assert_eq!(r.reorder_deadline(&log, 20_000, None), Some(20_000));
        // After the first flight, the delivered flight waits for its own
        // window below the delivered segment (its end here).
        assert_eq!(r.reorder_deadline(&log, 11_200, Some(1000)), Some(13_600));
        // Nothing waits when everything below the delivered segment is lost.
        assert_eq!(r.reorder_deadline(&log, 11_200, Some(2000)), None);
    }

    #[test]
    fn the_probe_timeout_is_twice_the_round_trip_and_covers_a_delayed_ack_for_one_segment() {
        assert_eq!(pto_us(20_000, 10, 200_000), 40_000);
        // One segment: 1.5 srtt plus 200 ms, but never above the timeout.
        assert_eq!(pto_us(20_000, 1, 500_000), 230_000);
        assert_eq!(pto_us(20_000, 1, 200_000), 200_000);
        // The floor of 10 ms.
        assert_eq!(pto_us(500, 10, 200_000), MIN_PTO_US);
        // Never above the timeout.
        assert_eq!(pto_us(150_000, 10, 200_000), 200_000);
    }

    #[test]
    fn the_pacing_gap_follows_the_rate_of_the_window_per_round_trip() {
        // A window of 100,000 bytes per 10 ms is 10 MB/s; twice that in slow
        // start, so 1,500 bytes take 75 us, and 1.25 times in avoidance, 120 us.
        assert_eq!(pacing_gap_us(1500, 100_000, 10_000, true), 75);
        assert_eq!(pacing_gap_us(1500, 100_000, 10_000, false), 120);
        assert_eq!(pacing_gap_us(1500, 0, 10_000, false), 12_000_000);
        assert_eq!(pacing_gap_us(1500, 100_000, 0, true), 0);
    }
}
