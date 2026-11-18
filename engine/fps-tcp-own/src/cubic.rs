// SPDX-License-Identifier: GPL-3.0-only
//! CUBIC congestion avoidance (RFC 9438), the state beside [`crate::recovery::NewReno`].
//!
//! The loss recovery of the stack (fast retransmit, fast recovery, the SACK
//! scoreboard, the retransmission timeout) stays the code of TRN-23 and
//! TRN-24. CUBIC replaces two rules of the congestion window: the reduction at
//! a congestion event (a factor of 0.7 instead of 0.5, with fast convergence)
//! and the growth in congestion avoidance (the cubic function of the time
//! since the last event instead of one segment per round trip). Slow start
//! stays the rule of RFC 5681.
//!
//! # Ported from smoltcp
//!
//! This module is a port of parts of smoltcp 0.14.0 (0BSD license),
//! `src/socket/tcp/congestion/cubic.rs`, to the byte counted window of this
//! stack:
//!
//! * the constants `BETA_CUBIC`, `C`, and `ALPHA_CUBIC`,
//! * `cube_root` (a bit manipulation start value and one Newton step, so the
//!   window function is the same on every platform),
//! * the window function `w_cubic`, the reno friendly window `w_est` with the
//!   switch of `alpha` at `cwnd_prior` (`Cubic::on_ack`),
//! * the target one round trip ahead, clamped to 1.5 times the window,
//! * the idle time absorption of RFC 9438 section 4.2 (`absorb_idle`),
//! * the fast convergence rule and the reduction of `on_loss` and `on_rto`.
//!
//! What differs from smoltcp: the epoch of a congestion avoidance stage starts
//! at the first acknowledgment after the event (and `K` comes from the window
//! at that moment, as RFC 9438 section 4.2 gives it), an acknowledgment counts
//! all the bytes it covers and not one segment (this stack acknowledges a
//! batch with one ACK), the fractional bytes of a growth step carry to the next
//! one, and the state has no `rwnd` (the send path limits the flight by the
//! window of the peer already).
//!
//! The whole state is 72 bytes and lives in the
//! [`crate::recovery::Recovery`] box, which a connection holds only while data
//! is in flight, so an idle socket pays nothing for it.

/// The multiplicative decrease of RFC 9438 section 4.6.
const BETA_CUBIC: f64 = 0.7;
/// The aggressiveness constant `C` of RFC 9438 section 4.2, in segments per
/// second cubed.
const C: f64 = 0.4;
/// `3 (1 - beta) / (1 + beta)` of RFC 9438 equation 4, the additive increase
/// of the reno friendly window that keeps pace with a NewReno flow.
const ALPHA_CUBIC: f64 = 3.0 * (1.0 - BETA_CUBIC) / (1.0 + BETA_CUBIC);

/// What an acknowledgment tells the window besides the bytes it covers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AckContext {
    /// The time of the acknowledgment in microseconds.
    pub now_us: i64,
    /// The smoothed round trip time in microseconds, once there is one.
    pub srtt_us: Option<i64>,
    /// The data still in flight after the acknowledgment.
    pub flight: u32,
    /// The round trip that this acknowledgment measured, in microseconds, or
    /// 0 when it gave none (it acknowledged a segment that was sent again, or
    /// the connection does not log send times).
    pub rtt_us: u32,
    /// The oldest unacknowledged sequence number after the acknowledgment.
    pub snd_una: u32,
    /// The next sequence number to send.
    pub snd_nxt: u32,
}

/// HyStart (Linux `tcp_cubic.c`, Ha and Rhee): leaves slow start before the
/// window overflows the queue of the path, on one of two signs. The ACK train:
/// the acknowledgments of one round arrive back to back, and the train is
/// longer than half the smallest round trip, so the window already fills the
/// pipe. The delay: the smallest round trip of the round, from eight samples,
/// rose above the smallest ever seen by a few milliseconds, so a queue builds.
/// Either one sets the slow start threshold to the window, which ends slow
/// start and starts the cubic function from there.
///
/// It runs only above 16 segments (`hystart_low_window`), as the kernel does.
/// The state is 24 bytes in the [`crate::recovery::Recovery`] box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HyStart {
    /// The smallest round trip seen, in microseconds (0: none yet).
    delay_min: u32,
    /// The start of the round, a truncated clock reading.
    round_start: u32,
    /// The time of the last acknowledgment of the train.
    last_ack: u32,
    /// The sequence number that ends the round: `snd_nxt` at its start.
    end_seq: u32,
    /// The smallest round trip of the round so far (`u32::MAX`: none).
    curr_rtt: u32,
    samples: u8,
    started: bool,
    found: bool,
}

