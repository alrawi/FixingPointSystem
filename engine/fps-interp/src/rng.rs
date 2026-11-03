// SPDX-License-Identifier: GPL-3.0-only
//! Per session random number generators.
//!
//! Every random draw of a session, from templates, branches, and delays,
//! comes from one generator that [`session_rng`] derives from the run seed,
//! the session id, and the side. A replay with the same seed therefore
//! reproduces every session, and the order in which a worker runs sessions
//! does not change their bytes.

use std::fmt;

use rand_pcg::Pcg64Mcg;

/// The side of a flow program that a session runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    /// The side that opens the session with `connect`.
    Client,
    /// The side that opens the session with `accept`.
    Server,
}

impl Side {
    /// Returns the value of the `side` label of the metric catalog.
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Client => "client",
            Side::Server => "server",
        }
    }

    /// Returns the name of the action that opens this side.
    pub fn opener(self) -> &'static str {
        match self {
            Side::Client => "connect",
            Side::Server => "accept",
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Domain constants that separate the two sides and this crate from other
/// users of the run seed, such as host selection in `fps-hosts`.
const CLIENT_DOMAIN: u64 = 0x6670_735f_696e_7463; // "fps_intc"
const SERVER_DOMAIN: u64 = 0x6670_735f_696e_7473; // "fps_ints"

/// Returns the generator for one side of one session.
///
/// The same run seed, session id, and side always give the same stream, and
/// different session ids give different streams. `Pcg64Mcg::new` forces the
/// two lowest state bits to 1, so the session id must not sit in the low
/// bits of the state, or ids `4k` to `4k + 3` would share one stream (bug
/// #90). We therefore put a bijective mix of the session id into the high
/// half and a mix of the seed and the side into the low half.
pub fn session_rng(run_seed: u64, session_id: u64, side: Side) -> Pcg64Mcg {
    let domain = match side {
        Side::Client => CLIENT_DOMAIN,
        Side::Server => SERVER_DOMAIN,
    };
    let seed = splitmix64(run_seed);
    // For a fixed seed `x ^ seed` and `splitmix64` are bijections, so two
    // session ids of one run never share a high half.
    let high = splitmix64(session_id ^ seed);
    let low = splitmix64(seed ^ domain);
    Pcg64Mcg::new(u128::from(high) << 64 | u128::from(low))
}

/// SplitMix64 finalizer, a bijection that spreads nearby inputs over the
/// whole output range.
fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use rand::RngCore;

    use super::*;

    #[test]
    fn same_inputs_give_the_same_stream() {
        let mut a = session_rng(42, 7, Side::Client);
        let mut b = session_rng(42, 7, Side::Client);
        for _ in 0..16 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    /// Regression test for bug #90, where ids that differ only in the two
    /// lowest bits shared one stream.
    #[test]
    fn ids_that_differ_in_the_low_bits_get_distinct_streams() {
        let mut firsts = HashSet::new();
        for id in 0..100_000u64 {
            assert!(firsts.insert(session_rng(7, id, Side::Client).next_u64()));
        }
    }

    #[test]
    fn sides_and_seeds_get_distinct_streams() {
        let client = session_rng(42, 7, Side::Client).next_u64();
        let server = session_rng(42, 7, Side::Server).next_u64();
        let other_seed = session_rng(43, 7, Side::Client).next_u64();
        assert_ne!(client, server);
        assert_ne!(client, other_seed);
    }
}
