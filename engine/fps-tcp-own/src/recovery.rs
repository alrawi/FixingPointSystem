// SPDX-License-Identifier: GPL-3.0-only
//! Loss recovery of one connection (TRN-23): the round trip estimator with the
//! retransmission timeout of RFC 6298, the duplicate acknowledgment rule and
//! NewReno of RFC 5681 and RFC 6582, and the queue of out of order data.
//!
//! Everything here is plain data and pure functions of its arguments, so the
//! unit tests below fix every input. The state machine of [`crate::host`]
//! calls these types and owns the sockets and the timers.
//!
//! # Memory
//!
//! A connection holds a [`Recovery`] in a box only while it needs one: while
//! data is in flight or queued to send, or while out of order data waits for
//! its hole. An idle connection holds a `None` of 8 bytes, which is the whole
//! per socket cost of this module, so a million idle sockets stay flat. The
//! cost of dropping the box is that the window restarts at the initial window
//! after an idle time (which RFC 5681 section 4.1 asks for in any case) and
//! that the smoothed round trip time and the slow start threshold are
//! forgotten. The retransmission timeout itself stays in the connection.

use super::host::{seq_le, seq_lt};
use super::sack::Scoreboard;

/// The clock granularity `G` of RFC 6298, in microseconds.
pub const CLOCK_GRANULARITY_US: i64 = 1_000;

/// The duplicate acknowledgments that start a fast retransmit.
pub const DUPACK_THRESHOLD: u8 = 3;

/// The round trip estimator of RFC 6298 section 2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RttEstimator {
    srtt_us: i64,
    rttvar_us: i64,
    sampled: bool,
}

impl RttEstimator {
    /// Takes one round trip measurement `r_us`, in microseconds. The first
    /// one sets `SRTT = R` and `RTTVAR = R / 2`. Later ones follow
    /// `RTTVAR = 3/4 RTTVAR + 1/4 |SRTT - R|` and `SRTT = 7/8 SRTT + 1/8 R`,
    /// with the variance updated first.
    pub fn sample(&mut self, r_us: i64) {
        let r = r_us.max(1);
        if self.sampled {
            let err = (self.srtt_us - r).abs();
            self.rttvar_us = (3 * self.rttvar_us + err) / 4;
            self.srtt_us = (7 * self.srtt_us + r) / 8;
        } else {
            self.srtt_us = r;
            self.rttvar_us = r / 2;
            self.sampled = true;
        }
    }

    /// The smoothed round trip time, or `None` before the first sample.
    pub fn srtt_us(&self) -> Option<i64> {
        self.sampled.then_some(self.srtt_us)
    }

    /// The round trip time variation, or `None` before the first sample.
    pub fn rttvar_us(&self) -> Option<i64> {
        self.sampled.then_some(self.rttvar_us)
    }

    /// The retransmission timeout `SRTT + max(G, 4 RTTVAR)`, kept between
    /// `min_us` and `max_us`, or `None` before the first sample.
    pub fn rto_us(&self, min_us: i64, max_us: i64) -> Option<i64> {
        self.sampled.then(|| {
            let spread = (4 * self.rttvar_us).max(CLOCK_GRANULARITY_US);
            (self.srtt_us + spread).clamp(min_us, max_us.max(min_us))
        })
    }
}

/// The timeout after a timeout: twice the old one, up to `max_us` (RFC 6298,
/// rule 5.5).
pub fn backoff_us(rto_us: i64, max_us: i64) -> i64 {
    rto_us.saturating_mul(2).min(max_us)
}

/// Whether an acknowledgment is a duplicate in the sense of RFC 5681 section
/// 2: it carries the same acknowledgment number as the oldest unacknowledged
/// byte, no data, no SYN, FIN, or RST, and the same window as the last one,
/// and data is outstanding.
pub fn is_dupack(a: &AckView) -> bool {
    a.ack == a.snd_una
        && a.snd_una != a.snd_max
        && a.payload_len == 0
        && !a.control
        && a.window == a.last_window
}

/// What [`is_dupack`] needs to know of one incoming segment.
#[derive(Clone, Copy, Debug)]
pub struct AckView {
    /// The acknowledgment number of the segment.
    pub ack: u32,
    /// The oldest unacknowledged sequence number.
    pub snd_una: u32,
    /// The highest sequence number sent so far.
    pub snd_max: u32,
    /// The data bytes of the segment.
    pub payload_len: usize,
    /// Whether the segment has SYN, FIN, or RST.
    pub control: bool,
    /// The window of the segment.
    pub window: u32,
    /// The window of the last acknowledgment before it.
    pub last_window: u32,
}

/// What the congestion control asks the sender to do after an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Nothing special: send whatever the window allows.
    None,
    /// Send the oldest unacknowledged segment again now.
    Retransmit,
}

/// NewReno: slow start and congestion avoidance (RFC 5681 section 3.1), and
/// fast retransmit and fast recovery with partial acknowledgments (RFC 5681
/// section 3.2 and RFC 6582). The window is counted in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NewReno {
    cwnd: u32,
    ssthresh: u32,
    mss: u32,
    dupacks: u8,
    in_recovery: bool,
    recover: u32,
    has_recover: bool,
    /// The recovery runs on the SACK scoreboard (RFC 6675): the window does
    /// not inflate and a partial acknowledgment sends nothing by itself.
    sack: bool,
}

/// The largest congestion window, which no real flight of this stack reaches.
const CWND_MAX: u32 = 1 << 30;

impl NewReno {
    /// A new state with an initial window of `iw_segments` segments of `mss`
    /// bytes and no slow start threshold.
    pub fn new(mss: u16, iw_segments: u32) -> Self {
        let mss = u32::from(mss).max(1);
        Self {
            cwnd: mss.saturating_mul(iw_segments.max(1)),
            ssthresh: u32::MAX,
            mss,
            dupacks: 0,
            in_recovery: false,
            recover: 0,
            has_recover: false,
            sack: false,
        }
    }