/// The window below which HyStart does not decide, in segments.
const HYSTART_LOW_WINDOW: u32 = 16;
/// The acknowledgments of a train are at most this far apart, in microseconds.
const HYSTART_ACK_DELTA_US: i32 = 2_000;
/// The samples of a round before its delay counts.
const HYSTART_MIN_SAMPLES: u8 = 8;
/// The bounds of the delay threshold, in microseconds.
const HYSTART_DELAY_MIN_US: u32 = 4_000;
const HYSTART_DELAY_MAX_US: u32 = 16_000;

impl HyStart {
    /// A state that waits for its first round.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether HyStart has ended the slow start.
    pub fn found(&self) -> bool {
        self.found
    }

    /// The smallest round trip seen, in microseconds.
    pub fn delay_min_us(&self) -> Option<u32> {
        (self.delay_min != 0).then_some(self.delay_min)
    }

    /// Starts over, as the kernel does when a timeout puts the connection
    /// back in slow start.
    pub fn reset(&mut self) {
        self.found = false;
        self.started = false;
        self.samples = 0;
    }

    fn start_round(&mut self, now: u32, snd_nxt: u32) {
        self.started = true;
        self.round_start = now;
        self.last_ack = now;
        self.end_seq = snd_nxt;
        self.curr_rtt = u32::MAX;
        self.samples = 0;
    }

    /// An acknowledgment arrived in slow start with the window `cwnd`.
    /// Returns true when slow start is over.
    pub fn on_ack(&mut self, cwnd: u32, mss: u32, ctx: &AckContext) -> bool {
        if ctx.rtt_us != 0 && (self.delay_min == 0 || ctx.rtt_us < self.delay_min) {
            self.delay_min = ctx.rtt_us;
        }
        if self.found || cwnd < HYSTART_LOW_WINDOW * mss.max(1) {
            return false;
        }
        let now = crate::rack::t32(ctx.now_us);
        if !self.started || crate::host::seq_lt(self.end_seq, ctx.snd_una) {
            self.start_round(now, ctx.snd_nxt);
        }
        let delay_min = self.delay_min;
        if delay_min == 0 {
            return false;
        }
        // The ACK train: acknowledgments that follow each other within two
        // milliseconds, in a train longer than half the smallest round trip.
        if crate::rack::since(now, self.last_ack) <= HYSTART_ACK_DELTA_US {
            self.last_ack = now;
            if crate::rack::since(now, self.round_start) > (delay_min >> 1) as i32 {
                self.found = true;
                return true;
            }
        }
        // The delay: the smallest round trip of this round, once there are
        // enough samples, against the smallest ever seen.
        if ctx.rtt_us != 0 {
            self.curr_rtt = self.curr_rtt.min(ctx.rtt_us);
            if self.samples < HYSTART_MIN_SAMPLES {
                self.samples += 1;
            } else {
                let thresh = (delay_min >> 3).clamp(HYSTART_DELAY_MIN_US, HYSTART_DELAY_MAX_US);
                if self.curr_rtt > delay_min.saturating_add(thresh) {
                    self.found = true;
                    return true;
                }
            }
        }
        false
    }
}

/// The CUBIC state of one connection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cubic {
    /// The window before the last congestion event (`W_max`), in bytes, after
    /// fast convergence.
    w_max: f64,
    /// The time in seconds that the cubic function takes to climb back to
    /// `w_max` (`K`).
    k: f64,
    /// The reno friendly window `W_est` of RFC 9438 section 4.3, in bytes.
    w_est: f64,
    /// The window at the last congestion event (`cwnd_prior`), which gates
    /// `alpha`.
    cwnd_prior: f64,
    /// The start of the congestion avoidance stage, in microseconds. `None`
    /// from an event until the first acknowledgment after it.
    epoch_us: Option<i64>,
    /// When the flight last fell to zero (RFC 9438 section 4.2).
    idle_us: Option<i64>,
    /// The fraction of a byte that the last growth step did not add.
    carry: f64,
    mss: f64,
}

