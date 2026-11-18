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
        self.reduced(flight)
    }

    /// The retransmission timeout fired with `flight` bytes outstanding.
    /// `in_recovery` says that a recovery had reduced the window already,
    /// which then counts as the event. Returns the slow start threshold.
    pub fn on_rto(&mut self, cwnd: u32, flight: u32, in_recovery: bool) -> u32 {
        if !in_recovery {
            self.on_loss(cwnd, flight);
        }
        self.epoch_us = None;
        self.idle_us = None;
        self.carry = 0.0;
        self.reduced(flight)
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
}