    /// The congestion window in bytes.
    pub fn cwnd(&self) -> u32 {
        self.cwnd
    }

    /// The slow start threshold in bytes.
    pub fn ssthresh(&self) -> u32 {
        self.ssthresh
    }

    /// Whether fast recovery is on.
    pub fn in_recovery(&self) -> bool {
        self.in_recovery
    }

    /// The duplicate acknowledgments counted since the last new one.
    pub fn dupacks(&self) -> u8 {
        self.dupacks
    }

    /// `max(FlightSize / 2, 2 SMSS)` of RFC 5681 equation 4.
    fn reduced(&self, flight: u32) -> u32 {
        (flight / 2).max(2 * self.mss)
    }

    /// A duplicate acknowledgment with acknowledgment number `ack` arrived.
    /// `flight` is the data outstanding and `snd_max` the highest sequence
    /// number sent. The third one starts a fast retransmit, unless the
    /// acknowledgment stops short of `recover`, the end of the flight of an
    /// earlier recovery or timeout (RFC 6582 section 3.2, step 2; `recover`
    /// here is the sequence number after the highest byte sent, so an
    /// acknowledgment of it covers more than the old flight). More of them
    /// inflate the window by one segment each while the recovery lasts.
    pub fn on_dupack(&mut self, ack: u32, flight: u32, snd_max: u32) -> Action {
        self.dupacks = self.dupacks.saturating_add(1);
        if self.in_recovery {
            if !self.sack {
                self.cwnd = self.cwnd.saturating_add(self.mss).min(CWND_MAX);
            }
            return Action::None;
        }
        if self.dupacks != DUPACK_THRESHOLD {
            return Action::None;
        }
        if self.has_recover && seq_lt(ack, self.recover) {
            return Action::None;
        }
        self.ssthresh = self.reduced(flight);
        self.recover = snd_max;
        self.has_recover = true;
        self.in_recovery = true;
        self.cwnd = self.ssthresh.saturating_add(3 * self.mss).min(CWND_MAX);
        Action::Retransmit
    }

    /// A duplicate acknowledgment on a connection with SACK (RFC 6675
    /// section 5). The recovery starts on the third duplicate, or at once
    /// when the scoreboard presumes the first segment lost (`lost`), under
    /// the same `recover` guard as [`NewReno::on_dupack`]. The window falls
    /// to the threshold without inflation, and the scoreboard decides what to
    /// send through the pipe.
    pub fn on_dupack_sack(&mut self, ack: u32, flight: u32, snd_max: u32, lost: bool) -> Action {
        self.dupacks = self.dupacks.saturating_add(1);
        if self.in_recovery || !(lost || self.dupacks >= DUPACK_THRESHOLD) {
            return Action::None;
        }
        if self.has_recover && seq_lt(ack, self.recover) {
            return Action::None;
        }
        self.ssthresh = self.reduced(flight);
        self.cwnd = self.ssthresh;
        self.recover = snd_max;
        self.has_recover = true;
        self.in_recovery = true;
        self.sack = true;
        Action::Retransmit
    }

    /// Whether the recovery that runs is driven by the SACK scoreboard.
    pub fn sack_recovery(&self) -> bool {
        self.in_recovery && self.sack
    }

    /// What the sender may have in flight: the congestion window, plus one
    /// segment for each of the first two duplicate acknowledgments, so that
    /// new data goes out and more duplicates can follow (Limited Transmit,
    /// RFC 3042, which RFC 5681 section 3.2 recommends).
    pub fn send_window(&self) -> u32 {
        let extra = if !self.in_recovery && (1..DUPACK_THRESHOLD).contains(&self.dupacks) {
            u32::from(self.dupacks) * self.mss
        } else {
            0
        };
        self.cwnd.saturating_add(extra)
    }

    /// An acknowledgment of `acked` new bytes up to `ack` arrived. In fast recovery a partial
    /// acknowledgment (below `recover`) asks for a retransmission of the next
    /// hole and deflates the window, and a full one ends the recovery.
    pub fn on_new_ack(&mut self, ack: u32, acked: u32) -> Action {
        self.dupacks = 0;
        if self.in_recovery {
            if self.sack && seq_lt(ack, self.recover) {
                // A partial acknowledgment under SACK: the window stays at
                // the threshold and the scoreboard picks the next segment.
                return Action::None;
            }
            if seq_le(self.recover, ack) {
                // A full acknowledgment: the inflated window falls to the
                // threshold (RFC 5681 section 3.2, step 6, which RFC 6582
                // section 3.2 step 3 keeps as its second option).
                self.cwnd = self.ssthresh;
                self.in_recovery = false;
                self.sack = false;
                return Action::None;
            }
            // A partial acknowledgment.
            self.cwnd = self.cwnd.saturating_sub(acked);
            if acked >= self.mss {
                self.cwnd = self.cwnd.saturating_add(self.mss);
            }
            self.cwnd = self.cwnd.max(self.mss);
            return Action::Retransmit;
        }
        let mut acked = acked;
        if self.cwnd < self.ssthresh {
            // Slow start: grow by the bytes acknowledged (appropriate byte
            // counting without the limit of RFC 5681 equation 2, because this
            // stack acknowledges a whole batch of segments with one ACK, and
            // a limit of one segment per ACK would not double the window per
            // round trip), up to the threshold.
            let room = self.ssthresh - self.cwnd;
            let grow = acked.min(room);
            self.cwnd = self.cwnd.saturating_add(grow);
            acked -= grow;
        }
        if acked > 0 && self.cwnd >= self.ssthresh {
            // Congestion avoidance: about one segment per round trip, as
            // RFC 5681 equation 3 gives for one segment per ACK and
            // proportionally more for an ACK of several.
            let step =
                (u64::from(self.mss) * u64::from(acked) / u64::from(self.cwnd.max(1))).max(1);
            self.cwnd = self.cwnd.saturating_add(u32::try_from(step).unwrap_or(1));
        }
        self.cwnd = self.cwnd.min(CWND_MAX);
        Action::None
    }