impl Cubic {
    /// A state for segments of `mss` bytes.
    pub fn new(mss: u32) -> Self {
        Self {
            w_max: 0.0,
            k: 0.0,
            w_est: 0.0,
            cwnd_prior: 0.0,
            epoch_us: None,
            idle_us: None,
            carry: 0.0,
            mss: f64::from(mss.max(1)),
        }
    }

    /// `W_max` in bytes.
    pub fn w_max(&self) -> f64 {
        self.w_max
    }

    /// `K` in seconds.
    pub fn k(&self) -> f64 {
        self.k
    }

    /// The reno friendly window in bytes.
    pub fn w_est(&self) -> f64 {
        self.w_est
    }

    /// `W_cubic(t)` of RFC 9438 equation 1 in bytes, for `t` seconds after the
    /// start of the stage.
    pub fn w_cubic(&self, t: f64) -> f64 {
        C * self.mss * (t - self.k).powi(3) + self.w_max
    }

    /// A congestion event of fast retransmit with the window `cwnd` and
    /// `flight` bytes outstanding. Returns the new slow start threshold. The
    /// fast convergence rule (RFC 9438 section 4.7) releases bandwidth to a
    /// new flow when the window fell short of the last maximum.
    pub fn on_loss(&mut self, cwnd: u32, flight: u32) -> u32 {
        let basis = flight.min(cwnd);
        let cwnd = f64::from(cwnd);
        self.cwnd_prior = cwnd;
        self.w_max = if cwnd < self.w_max {
            cwnd * (1.0 + BETA_CUBIC) / 2.0
        } else {
            cwnd
        };
        self.epoch_us = None;
        self.idle_us = None;
        self.carry = 0.0;
        self.reduced(basis)
    }

    /// The retransmission timeout fired with `flight` bytes outstanding.
    /// `in_recovery` says that a recovery had reduced the window already,
    /// which then counts as the event. Returns the slow start threshold.
    pub fn on_rto(&mut self, cwnd: u32, flight: u32, in_recovery: bool) -> u32 {
        let reduced = if in_recovery {
            self.reduced(flight)
        } else {
            self.on_loss(cwnd, flight)
        };
        self.epoch_us = None;
        self.idle_us = None;
        self.carry = 0.0;
        reduced
    }

    /// `max(beta * flight, 2 SMSS)`.
    fn reduced(&self, flight: u32) -> u32 {
        let reduced = (f64::from(flight) * BETA_CUBIC) as u32;
        reduced.max(2 * self.mss as u32)
    }

    /// A recovery ended with the window `cwnd`: the next acknowledgment in
    /// congestion avoidance starts a new stage from there.
    pub fn on_recovery_exit(&mut self, cwnd: u32) {
        self.epoch_us = None;
        self.w_est = f64::from(cwnd);
        self.carry = 0.0;
    }

    /// Moves the epoch by the time the connection was idle (RFC 9438 section
    /// 4.2), so that a pause does not count as growth time.
    fn absorb_idle(&mut self, now_us: i64) {
        if let (Some(idle), Some(epoch)) = (self.idle_us, self.epoch_us) {
            if now_us >= idle {
                self.epoch_us = Some(epoch + (now_us - idle));
            }
        }
        self.idle_us = None;
    }

