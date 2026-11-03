// SPDX-License-Identifier: GPL-3.0-only
//! Deterministic mapping from sessions to hosts.
//!
//! A [`HostSelector`] picks one host of a worker's range for each session.
//! The pick depends only on the run seed, the pool name, the worker id, the
//! session id, and the selection mode, so a replay with the same seed gives
//! every session the same host. The selector keeps no state between calls and
//! allocates nothing per session.

use rand::Rng;
use rand_pcg::Pcg64Mcg;

use crate::slice::HostRange;

/// Errors that building a selector reports.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SelectError {
    /// The range holds no hosts.
    #[error("host pool {pool} worker {worker}: no hosts to select from")]
    EmptyRange {
        /// The pool name.
        pool: String,
        /// The worker id.
        worker: u32,
    },
    /// The Zipf exponent is not a positive finite number.
    #[error("host pool {pool}: zipf exponent {exponent} must be positive and finite")]
    ZipfExponent {
        /// The pool name.
        pool: String,
        /// The rejected exponent.
        exponent: f64,
    },
}

/// How a selector spreads sessions over the hosts of a range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Selection {
    /// Session `s` uses host `offset + s mod count`, so consecutive session
    /// ids visit every host in turn.
    RoundRobin,
    /// Every host is equally likely, drawn from the session generator.
    Uniform,
    /// Host of rank `k`, counting from 1 at the start of the range, has
    /// probability proportional to `1 / k^exponent`.
    Zipf {
        /// The exponent, which must be positive. Larger values concentrate
        /// sessions on fewer hosts.
        exponent: f64,
    },
}

/// Picks the host of each session within one worker's range of one pool.
#[derive(Debug, Clone)]
pub struct HostSelector {
    range: HostRange,
    key: u64,
    mode: Mode,
}

#[derive(Debug, Clone)]
enum Mode {
    RoundRobin,
    Uniform,
    Zipf(Zipf),
}

/// Domain separator, so the host stream never equals another stream that the
/// worker derives from the same seed and session id.
const HOSTS_DOMAIN: u64 = 0x686f_7374_706f_6f6c;

/// The SplitMix64 finalizer.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// FNV-1a over the pool name, which is stable across platforms and releases.
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

impl HostSelector {
    /// Returns a selector for `range` of pool `pool` on worker `worker`.
    pub fn new(
        pool: &str,
        range: HostRange,
        selection: Selection,
        run_seed: u64,
        worker: u32,
    ) -> Result<Self, SelectError> {
        if range.is_empty() {
            return Err(SelectError::EmptyRange {
                pool: pool.to_owned(),
                worker,
            });
        }
        let mode = match selection {
            Selection::RoundRobin => Mode::RoundRobin,
            Selection::Uniform => Mode::Uniform,
            Selection::Zipf { exponent } => {
                if !(exponent.is_finite() && exponent > 0.0) {
                    return Err(SelectError::ZipfExponent {
                        pool: pool.to_owned(),
                        exponent,
                    });
                }
                Mode::Zipf(Zipf::new(range.count, exponent))
            }
        };
        let key = mix(run_seed ^ mix(fnv1a(pool) ^ mix(HOSTS_DOMAIN ^ u64::from(worker))));
        Ok(Self { range, key, mode })
    }

    /// The range that the selector picks from.
    pub fn range(&self) -> HostRange {
        self.range
    }

    /// Returns the pool index of the host for `session_id`.
    pub fn select(&self, session_id: u64) -> u32 {
        let count = self.range.count;
        let rank = match &self.mode {
            // The remainder is below count, which is a u32.
            Mode::RoundRobin => u32::try_from(session_id % u64::from(count)).unwrap_or(0),
            Mode::Uniform => self.rng(session_id).gen_range(0..count),
            Mode::Zipf(z) => z.sample(&mut self.rng(session_id)) - 1,
        };
        self.range.offset + rank
    }