    /// The retransmission timer fired with `flight` bytes outstanding and
    /// `snd_max` the highest sequence number sent: the threshold falls to
    /// half the flight, the window to one segment, and `recover` marks the
    /// highest sequence number so that duplicate acknowledgments of the old
    /// flight do not start another recovery.
    pub fn on_timeout(&mut self, flight: u32, snd_max: u32) {
        self.ssthresh = self.reduced(flight);
        self.cwnd = self.mss;
        self.in_recovery = false;
        self.sack = false;
        self.dupacks = 0;
        self.recover = snd_max;
        self.has_recover = true;
    }
}

/// Segments that arrived beyond a hole, held until the hole fills. Segments
/// are kept in order of sequence number without overlap, and the bytes held
/// never exceed the receive window at the time of insertion.
#[derive(Debug, Default)]
pub struct OooQueue {
    segs: Vec<(u32, Vec<u8>)>,
    fin: Option<u32>,
    /// The first byte of the segment that arrived last, which the first
    /// SACK block must cover (RFC 2018 section 4).
    recent: Option<u32>,
}

impl OooQueue {
    /// Whether nothing is held, neither data nor a FIN.
    pub fn is_empty(&self) -> bool {
        self.segs.is_empty() && self.fin.is_none()
    }

    /// The bytes held.
    pub fn bytes(&self) -> usize {
        self.segs.iter().map(|(_, d)| d.len()).sum()
    }

    /// The number of separate runs held.
    pub fn runs(&self) -> usize {
        self.segs.len()
    }

    /// Keeps `data` that starts at `seq`, after `rcv_nxt`, if it lies inside
    /// the window of `window` bytes from `rcv_nxt`. Bytes beyond the window
    /// are cut and bytes already held are not kept twice. Returns the bytes
    /// that were new.
    pub fn insert(&mut self, rcv_nxt: u32, seq: u32, data: &[u8], window: u32) -> usize {
        let start = seq.wrapping_sub(rcv_nxt);
        if start == 0 || start >= window || data.is_empty() {
            return 0;
        }
        self.recent = Some(seq);
        let end = start.saturating_add(data.len() as u32).min(window);
        // The parts of [start, end) that no held run covers.
        let mut pieces = vec![(start, end)];
        for (s, d) in &self.segs {
            let a = s.wrapping_sub(rcv_nxt);
            let b = a.saturating_add(d.len() as u32);
            let mut next = Vec::with_capacity(pieces.len() + 1);
            for (ps, pe) in pieces {
                if pe <= a || ps >= b {
                    next.push((ps, pe));
                } else {
                    if ps < a {
                        next.push((ps, a));
                    }
                    if pe > b {
                        next.push((b, pe));
                    }
                }
            }
            pieces = next;
        }
        let mut added = 0;
        for (ps, pe) in pieces {
            let from = (ps - start) as usize;
            let to = (pe - start) as usize;
            self.segs
                .push((rcv_nxt.wrapping_add(ps), data[from..to].to_vec()));
            added += to - from;
        }
        self.segs.sort_by_key(|(s, _)| s.wrapping_sub(rcv_nxt));
        self.merge(rcv_nxt);
        added
    }

    /// Joins runs that touch.
    fn merge(&mut self, rcv_nxt: u32) {
        let mut merged: Vec<(u32, Vec<u8>)> = Vec::with_capacity(self.segs.len());
        for (s, d) in self.segs.drain(..) {
            if let Some((ps, pd)) = merged.last_mut() {
                let prev_end = ps.wrapping_sub(rcv_nxt).wrapping_add(pd.len() as u32);
                if prev_end == s.wrapping_sub(rcv_nxt) {
                    pd.extend_from_slice(&d);
                    continue;
                }
            }
            merged.push((s, d));
        }
        self.segs = merged;
    }

    /// Notes that the peer's FIN sits at sequence number `seq`, after the data
    /// held.
    /// Writes the SACK blocks for an ACK into `opts`: first the run that
    /// holds the segment that arrived last, then the others from the lowest
    /// sequence number, as many as the option holds.
    pub fn sack_blocks(&self, rcv_nxt: u32, opts: &mut crate::wire::Options) {
        let recent = self.recent.map(|r| r.wrapping_sub(rcv_nxt));
        let mut first = None;
        for (i, (s, d)) in self.segs.iter().enumerate() {
            let a = s.wrapping_sub(rcv_nxt);
            if recent.is_some_and(|r| r >= a && r < a.saturating_add(d.len() as u32)) {
                first = Some(i);
                opts.push_block(*s, s.wrapping_add(d.len() as u32));
                break;
            }
        }
        for (i, (s, d)) in self.segs.iter().enumerate() {
            if Some(i) != first && !opts.push_block(*s, s.wrapping_add(d.len() as u32)) {
                break;
            }
        }
    }

    /// Notes that the peer's FIN sits at sequence number `seq`, after the data
    /// held.
    pub fn set_fin(&mut self, seq: u32) {
        self.fin = Some(seq);
    }

    /// Whether the FIN held comes next after `rcv_nxt`.
    pub fn fin_at(&self, rcv_nxt: u32) -> bool {
        self.fin == Some(rcv_nxt)
    }

    /// Forgets the FIN, after the connection took it.
    pub fn clear_fin(&mut self) {
        self.fin = None;
    }

