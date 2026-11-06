// SPDX-License-Identifier: GPL-3.0-only
//! The small generator that every content function draws from.
//!
//! A content function derives one 64 bit seed per call, either from its `seed`
//! argument or from the session generator, and then draws everything from this
//! SplitMix64 generator. The sequence depends only on the seed, so the bytes
//! of a call depend only on the arguments and, without a `seed` argument, on
//! the run seed and the session id.

/// SplitMix64, a 64 bit generator with a full period and good mixing, which
/// is why consecutive seeds such as 0, 1, 2 give unrelated sequences.
#[derive(Clone, Debug)]
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub(crate) fn byte(&mut self) -> u8 {
        (self.next_u64() >> 56) as u8
    }

    /// Returns a number in `0..n`. `n` must not be zero.
    pub(crate) fn below(&mut self, n: usize) -> usize {
        ((u128::from(self.next_u64()) * n as u128) >> 64) as usize
    }

    /// Returns a number in `lo..=hi`.
    pub(crate) fn range(&mut self, lo: usize, hi: usize) -> usize {
        lo + self.below(hi - lo + 1)
    }

    pub(crate) fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// Returns true with probability `num / den`.
    pub(crate) fn chance(&mut self, num: usize, den: usize) -> bool {
        self.below(den) < num
    }

    /// Returns `digits` lowercase hexadecimal digits.
    pub(crate) fn hex(&mut self, digits: usize) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        (0..digits)
            .map(|_| char::from(HEX[self.below(16)]))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::Rng;

    #[test]
    fn the_same_seed_gives_the_same_sequence() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn consecutive_seeds_differ() {
        assert_ne!(Rng::new(0).next_u64(), Rng::new(1).next_u64());
    }

    #[test]
    fn below_stays_in_range() {
        let mut r = Rng::new(1);
        for n in 1..50 {
            for _ in 0..50 {
                assert!(r.below(n) < n);
            }
        }
        assert_eq!(r.hex(12).len(), 12);
    }
}