    /// An acknowledgment of `acked` new bytes arrived in congestion
    /// avoidance with the window `cwnd`. Returns the new window.
    pub fn on_ack(&mut self, cwnd: u32, acked: u32, ctx: &AckContext) -> u32 {
        self.absorb_idle(ctx.now_us);
        if ctx.flight == 0 {
            self.idle_us = Some(ctx.now_us);
        }
        if acked == 0 {
            return cwnd;
        }
        let cwnd_f = f64::from(cwnd.max(1));
        let epoch = match self.epoch_us {
            Some(epoch) => epoch,
            None => {
                // The start of a stage (RFC 9438 section 4.2): the curve
                // climbs from the window to W_max, or starts flat at the
                // window when it is at or above it.
                if cwnd_f < self.w_max {
                    self.k = cube_root((self.w_max - cwnd_f) / (C * self.mss));
                } else {
                    self.w_max = cwnd_f;
                    self.k = 0.0;
                }
                self.w_est = cwnd_f;
                self.epoch_us = Some(ctx.now_us);
                ctx.now_us
            }
        };
        let t_us = ctx.now_us - epoch;
        if t_us < 0 {
            return cwnd;
        }
        let acked_f = f64::from(acked).min(cwnd_f);
        let t = t_us as f64 / 1e6;
        let w_cubic = self.w_cubic(t);
        // The reno friendly window (RFC 9438 equations 4 and 5): `alpha`
        // keeps pace with NewReno after a loss, and is 1 once the estimate
        // has caught up with the window before the event.
        let alpha = if self.w_est >= self.cwnd_prior {
            1.0
        } else {
            ALPHA_CUBIC
        };
        self.w_est += alpha * self.mss * acked_f / cwnd_f;
        if w_cubic < self.w_est {
            // The TCP friendly region: the window follows the estimate.
            let grown = self.w_est.max(cwnd_f);
            return Self::to_window(grown);
        }
        // The concave and convex regions: the target is the cubic function
        // one round trip ahead, at most 1.5 times the window.
        let rtt = ctx.srtt_us.unwrap_or(0).max(1000) as f64 / 1e6;
        let target = self.w_cubic(t + rtt).min(1.5 * cwnd_f);
        let step = if target > cwnd_f {
            (target - cwnd_f) * acked_f / cwnd_f
        } else {
            // Below the curve the window still grows a little (RFC 9438
            // section 4.3, one segment per 100 windows).
            self.mss * acked_f / (100.0 * cwnd_f)
        } + self.carry;
        let whole = step.floor();
        self.carry = step - whole;
        Self::to_window(cwnd_f + whole)
    }

    fn to_window(bytes: f64) -> u32 {
        if bytes >= f64::from(u32::MAX) {
            u32::MAX
        } else {
            bytes as u32
        }
    }
}