    fn rng(&self, session_id: u64) -> Pcg64Mcg {
        // `Pcg64Mcg::new` sets the two lowest state bits, so the session id
        // must not sit in the low half, or ids 4k to 4k+3 would share one
        // stream. `mix` is a bijection, so distinct ids give distinct high
        // halves. A constant low half biases the first outputs of the
        // generator, so the low half is mixed as well.
        let hi = mix(self.key ^ session_id);
        let lo = mix(hi ^ HOSTS_DOMAIN);
        Pcg64Mcg::new((u128::from(hi) << 64) | u128::from(lo))
    }
}

/// Zipf sampler over ranks `1..=n` by rejection inversion, after W. Hörmann
/// and G. Derflinger, "Rejection-inversion to generate variates from monotone
/// discrete distributions", 1996. It needs constant memory for any `n`.
#[derive(Debug, Clone)]
struct Zipf {
    n: f64,
    s: f64,
    h_x1: f64,
    h_n: f64,
    threshold: f64,
}

impl Zipf {
    fn new(n: u32, s: f64) -> Self {
        let mut z = Self {
            n: f64::from(n),
            s,
            h_x1: 0.0,
            h_n: 0.0,
            threshold: 0.0,
        };
        z.h_x1 = z.h_integral(1.5) - 1.0;
        z.h_n = z.h_integral(z.n + 0.5);
        z.threshold = 2.0 - z.h_integral_inverse(z.h_integral(2.5) - z.h(2.0));
        z
    }

    fn sample<R: Rng>(&self, rng: &mut R) -> u32 {
        loop {
            let u = self.h_n + rng.gen::<f64>() * (self.h_x1 - self.h_n);
            let x = self.h_integral_inverse(u);
            let k = (x + 0.5).floor().clamp(1.0, self.n);
            if k - x <= self.threshold || u >= self.h_integral(k + 0.5) - self.h(k) {
                // k is a whole number in 1..=n, and n is a u32, so the cast
                // is exact.
                return k as u32;
            }
        }
    }

    fn h(&self, x: f64) -> f64 {
        (-self.s * x.ln()).exp()
    }

    fn h_integral(&self, x: f64) -> f64 {
        let log_x = x.ln();
        helper2((1.0 - self.s) * log_x) * log_x
    }

    fn h_integral_inverse(&self, x: f64) -> f64 {
        let t = (x * (1.0 - self.s)).max(-1.0);
        (helper1(t) * x).exp()
    }
}

/// `ln(1 + x) / x`, continued to 1 at 0.
fn helper1(x: f64) -> f64 {
    if x.abs() > 1e-8 {
        x.ln_1p() / x
    } else {
        1.0 - x * (0.5 - x * (1.0 / 3.0 - 0.25 * x))
    }
}