    /// Moves the runs that now follow `*rcv_nxt` to `sink`, at most `space`
    /// bytes, advances `*rcv_nxt`, drops what in-order data already covered,
    /// and returns the bytes moved.
    pub fn drain_into(
        &mut self,
        rcv_nxt: &mut u32,
        sink: &mut std::collections::VecDeque<u8>,
        space: usize,
    ) -> usize {
        let mut moved = 0;
        while let Some((s, d)) = self.segs.first_mut() {
            if seq_lt(*s, *rcv_nxt) {
                // Covered at least in part by data that came in order.
                let skip = rcv_nxt.wrapping_sub(*s) as usize;
                if skip >= d.len() {
                    self.segs.remove(0);
                    continue;
                }
                d.drain(..skip);
                *s = *rcv_nxt;
            }
            if *s != *rcv_nxt || moved >= space {
                break;
            }
            let take = d.len().min(space - moved);
            sink.extend(&d[..take]);
            *rcv_nxt = rcv_nxt.wrapping_add(take as u32);
            moved += take;
            if take == d.len() {
                self.segs.remove(0);
            } else {
                d.drain(..take);
                *s = *rcv_nxt;
                break;
            }
        }
        moved
    }
}

/// The recovery state of a connection that has data in flight or out of order
/// data waiting.
#[derive(Debug)]
pub struct Recovery {
    /// Congestion control.
    pub cc: NewReno,
    /// The round trip estimator.
    pub rtt: RttEstimator,
    /// The highest sequence number sent, including a FIN. The connection's
    /// `snd_nxt` falls below it after a timeout (go back N) and an
    /// acknowledgment can jump past `snd_nxt` up to it.
    pub snd_max: u32,
    /// Whether a FIN was sent, which a timeout does not undo for `snd_max`.
    pub fin_sent: bool,
    /// Send the oldest unacknowledged segment again at the next output.
    pub rtx: bool,
    /// The one segment being timed: the acknowledgment number that covers it
    /// and the time of its first send (Karn's rule: a segment that is sent
    /// again is not timed).
    pub probe: Option<(u32, i64)>,
    /// Data that arrived beyond a hole.
    pub ooo: OooQueue,
    /// The SACK scoreboard of the sender side (RFC 6675).
    pub sb: Scoreboard,
}

impl Recovery {
    /// A state for a connection that sends segments of `mss` bytes, whose
    /// next sequence number is `snd_nxt`.
    pub fn new(mss: u16, iw_segments: u32, snd_nxt: u32) -> Self {
        Self {
            cc: NewReno::new(mss, iw_segments),
            rtt: RttEstimator::default(),
            snd_max: snd_nxt,
            fin_sent: false,
            rtx: false,
            probe: None,
            ooo: OooQueue::default(),
            sb: Scoreboard::default(),
        }
    }

    /// Starts timing a segment that ends at `end` unless one is timed
    /// already.
    pub fn time_segment(&mut self, end: u32, now: i64) {
        if self.probe.is_none() {
            self.probe = Some((end, now));
        }
    }

    /// Stops timing, because a segment was sent again and its sample would
    /// be ambiguous (Karn).
    pub fn retransmitted(&mut self) {
        self.probe = None;
    }