/// The cube root without `std`'s `cbrt`, as smoltcp 0.14.0 has it in
/// `congestion/cubic.rs`: a start value from the exponent and mantissa bits
/// and one Newton step. The result is within 0.2 percent, which the window
/// function does not feel, and it is the same on every platform.
fn cube_root(a: f64) -> f64 {
    if !(a >= f64::MIN_POSITIVE && a.is_finite()) {
        return 0.0;
    }
    const POW2_REM_OVER_3: [f64; 3] = [1.0, 1.259_921_049_894_873_2, 1.587_401_051_968_199_5];
    let bits = a.to_bits();
    let m = f64::from_bits((bits & 0x000F_FFFF_FFFF_FFFF) | 0x3FF0_0000_0000_0000);
    let cbrt_m = m.mul_add(0.2599, 0.7401);
    let e = (((bits >> 52) & 0x7FF) as i64) - 1023;
    let q = e.div_euclid(3);
    let r = e.rem_euclid(3) as usize;
    let pow2q = f64::from_bits(((q + 1023) as u64) << 52);
    let x = cbrt_m * pow2q * POW2_REM_OVER_3[r];
    (2.0 * x + a / (x * x)) / 3.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const MSS: u32 = 1000;

    fn ctx(now_us: i64) -> AckContext {
        AckContext {
            now_us,
            srtt_us: Some(100_000),
            flight: 10 * MSS,
            ..AckContext::default()
        }
    }

    #[test]
    fn the_cube_root_is_close_for_small_and_large_values() {
        for a in [1e-3, 0.5, 1.0, 8.0, 27.0, 1000.0, 3.3e7, 1e12] {
            let r = cube_root(a);
            assert!((r - a.cbrt()).abs() / a.cbrt() < 2e-3, "{a}: {r}");
        }
        assert_eq!(cube_root(0.0), 0.0);
        assert_eq!(cube_root(f64::NAN), 0.0);
        assert_eq!(cube_root(-8.0), 0.0);
    }

    #[test]
    fn a_loss_sets_wmax_and_reduces_by_beta() {
        let mut c = Cubic::new(MSS);
        let ssthresh = c.on_loss(100 * MSS, 100 * MSS);
        assert_eq!(c.w_max(), f64::from(100 * MSS));
        assert_eq!(ssthresh, 70 * MSS);
    }

    #[test]
    fn the_reduction_has_a_floor_of_two_segments() {
        let mut c = Cubic::new(MSS);
        assert_eq!(c.on_loss(2 * MSS, 2 * MSS), 2 * MSS);
    }

    #[test]
    fn fast_convergence_lowers_wmax_when_the_window_fell_short() {
        let mut c = Cubic::new(MSS);
        c.on_loss(100 * MSS, 100 * MSS);
        // The next event comes at 80 segments, below the last maximum of 100.
        c.on_loss(80 * MSS, 80 * MSS);
        let expected = f64::from(80 * MSS) * (1.0 + 0.7) / 2.0;
        assert!((c.w_max() - expected).abs() < 1e-6);
        // An event above the last maximum takes the window as it is.
        c.on_loss(200 * MSS, 200 * MSS);
        assert_eq!(c.w_max(), f64::from(200 * MSS));
    }

    #[test]
    fn k_is_the_time_to_climb_from_the_window_back_to_wmax() {
        let mut c = Cubic::new(MSS);
        let ssthresh = c.on_loss(100 * MSS, 100 * MSS);
        // The first acknowledgment of the stage computes K.
        let _ = c.on_ack(ssthresh, MSS, &ctx(1_000_000));
        let expected = (30.0f64 / C).cbrt();
        assert!((c.k() - expected).abs() / expected < 2e-3, "{}", c.k());
        // W_cubic(K) is W_max, and W_cubic(0) is the window of the start.
        assert!((c.w_cubic(c.k()) - c.w_max()).abs() < 1.0);
        assert!((c.w_cubic(0.0) - f64::from(ssthresh)).abs() / f64::from(ssthresh) < 0.01);
    }

    #[test]
    fn without_an_earlier_loss_the_curve_starts_flat_at_the_window() {
        let mut c = Cubic::new(MSS);
        let _ = c.on_ack(50 * MSS, MSS, &ctx(0));
        assert_eq!(c.k(), 0.0);
        assert_eq!(c.w_max(), f64::from(50 * MSS));
    }

    /// Feeds one acknowledgment per round trip of 100 ms that covers a
    /// window, and returns the window after `secs` seconds.
    fn run(c: &mut Cubic, mut cwnd: u32, from_us: i64, secs: f64) -> u32 {
        let end = from_us + (secs * 1e6) as i64;
        let mut now = from_us;
        while now < end {
            cwnd = c.on_ack(cwnd, cwnd, &ctx(now));
            now += 100_000;
        }
        cwnd
    }

    #[test]
    fn the_window_climbs_concave_to_wmax_and_then_convex_past_it() {
        let mut c = Cubic::new(MSS);
        // A large window, so that the cubic curve and not the reno friendly
        // estimate sets the window.
        let before = 2000 * MSS;
        let mut cwnd = c.on_loss(before, before);
        // Early on the window grows fast, near W_max slowly (concave).
        let w0 = cwnd;
        let w1 = run(&mut c, w0, 0, 1.0);
        let w2 = run(&mut c, w1, 1_000_000, 1.0);
        assert!(w1 > w0 && w2 > w1);
        // Past K the window passes W_max and keeps growing (convex).
        let k = c.k();
        cwnd = run(&mut c, w2, 2_000_000, k + 3.0);
        assert!(f64::from(cwnd) > c.w_max(), "{cwnd} vs {}", c.w_max());
    }

    #[test]
    fn the_growth_per_round_trip_is_limited_to_one_and_a_half_windows() {
        let mut c = Cubic::new(MSS);
        // A start far below W_max would call for a huge target.
        c.on_loss(100_000 * MSS, 100_000 * MSS);
        let cwnd = 10 * MSS;
        let after = c.on_ack(cwnd, cwnd, &ctx(0));
        // One window acknowledged: at most +0.5 window.
        assert!(after <= cwnd + cwnd / 2 + 1, "{after}");
        assert!(after > cwnd);
    }

    #[test]
    fn the_tcp_friendly_region_follows_the_reno_estimate_at_a_small_window() {
        let mut c = Cubic::new(MSS);
        // 10 segments, a short round trip: the cubic curve is below the
        // estimate and the window grows about 0.53 segments per round trip
        // (alpha_cubic) while below cwnd_prior.
        let mut cwnd = c.on_loss(14 * MSS, 14 * MSS);
        assert_eq!(cwnd, 9 * MSS + 800);
        let start = cwnd;
        for round in 0..20 {
            // One window of acknowledgments in 10 pieces.
            for _ in 0..10 {
                cwnd = c.on_ack(cwnd, cwnd / 10, &ctx(i64::from(round) * 100_000));
            }
        }
        let grown = f64::from(cwnd - start) / f64::from(MSS);
        // 20 rounds of about 0.53 segments (more, once w_est passes
        // cwnd_prior and alpha is 1).
        assert!(grown > 8.0 && grown < 20.0, "{grown}");
        assert!(f64::from(cwnd) >= c.w_est() - f64::from(MSS));
    }

    #[test]
    fn alpha_is_one_once_the_estimate_passes_the_window_before_the_event() {
        let mut c = Cubic::new(MSS);
        c.on_loss(20 * MSS, 20 * MSS);
        c.on_recovery_exit(14 * MSS);
        let _ = c.on_ack(14 * MSS, MSS, &ctx(0));
        let low = c.w_est();
        // Below cwnd_prior: alpha 0.529 of a segment per window.
        let step = low - f64::from(14 * MSS);
        assert!(step > 0.0 && step < f64::from(MSS) * 0.6 / 14.0 * 1.0 + 1.0);
        // Force w_est above cwnd_prior: the next step uses alpha = 1.
        c.w_est = f64::from(25 * MSS);
        let before = c.w_est();
        let _ = c.on_ack(14 * MSS, MSS, &ctx(1000));
        let gain = c.w_est() - before;
        assert!((gain - f64::from(MSS) * f64::from(MSS) / f64::from(14 * MSS)).abs() < 1.0);
    }

    #[test]
    fn idle_time_does_not_count_as_growth_time() {
        let mut c = Cubic::new(MSS);
        let cwnd = c.on_loss(1000 * MSS, 1000 * MSS);
        let _ = c.on_ack(cwnd, MSS, &ctx(0));
        // The flight drops to zero at 1 s, and data flows again at 11 s.
        let idle = AckContext {
            now_us: 1_000_000,
            srtt_us: Some(100_000),
            flight: 0,
            ..AckContext::default()
        };
        let _ = c.on_ack(cwnd, MSS, &idle);
        let epoch_before = c.epoch_us.unwrap();
        let _ = c.on_ack(cwnd, MSS, &ctx(11_000_000));
        assert_eq!(c.epoch_us.unwrap(), epoch_before + 10_000_000);
    }

    #[test]
    fn a_timeout_after_a_recovery_keeps_wmax_and_one_before_it_sets_it() {
        let mut c = Cubic::new(MSS);
        c.on_loss(100 * MSS, 100 * MSS);
        let w = c.w_max();
        // The timeout ends a recovery: same event, W_max stays.
        let ssthresh = c.on_rto(70 * MSS, 60 * MSS, true);
        assert_eq!(c.w_max(), w);
        assert_eq!(ssthresh, 42 * MSS);
        // A timeout without a recovery before is an event of its own.
        let mut d = Cubic::new(MSS);
        let ssthresh = d.on_rto(50 * MSS, 50 * MSS, false);
        assert_eq!(d.w_max(), f64::from(50 * MSS));
        assert_eq!(ssthresh, 35 * MSS);
    }

    #[test]
    fn a_recovery_exit_starts_a_new_stage_from_the_window() {
        let mut c = Cubic::new(MSS);
        let cwnd = c.on_loss(1000 * MSS, 1000 * MSS);
        let _ = c.on_ack(cwnd, MSS, &ctx(0));
        assert!(c.epoch_us.is_some());
        c.on_recovery_exit(cwnd);
        assert!(c.epoch_us.is_none());
        assert_eq!(c.w_est(), f64::from(cwnd));
    }

    #[test]
    fn the_reduction_follows_the_window_when_the_flight_is_larger() {
        // A SACK recovery sends new data beyond the window, and a timeout
        // leaves the whole old flight outstanding. Neither raises the
        // threshold above beta times the window (RFC 9438 section 4.6).
        let mut c = Cubic::new(MSS);
        assert_eq!(c.on_loss(40 * MSS, 100 * MSS), 28 * MSS);
        // An application limited flow has less in flight than the window.
        let mut c = Cubic::new(MSS);
        assert_eq!(c.on_loss(40 * MSS, 10 * MSS), 7 * MSS);
        let mut c = Cubic::new(MSS);
        assert_eq!(c.on_rto(1, 100 * MSS, false), 2 * MSS);
    }

    // ---- HyStart (TRN-34) ----

    fn hs_ctx(now_us: i64, rtt_us: u32, snd_una: u32, snd_nxt: u32) -> AckContext {
        AckContext {
            now_us,
            srtt_us: Some(10_000),
            flight: 0,
            rtt_us,
            snd_una,
            snd_nxt,
        }
    }

    #[test]
    fn hystart_does_not_decide_below_sixteen_segments() {
        let mut h = HyStart::new();
        for i in 0..40 {
            let ctx = hs_ctx(i * 100, 1000, 1000 * i as u32, 100_000);
            assert!(!h.on_ack(15 * MSS, MSS, &ctx));
        }
        assert!(!h.found());
        // The smallest round trip is kept all the same.
        assert_eq!(h.delay_min_us(), Some(1000));
    }

    #[test]
    fn an_ack_train_longer_than_half_the_smallest_round_trip_ends_slow_start() {
        let mut h = HyStart::new();
        // The first acknowledgment of a round starts it and gives the minimum.
        assert!(!h.on_ack(20 * MSS, MSS, &hs_ctx(0, 1000, 0, 50_000)));
        // The next one 400 us later is within the train, and the train is not
        // yet longer than 500 us.
        assert!(!h.on_ack(20 * MSS, MSS, &hs_ctx(400, 1000, 1000, 50_000)));
        assert!(h.on_ack(20 * MSS, MSS, &hs_ctx(900, 1000, 2000, 50_000)));
        assert!(h.found());
        // Once found it stays quiet.
        assert!(!h.on_ack(20 * MSS, MSS, &hs_ctx(5000, 1000, 3000, 50_000)));
    }

    #[test]
    fn acknowledgments_far_apart_are_not_a_train() {
        let mut h = HyStart::new();
        assert!(!h.on_ack(20 * MSS, MSS, &hs_ctx(0, 10_000, 0, 500_000)));
        // Three milliseconds apart (more than the two of a train), each one
        // with the same round trip: neither sign.
        for i in 1..12 {
            let ctx = hs_ctx(i * 3_000, 10_000, 1000 * i as u32, 500_000);
            assert!(!h.on_ack(20 * MSS, MSS, &ctx), "{i}");
        }
        assert!(!h.found());
    }

    #[test]
    fn a_round_of_larger_round_trips_after_the_minimum_ends_slow_start() {
        let mut h = HyStart::new();
        let mut t = 0;
        // Round one: the minimum of 10 ms.
        h.on_ack(20 * MSS, MSS, &hs_ctx(t, 10_000, 0, 10_000));
        // Round two begins when `snd_una` passes 10,000 and every sample is
        // 20 ms: the ninth sample finds the rise.
        let mut found_at = None;
        for i in 0..12u32 {
            t += 3_000;
            let ctx = hs_ctx(t, 20_000, 10_001 + i * 100, 90_000);
            if h.on_ack(20 * MSS, MSS, &ctx) {
                found_at = Some(i);
                break;
            }
        }
        assert_eq!(found_at, Some(8), "{h:?}");
    }

    #[test]
    fn a_reset_starts_the_search_again() {
        let mut h = HyStart::new();
        h.on_ack(20 * MSS, MSS, &hs_ctx(0, 1000, 0, 50_000));
        h.on_ack(20 * MSS, MSS, &hs_ctx(900, 1000, 1000, 50_000));
        assert!(h.found());
        h.reset();
        assert!(!h.found());
        // The minimum survives, as in the kernel.
        assert_eq!(h.delay_min_us(), Some(1000));
    }
}