/// `(e^x - 1) / x`, continued to 1 at 0.
fn helper2(x: f64) -> f64 {
    if x.abs() > 1e-8 {
        x.exp_m1() / x
    } else {
        1.0 + x * 0.5 * (1.0 + x * (1.0 / 3.0) * (1.0 + 0.25 * x))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sel(range: HostRange, s: Selection) -> HostSelector {
        HostSelector::new("clients", range, s, 7, 0).unwrap()
    }

    #[test]
    fn round_robin_visits_every_host_in_turn() {
        let s = sel(HostRange::new(127, 4), Selection::RoundRobin);
        let got: Vec<u32> = (0..9).map(|i| s.select(i)).collect();
        assert_eq!(got, vec![127, 128, 129, 130, 127, 128, 129, 130, 127]);
    }

    #[test]
    fn uniform_is_roughly_flat() {
        let s = sel(HostRange::new(10, 8), Selection::Uniform);
        let mut hist = [0u32; 8];
        for i in 0..80_000 {
            hist[(s.select(i) - 10) as usize] += 1;
        }
        for h in hist {
            assert!((9_000..11_000).contains(&h), "{hist:?}");
        }
    }

    #[test]
    fn zipf_matches_its_distribution() {
        let n = 10u32;
        let exponent = 1.0;
        let s = sel(HostRange::new(0, n), Selection::Zipf { exponent });
        let draws = 200_000u32;
        let mut hist = vec![0u32; n as usize];
        for i in 0..u64::from(draws) {
            hist[s.select(i) as usize] += 1;
        }
        let norm: f64 = (1..=n).map(|k| 1.0 / f64::from(k).powf(exponent)).sum();
        for (k, h) in hist.iter().enumerate() {
            let want = 1.0 / f64::from(k as u32 + 1).powf(exponent) / norm;
            let got = f64::from(*h) / f64::from(draws);
            assert!(
                (got - want).abs() < 0.005,
                "rank {} got {got} want {want}",
                k + 1
            );
        }
    }

    #[test]
    fn zipf_handles_large_pools_and_steep_exponents() {
        for exponent in [0.5, 1.0, 1.2, 3.0] {
            let s = sel(HostRange::new(5, 16_000_000), Selection::Zipf { exponent });
            for i in 0..2_000 {
                assert!(s.range().contains(s.select(i)));
            }
        }
        let one = sel(HostRange::new(9, 1), Selection::Zipf { exponent: 1.0 });
        assert!((0..100).all(|i| one.select(i) == 9));
    }

    #[test]
    fn selection_depends_on_seed_pool_and_worker() {
        let r = HostRange::new(0, 1_000_000);
        let picks = |pool: &str, seed, worker| {
            let s = HostSelector::new(pool, r, Selection::Uniform, seed, worker).unwrap();
            (0..64).map(|i| s.select(i)).collect::<Vec<_>>()
        };
        let base = picks("clients", 7, 0);
        assert_eq!(base, picks("clients", 7, 0));
        assert_ne!(base, picks("clients", 8, 0));
        assert_ne!(base, picks("servers", 7, 0));
        assert_ne!(base, picks("clients", 7, 1));
    }

    #[test]
    fn uniform_picks_are_pinned_for_seed_7() {
        // A change to the derivation changes every replay, so this test pins
        // the first picks. Update it only on purpose.
        let s = sel(HostRange::new(0, 254), Selection::Uniform);
        let got: Vec<u32> = (0..6).map(|i| s.select(i)).collect();
        assert_eq!(got, PINNED_UNIFORM.to_vec());
    }

    #[test]
    fn neighbouring_session_ids_get_distinct_streams() {
        let s = sel(HostRange::new(0, u32::MAX), Selection::Uniform);
        let picks: std::collections::HashSet<u32> = (0..4096).map(|i| s.select(i)).collect();
        assert!(picks.len() > 4090, "{} distinct picks", picks.len());
    }

    const PINNED_UNIFORM: [u32; 6] = [40, 189, 171, 239, 91, 215];

    #[test]
    fn rejects_empty_ranges_and_bad_exponents() {
        assert!(matches!(
            HostSelector::new("p", HostRange::new(3, 0), Selection::Uniform, 1, 2),
            Err(SelectError::EmptyRange { worker: 2, .. })
        ));
        for exponent in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                HostSelector::new(
                    "p",
                    HostRange::new(0, 3),
                    Selection::Zipf { exponent },
                    1,
                    0
                ),
                Err(SelectError::ZipfExponent { .. })
            ));
        }
    }

    fn selection() -> impl Strategy<Value = Selection> {
        prop_oneof![
            Just(Selection::RoundRobin),
            Just(Selection::Uniform),
            (0.01f64..5.0).prop_map(|exponent| Selection::Zipf { exponent }),
        ]
    }

    proptest! {
        #[test]
        fn select_stays_in_range_and_is_deterministic(
            offset in 0u32..1_000_000,
            count in 1u32..10_000_000,
            mode in selection(),
            seed: u64,
            worker in 0u32..256,
            session: u64,
        ) {
            let r = HostRange::new(offset, count);
            let a = HostSelector::new("p", r, mode, seed, worker).unwrap();
            let b = HostSelector::new("p", r, mode, seed, worker).unwrap();
            let host = a.select(session);
            prop_assert!(r.contains(host));
            prop_assert_eq!(host, b.select(session));
        }
    }
}