    /// An acknowledgment up to `ack` arrived at `now`: returns the round trip
    /// sample if it covers the timed segment.
    pub fn take_sample(&mut self, ack: u32, now: i64) -> Option<i64> {
        let (end, start) = self.probe?;
        if seq_le(end, ack) {
            self.probe = None;
            Some((now - start).max(1))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    const MSS: u16 = 1000;
    const M: u32 = 1000;

    // ---- RTO computation (RFC 6298) ----

    #[test]
    fn the_first_sample_sets_srtt_and_half_of_it_as_variance() {
        let mut e = RttEstimator::default();
        assert_eq!(e.rto_us(0, i64::MAX), None);
        e.sample(40_000);
        assert_eq!(e.srtt_us(), Some(40_000));
        assert_eq!(e.rttvar_us(), Some(20_000));
        // RTO = SRTT + 4 RTTVAR = 40 + 80 ms.
        assert_eq!(e.rto_us(0, 60_000_000), Some(120_000));
    }

    #[test]
    fn later_samples_use_the_gains_one_eighth_and_one_quarter() {
        let mut e = RttEstimator::default();
        e.sample(40_000);
        e.sample(80_000);
        // RTTVAR = 3/4 * 20,000 + 1/4 * |40,000 - 80,000| = 25,000.
        // SRTT = 7/8 * 40,000 + 1/8 * 80,000 = 45,000.
        assert_eq!(e.rttvar_us(), Some(25_000));
        assert_eq!(e.srtt_us(), Some(45_000));
        assert_eq!(e.rto_us(0, i64::MAX), Some(45_000 + 100_000));
    }

    #[test]
    fn a_steady_round_trip_shrinks_the_variance_toward_the_granularity() {
        let mut e = RttEstimator::default();
        for _ in 0..200 {
            e.sample(10_000);
        }
        assert_eq!(e.srtt_us(), Some(10_000));
        // The variance decays to a few microseconds, so the spread is the
        // granularity G of one millisecond.
        assert!(e.rttvar_us().is_some_and(|v| v < 250));
        assert_eq!(e.rto_us(0, i64::MAX), Some(11_000));
    }

    #[test]
    fn the_timeout_is_kept_between_the_minimum_and_the_maximum() {
        let mut e = RttEstimator::default();
        e.sample(1_000);
        assert_eq!(e.rto_us(200_000, 2_000_000), Some(200_000));
        e.sample(5_000_000);
        assert_eq!(e.rto_us(200_000, 2_000_000), Some(2_000_000));
    }

    #[test]
    fn a_timeout_doubles_the_timeout_up_to_the_maximum() {
        assert_eq!(backoff_us(200_000, 2_000_000), 400_000);
        assert_eq!(backoff_us(1_500_000, 2_000_000), 2_000_000);
        assert_eq!(backoff_us(2_000_000, 2_000_000), 2_000_000);
        assert_eq!(backoff_us(i64::MAX, 2_000_000), 2_000_000);
    }

    // ---- Karn's rule ----

    #[test]
    fn one_segment_is_timed_at_a_time_and_its_ack_gives_the_sample() {
        let mut r = Recovery::new(MSS, 10, 1000);
        r.time_segment(2000, 100);
        // A second segment is not timed while the first one is.
        r.time_segment(3000, 150);
        assert_eq!(r.take_sample(1500, 200), None);
        assert_eq!(r.take_sample(2000, 340), Some(240));
        // The sample is used once.
        assert_eq!(r.take_sample(3000, 400), None);
    }

    #[test]
    fn a_segment_that_was_sent_again_gives_no_sample() {
        let mut r = Recovery::new(MSS, 10, 1000);
        r.time_segment(2000, 100);
        r.retransmitted();
        assert_eq!(r.take_sample(2000, 340), None);
        // The next new segment is timed again.
        r.time_segment(3000, 400);
        assert_eq!(r.take_sample(3000, 450), Some(50));
    }

    #[test]
    fn the_sample_works_across_the_sequence_wrap() {
        let mut r = Recovery::new(MSS, 10, u32::MAX - 500);
        r.time_segment(499, 0);
        assert_eq!(r.take_sample(u32::MAX - 10, 10), None);
        assert_eq!(r.take_sample(600, 77), Some(77));
    }

    // ---- Duplicate acknowledgments ----

    fn view() -> AckView {
        AckView {
            ack: 5000,
            snd_una: 5000,
            snd_max: 9000,
            payload_len: 0,
            control: false,
            window: 65_535,
            last_window: 65_535,
        }
    }

    #[test]
    fn an_acknowledgment_is_a_duplicate_only_when_all_five_conditions_hold() {
        assert!(is_dupack(&view()));
        assert!(!is_dupack(&AckView {
            ack: 5001,
            ..view()
        }));
        assert!(!is_dupack(&AckView {
            snd_max: 5000,
            ..view()
        }));
        assert!(!is_dupack(&AckView {
            payload_len: 1,
            ..view()
        }));
        assert!(!is_dupack(&AckView {
            control: true,
            ..view()
        }));
        assert!(!is_dupack(&AckView {
            window: 60_000,
            ..view()
        }));
    }

    #[test]
    fn the_third_duplicate_acknowledgment_starts_a_fast_retransmit() {
        let mut cc = NewReno::new(MSS, 10);
        assert_eq!(cc.on_dupack(5000, 8 * M, 13_000), Action::None);
        assert_eq!(cc.on_dupack(5000, 8 * M, 13_000), Action::None);
        assert_eq!(cc.dupacks(), 2);
        assert!(!cc.in_recovery());
        assert_eq!(cc.on_dupack(5000, 8 * M, 13_000), Action::Retransmit);
        assert!(cc.in_recovery());
        // ssthresh = max(flight / 2, 2 MSS), window = ssthresh + 3 MSS.
        assert_eq!(cc.ssthresh(), 4 * M);
        assert_eq!(cc.cwnd(), 7 * M);
    }

    #[test]
    fn a_new_acknowledgment_resets_the_duplicate_count() {
        let mut cc = NewReno::new(MSS, 10);
        cc.on_dupack(5000, 8 * M, 13_000);
        cc.on_dupack(5000, 8 * M, 13_000);
        cc.on_new_ack(6000, M);
        assert_eq!(cc.dupacks(), 0);
        assert_eq!(cc.on_dupack(6000, 7 * M, 13_000), Action::None);
        assert_eq!(cc.dupacks(), 1);
    }

    #[test]
    fn duplicate_acknowledgments_after_a_timeout_do_not_start_another_recovery() {
        let mut cc = NewReno::new(MSS, 10);
        cc.on_timeout(8 * M, 13_000);
        // The acknowledgments of the old flight cover no more than recover.
        for _ in 0..5 {
            assert_eq!(cc.on_dupack(9000, 4 * M, 13_000), Action::None);
        }
        assert!(!cc.in_recovery());
        // Once the acknowledgment covers more than recover, three do.
        cc.on_new_ack(13_001, 4 * M);
        cc.on_dupack(13_001, 3 * M, 16_000);
        cc.on_dupack(13_001, 3 * M, 16_000);
        assert_eq!(cc.on_dupack(13_001, 3 * M, 16_000), Action::Retransmit);
    }

    #[test]
    fn duplicates_of_the_end_of_the_old_flight_belong_to_new_data() {
        // Everything up to recover was acknowledged and the next segment was
        // lost: its duplicates carry recover as the acknowledgment.
        let mut cc = NewReno::new(MSS, 10);
        cc.on_timeout(8 * M, 13_000);
        cc.on_dupack(13_000, 3 * M, 16_000);
        cc.on_dupack(13_000, 3 * M, 16_000);
        assert_eq!(cc.on_dupack(13_000, 3 * M, 16_000), Action::Retransmit);
    }

    // ---- NewReno ----

    #[test]
    fn slow_start_adds_the_acknowledged_bytes() {
        let mut cc = NewReno::new(MSS, 2);
        assert_eq!(cc.cwnd(), 2 * M);
        assert_eq!(cc.ssthresh(), u32::MAX);
        cc.on_new_ack(1000, 1000);
        assert_eq!(cc.cwnd(), 3 * M);
        // A stretch acknowledgment of three segments adds three.
        cc.on_new_ack(4000, 3000);
        assert_eq!(cc.cwnd(), 6 * M);
        // A partial segment adds what it acknowledged.
        cc.on_new_ack(4400, 400);
        assert_eq!(cc.cwnd(), 6 * M + 400);
    }

    #[test]
    fn slow_start_stops_at_the_threshold_and_avoidance_takes_the_rest() {
        let mut cc = NewReno::new(MSS, 10);
        cc.on_timeout(20 * M, 40_000);
        assert_eq!((cc.cwnd(), cc.ssthresh()), (M, 10 * M));
        // One acknowledgment of 12 segments: nine bring the window to the
        // threshold, and the other three count as avoidance (a step of
        // MSS * 3 MSS / 10 MSS = 300 bytes).
        cc.on_new_ack(52_000, 12 * M);
        assert_eq!(cc.cwnd(), 10 * M + 300);
    }

    #[test]
    fn congestion_avoidance_adds_about_one_segment_per_window() {
        let mut cc = NewReno::new(MSS, 10);
        cc.on_timeout(20 * M, 40_000);
        assert_eq!(cc.ssthresh(), 10 * M);
        assert_eq!(cc.cwnd(), M);
        // Slow start up to the threshold.
        let mut ack = 40_000;
        while cc.cwnd() < cc.ssthresh() {
            ack += M;
            cc.on_new_ack(ack, M);
        }
        assert_eq!(cc.cwnd(), 10 * M);
        // Ten acknowledgments of a segment each add one segment together,
        // within the rounding of the integer step.
        for _ in 0..10 {
            ack += M;
            cc.on_new_ack(ack, M);
        }
        assert!(cc.cwnd() > 10 * M + 900 && cc.cwnd() < 11 * M + 100);
    }

    #[test]
    fn the_window_grows_by_at_least_one_byte_per_ack_in_avoidance() {
        let mut cc = NewReno::new(10, 1);
        cc.on_timeout(8, 0);
        assert_eq!((cc.ssthresh(), cc.cwnd()), (20, 10));
        cc.on_new_ack(10, 10);
        assert_eq!(cc.cwnd(), 20);
        // Avoidance: MSS squared over the window, 100 / 20 and then 100 / 25.
        cc.on_new_ack(20, 10);
        assert_eq!(cc.cwnd(), 25);
        cc.on_new_ack(30, 10);
        assert_eq!(cc.cwnd(), 29);
        // With a window above MSS squared the step would be 0, and it is 1.
        let mut tiny = NewReno::new(1, 1);
        tiny.on_timeout(0, 0);
        tiny.on_new_ack(1, 1);
        assert_eq!(tiny.cwnd(), 2);
        tiny.on_new_ack(2, 1);
        assert_eq!(tiny.cwnd(), 3);
    }

    #[test]
    fn a_timeout_halves_the_flight_and_restarts_from_one_segment() {
        let mut cc = NewReno::new(MSS, 10);
        cc.on_timeout(20 * M, 40_000);
        assert_eq!(cc.ssthresh(), 10 * M);
        assert_eq!(cc.cwnd(), M);
        // A small flight leaves two segments as the threshold.
        cc.on_timeout(M, 41_000);
        assert_eq!(cc.ssthresh(), 2 * M);
        assert_eq!(cc.cwnd(), M);
    }

    #[test]
    fn more_duplicates_inflate_the_window_in_fast_recovery() {
        let mut cc = NewReno::new(MSS, 10);
        for _ in 0..3 {
            cc.on_dupack(5000, 10 * M, 15_000);
        }
        assert_eq!((cc.ssthresh(), cc.cwnd()), (5 * M, 8 * M));
        assert_eq!(cc.on_dupack(5000, 10 * M, 15_000), Action::None);
        assert_eq!(cc.on_dupack(5000, 10 * M, 15_000), Action::None);
        assert_eq!(cc.cwnd(), 10 * M);
    }

    #[test]
    fn a_partial_acknowledgment_retransmits_and_deflates() {
        let mut cc = NewReno::new(MSS, 10);
        for _ in 0..3 {
            cc.on_dupack(5000, 10 * M, 15_000);
        }
        assert_eq!(cc.cwnd(), 8 * M);
        // Two segments acknowledged, below recover (15,000).
        assert_eq!(cc.on_new_ack(7000, 2 * M), Action::Retransmit);
        // Deflated by 2 MSS and one MSS added back.
        assert_eq!(cc.cwnd(), 7 * M);
        assert!(cc.in_recovery());
        // Less than one segment acknowledged: deflated, nothing added back.
        assert_eq!(cc.on_new_ack(7400, 400), Action::Retransmit);
        assert_eq!(cc.cwnd(), 7 * M - 400);
    }

    #[test]
    fn a_full_acknowledgment_ends_fast_recovery_at_the_threshold() {
        let mut cc = NewReno::new(MSS, 10);
        for _ in 0..3 {
            cc.on_dupack(5000, 10 * M, 15_000);
        }
        for _ in 0..4 {
            cc.on_dupack(5000, 10 * M, 15_000);
        }
        assert_eq!(cc.cwnd(), 12 * M);
        assert_eq!(cc.on_new_ack(15_000, 10 * M), Action::None);
        assert!(!cc.in_recovery());
        assert_eq!(cc.cwnd(), 5 * M);
        // Then it grows by congestion avoidance: MSS * MSS / 5 MSS.
        cc.on_new_ack(15_000 + M, M);
        assert_eq!(cc.cwnd(), 5 * M + 200);
    }

    #[test]
    fn the_first_two_duplicates_allow_new_data_without_a_larger_window() {
        let mut cc = NewReno::new(MSS, 10);
        assert_eq!(cc.send_window(), 10 * M);
        cc.on_dupack(5000, 8 * M, 13_000);
        assert_eq!((cc.cwnd(), cc.send_window()), (10 * M, 11 * M));
        cc.on_dupack(5000, 8 * M, 13_000);
        assert_eq!((cc.cwnd(), cc.send_window()), (10 * M, 12 * M));
        // The third starts the recovery, whose window is inflated already.
        cc.on_dupack(5000, 8 * M, 13_000);
        assert_eq!(cc.send_window(), cc.cwnd());
        // A new acknowledgment takes the allowance away.
        let mut cc = NewReno::new(MSS, 10);
        cc.on_dupack(5000, 8 * M, 13_000);
        cc.on_new_ack(6000, M);
        assert_eq!(cc.send_window(), cc.cwnd());
    }

    // ---- Out of order queue ----

    fn drain(q: &mut OooQueue, rcv_nxt: &mut u32, space: usize) -> Vec<u8> {
        let mut sink = VecDeque::new();
        q.drain_into(rcv_nxt, &mut sink, space);
        sink.into_iter().collect()
    }

    fn bytes(range: std::ops::Range<u8>) -> Vec<u8> {
        range.collect()
    }

    #[test]
    fn segments_that_arrive_out_of_order_are_reassembled() {
        let mut q = OooQueue::default();
        let mut rcv = 100;
        // 100..110 is missing; 110..120, 130..140, 120..130 arrive.
        assert_eq!(q.insert(rcv, 110, &bytes(10..20), 1000), 10);
        assert_eq!(q.insert(rcv, 130, &bytes(30..40), 1000), 10);
        assert_eq!(q.runs(), 2);
        assert_eq!(q.insert(rcv, 120, &bytes(20..30), 1000), 10);
        // The three touch and form one run.
        assert_eq!(q.runs(), 1);
        assert_eq!(q.bytes(), 30);
        // Nothing moves while the hole is open.
        assert!(drain(&mut q, &mut rcv, 1000).is_empty());
        // The hole fills in order: the caller takes 100..110 itself.
        rcv += 10;
        assert_eq!(drain(&mut q, &mut rcv, 1000), bytes(10..40));
        assert_eq!(rcv, 140);
        assert!(q.is_empty());
    }

    #[test]
    fn overlaps_and_repeats_are_kept_once() {
        let mut q = OooQueue::default();
        let rcv = 0;
        assert_eq!(q.insert(rcv, 10, &bytes(10..30), 1000), 20);
        // A repeat adds nothing.
        assert_eq!(q.insert(rcv, 10, &bytes(10..30), 1000), 0);
        // An overlap at the front and one at the end add only the new bytes.
        assert_eq!(q.insert(rcv, 5, &bytes(5..15), 1000), 5);
        assert_eq!(q.insert(rcv, 25, &bytes(25..40), 1000), 10);
        // A segment that covers a run entirely adds the two flanks.
        assert_eq!(q.insert(rcv, 1, &bytes(1..50), 1000), 14);
        assert_eq!(q.bytes(), 49);
        let mut r = 1;
        assert_eq!(drain(&mut q, &mut r, 1000), bytes(1..50));
    }

    #[test]
    fn data_beyond_the_window_is_cut_or_refused() {
        let mut q = OooQueue::default();
        // Window of 100 bytes from rcv_nxt 0: bytes 90..110 keep 90..100.
        assert_eq!(q.insert(0, 90, &bytes(0..20), 100), 10);
        assert_eq!(q.bytes(), 10);
        // Starting at or beyond the edge is refused, and so is the byte at
        // rcv_nxt itself, which is in order, not out of order.
        assert_eq!(q.insert(0, 100, &bytes(0..5), 100), 0);
        assert_eq!(q.insert(0, 0, &bytes(0..5), 100), 0);
        assert_eq!(q.insert(0, 50, &[], 100), 0);
    }

    #[test]
    fn in_order_data_that_covers_a_held_run_trims_it() {
        let mut q = OooQueue::default();
        let mut rcv = 1000;
        q.insert(rcv, 1010, &bytes(10..30), 5000);
        // The caller received 1000..1015 in order.
        rcv = 1015;
        assert_eq!(drain(&mut q, &mut rcv, 5000), bytes(15..30));
        assert_eq!(rcv, 1030);
        // A run entirely behind rcv_nxt is dropped without a move.
        q.insert(1030, 1040, &bytes(0..5), 5000);
        let mut r = 1050;
        assert!(drain(&mut q, &mut r, 5000).is_empty());
        assert!(q.is_empty());
    }

    #[test]
    fn a_drain_stops_at_the_space_and_keeps_the_rest() {
        let mut q = OooQueue::default();
        let mut rcv = 0;
        q.insert(rcv, 1, &bytes(1..21), 1000);
        rcv = 1;
        assert_eq!(drain(&mut q, &mut rcv, 8), bytes(1..9));
        assert_eq!(rcv, 9);
        assert_eq!(q.bytes(), 12);
        assert_eq!(drain(&mut q, &mut rcv, 100), bytes(9..21));
        assert!(q.is_empty());
    }

    #[test]
    fn the_queue_works_across_the_sequence_wrap() {
        let mut q = OooQueue::default();
        let mut rcv = u32::MAX - 4;
        // Offsets 9..19, 6..9, and 3..6 from rcv_nxt, with the wrap between
        // the second and the third of them.
        assert_eq!(q.insert(rcv, 4, &bytes(9..19), 1000), 10);
        assert_eq!(q.insert(rcv, 1, &bytes(6..9), 1000), 3);
        assert_eq!(q.insert(rcv, u32::MAX - 1, &bytes(3..6), 1000), 3);
        assert_eq!(q.runs(), 1);
        // The caller received the first three bytes in order.
        rcv = u32::MAX - 1;
        assert_eq!(drain(&mut q, &mut rcv, 1000), bytes(3..19));
        assert_eq!(rcv, 14);
    }

    #[test]
    fn a_fin_beyond_a_hole_is_held_until_the_hole_fills() {
        let mut q = OooQueue::default();
        q.insert(100, 110, &bytes(0..10), 1000);
        q.set_fin(120);
        assert!(!q.is_empty());
        assert!(!q.fin_at(100));
        let mut rcv = 110;
        drain(&mut q, &mut rcv, 1000);
        assert_eq!(rcv, 120);
        assert!(q.fin_at(rcv));
        q.clear_fin();
        assert!(q.is_empty());
    }

    #[test]
    fn a_recovery_box_has_the_initial_window_and_no_sample() {
        let r = Recovery::new(1460, 10, 77);
        assert_eq!(r.cc.cwnd(), 14_600);
        assert_eq!(r.snd_max, 77);
        assert!(r.probe.is_none() && r.ooo.is_empty() && !r.rtx && !r.fin_sent);
    }

    // ---- SACK (TRN-24) ----

    #[test]
    fn sack_blocks_start_with_the_run_of_the_latest_segment() {
        use crate::wire::Options;
        let mut q = OooQueue::default();
        q.insert(100, 1100, &[1; 100], 10_000);
        q.insert(100, 1500, &[2; 100], 10_000);
        q.insert(100, 2000, &[3; 100], 10_000);
        let mut o = Options::default();
        q.sack_blocks(100, &mut o);
        // The segment at 2000 came last, so its run is first; the rest
        // follow from the lowest.
        assert_eq!(o.blocks(), [(2000, 2100), (1100, 1200), (1500, 1600)]);
        // A segment that joins two runs gives one block, first.
        q.insert(100, 1200, &[4; 300], 10_000);
        let mut o = Options::default();
        q.sack_blocks(100, &mut o);
        assert_eq!(o.blocks(), [(1100, 1600), (2000, 2100)]);
    }

    #[test]
    fn sack_blocks_stop_where_the_option_is_full() {
        use crate::wire::Options;
        let mut q = OooQueue::default();
        for i in 0..6u32 {
            q.insert(0, 1000 + 200 * i, &[0; 100], 10_000);
        }
        let mut o = Options::default();
        q.sack_blocks(0, &mut o);
        assert_eq!(o.blocks().len(), 4);
        assert_eq!(o.blocks()[0], (2000, 2100));
        let mut o = Options {
            timestamp: Some((0, 0)),
            ..Options::default()
        };
        q.sack_blocks(0, &mut o);
        assert_eq!(o.blocks().len(), 3);
    }

    #[test]
    fn sack_blocks_wrap_with_the_sequence_numbers() {
        use crate::wire::Options;
        let rcv_nxt = u32::MAX - 150;
        let mut q = OooQueue::default();
        q.insert(rcv_nxt, rcv_nxt.wrapping_add(100), &[0; 100], 10_000);
        q.insert(rcv_nxt, rcv_nxt.wrapping_add(400), &[0; 100], 10_000);
        let mut o = Options::default();
        q.sack_blocks(rcv_nxt, &mut o);
        assert_eq!(
            o.blocks(),
            [
                (rcv_nxt.wrapping_add(400), rcv_nxt.wrapping_add(500)),
                (rcv_nxt.wrapping_add(100), rcv_nxt.wrapping_add(200))
            ]
        );
    }

    #[test]
    fn with_sack_the_third_duplicate_starts_a_recovery_that_does_not_inflate() {
        let mut cc = NewReno::new(MSS, 10);
        assert_eq!(cc.on_dupack_sack(5000, 8 * M, 13_000, false), Action::None);
        assert_eq!(cc.on_dupack_sack(5000, 8 * M, 13_000, false), Action::None);
        assert_eq!(
            cc.on_dupack_sack(5000, 8 * M, 13_000, false),
            Action::Retransmit
        );
        assert!(cc.sack_recovery());
        // The window is the threshold, without the 3 segments of inflation.
        assert_eq!(cc.ssthresh(), 4 * M);
        assert_eq!(cc.cwnd(), 4 * M);
        // More duplicates change nothing.
        assert_eq!(cc.on_dupack_sack(5000, 8 * M, 13_000, true), Action::None);
        assert_eq!(cc.cwnd(), 4 * M);
    }

    #[test]
    fn a_scoreboard_loss_starts_the_recovery_on_the_first_duplicate() {
        let mut cc = NewReno::new(MSS, 10);
        assert_eq!(
            cc.on_dupack_sack(5000, 8 * M, 13_000, true),
            Action::Retransmit
        );
        assert!(cc.in_recovery());
    }

    #[test]
    fn a_partial_acknowledgment_in_a_sack_recovery_leaves_the_window_and_sends_nothing() {
        let mut cc = NewReno::new(MSS, 10);
        cc.on_dupack_sack(5000, 8 * M, 13_000, true);
        assert_eq!(cc.on_new_ack(7000, 2 * M), Action::None);
        assert_eq!(cc.cwnd(), 4 * M);
        assert!(cc.sack_recovery());
        // A full acknowledgment ends it at the threshold.
        assert_eq!(cc.on_new_ack(13_000, 6 * M), Action::None);
        assert!(!cc.in_recovery());
        assert!(!cc.sack_recovery());
        assert_eq!(cc.cwnd(), 4 * M);
    }

    #[test]
    fn a_timeout_ends_a_sack_recovery() {
        let mut cc = NewReno::new(MSS, 10);
        cc.on_dupack_sack(5000, 8 * M, 13_000, true);
        cc.on_timeout(8 * M, 13_000);
        assert!(!cc.sack_recovery());
        assert_eq!(cc.cwnd(), M);
    }

    #[test]
    fn the_recover_guard_also_holds_for_sack() {
        let mut cc = NewReno::new(MSS, 10);
        cc.on_timeout(8 * M, 13_000);
        // Duplicates below `recover` do not start a recovery again.
        for _ in 0..4 {
            assert_eq!(cc.on_dupack_sack(9000, 8 * M, 13_000, true), Action::None);
        }
    }
}
