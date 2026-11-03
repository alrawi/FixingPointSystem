// SPDX-License-Identifier: GPL-3.0-only
//! Slicing of host pools by agent and by worker.
//!
//! The controller splits each pool into one contiguous slice per agent, and
//! each agent splits its slice into one contiguous range per worker. Both
//! steps use [`HostRange::split`], which gives every part a share
//! proportional to its weight, such as the core count of an agent. The parts
//! are disjoint, cover the whole range, and lie in the order of the weights.
//! Disjoint host indexes give disjoint IPv4 addresses, IPv6 addresses, and MAC
//! addresses, because the mapping in [`crate::pool`] is injective.

/// Errors that slicing or checking a partition reports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SliceError {
    /// No part has a positive weight.
    #[error("split of hosts {offset}..{end}: no part has a positive weight")]
    NoWeight {
        /// The first host index of the range.
        offset: u32,
        /// One past the last host index of the range.
        end: u64,
    },
    /// A range runs past host index `u32::MAX`.
    #[error("host range at offset {offset} with {count} hosts runs past the index space")]
    Overflow {
        /// The first host index of the range.
        offset: u32,
        /// The number of hosts.
        count: u32,
    },
    /// A part does not start where the previous part ended.
    #[error("part {part} starts at host {got}, expected {want}")]
    Gap {
        /// The index of the part in the list.
        part: usize,
        /// The first host index of the part.
        got: u32,
        /// The host index where the part should start.
        want: u64,
    },
    /// The parts end before or after the whole range.
    #[error("parts end at host {got}, the range ends at {want}")]
    Coverage {
        /// One past the last host index of the parts.
        got: u64,
        /// One past the last host index of the range.
        want: u64,
    },
}

/// A contiguous range of host indexes of one pool, like the `HostRange` and
/// `HostPoolSlice` messages of the agent contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct HostRange {
    /// The pool index of the first host.
    pub offset: u32,
    /// The number of hosts.
    pub count: u32,
}

impl HostRange {
    /// Returns the range of `count` hosts that starts at `offset`.
    pub fn new(offset: u32, count: u32) -> Self {
        Self { offset, count }
    }

    /// One past the last host index. The type is `u64`, so the sum never
    /// overflows.
    pub fn end(&self) -> u64 {
        u64::from(self.offset) + u64::from(self.count)
    }

    /// Reports whether the range holds no hosts.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Reports whether host `index` lies inside the range.
    pub fn contains(&self, index: u32) -> bool {
        index >= self.offset && u64::from(index) < self.end()
    }

    /// Reports whether `inner` lies inside this range.
    pub fn covers(&self, inner: &HostRange) -> bool {
        inner.offset >= self.offset && inner.end() <= self.end()
    }

    /// The host indexes of the range in order.
    pub fn indexes(&self) -> impl Iterator<Item = u32> {
        // The end may exceed u32::MAX only for an invalid range, which
        // `split` refuses, so the clamp never drops a valid index.
        let end = u32::try_from(self.end()).unwrap_or(u32::MAX);
        self.offset..end
    }

    /// Splits the range into one contiguous part per weight, in the order of
    /// the weights.
    ///
    /// Each part with a positive weight first gets one host while hosts
    /// remain, so no such part stays empty when the range holds at least as
    /// many hosts as there are positive weights. The rest goes out in
    /// proportion to the weights by the largest remainder method, and ties go
    /// to the lower part. A part with weight zero gets no host. The result
    /// depends only on the range and the weights.
    pub fn split(&self, weights: &[u32]) -> Result<Vec<HostRange>, SliceError> {
        if self.end() > u64::from(u32::MAX) + 1 {
            return Err(SliceError::Overflow {
                offset: self.offset,
                count: self.count,
            });
        }
        let total: u64 = weights.iter().map(|w| u64::from(*w)).sum();
        if total == 0 {
            return Err(SliceError::NoWeight {
                offset: self.offset,
                end: self.end(),
            });
        }
        let mut counts = vec![0u64; weights.len()];
        let mut left = u64::from(self.count);
        for (c, w) in counts.iter_mut().zip(weights) {
            if *w > 0 && left > 0 {
                *c = 1;
                left -= 1;
            }
        }
        let share = left;
        let mut rems = Vec::with_capacity(weights.len());
        for (i, (c, w)) in counts.iter_mut().zip(weights).enumerate() {
            let exact = u128::from(share) * u128::from(*w);
            let quota = exact / u128::from(total);
            // quota <= share, which is a u64.
            let quota = u64::try_from(quota).unwrap_or(share);
            *c += quota;
            left -= quota;
            rems.push((exact % u128::from(total), i));
        }
        // Largest remainder first, lower part first on ties.
        rems.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, i) in rems
            .iter()
            .take(usize::try_from(left).unwrap_or(usize::MAX))
        {
            counts[*i] += 1;
        }
        let mut next = u64::from(self.offset);
        let mut parts = Vec::with_capacity(weights.len());
        for c in counts {
            // Every count and offset stays within the range, which fits in
            // the u32 index space as checked above.
            let offset = u32::try_from(next).unwrap_or(u32::MAX);
            let count = u32::try_from(c).unwrap_or(u32::MAX);
            parts.push(HostRange::new(offset, count));
            next += c;
        }
        Ok(parts)
    }

    /// Splits the range into `parts` parts of equal weight.
    pub fn split_even(&self, parts: usize) -> Result<Vec<HostRange>, SliceError> {
        self.split(&vec![1; parts])
    }
}

/// Checks that `parts`, in order, are contiguous, disjoint, and cover `whole`
/// exactly. Empty parts are allowed anywhere. An agent runs this check on the
/// worker ranges of each pool in its plan against its own slice.
pub fn check_partition(whole: &HostRange, parts: &[HostRange]) -> Result<(), SliceError> {
    let mut next = u64::from(whole.offset);
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() {
            continue;
        }
        if u64::from(p.offset) != next {
            return Err(SliceError::Gap {
                part: i,
                got: p.offset,
                want: next,
            });
        }
        next = p.end();
    }
    if next != whole.end() {
        return Err(SliceError::Coverage {
            got: next,
            want: whole.end(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn split_follows_weights_and_order() {
        let r = HostRange::new(0, 254);
        assert_eq!(
            r.split_even(2).unwrap(),
            vec![HostRange::new(0, 127), HostRange::new(127, 127)]
        );
        let parts = HostRange::new(100, 10).split(&[1, 0, 3]).unwrap();
        assert_eq!(
            parts,
            vec![
                HostRange::new(100, 3),
                HostRange::new(103, 0),
                HostRange::new(103, 7)
            ]
        );
    }

    #[test]
    fn split_gives_each_weighted_part_a_host_when_possible() {
        let parts = HostRange::new(0, 10).split(&[100, 1]).unwrap();
        assert_eq!(parts, vec![HostRange::new(0, 9), HostRange::new(9, 1)]);
        let parts = HostRange::new(0, 2).split_even(3).unwrap();
        assert_eq!(
            parts,
            vec![
                HostRange::new(0, 1),
                HostRange::new(1, 1),
                HostRange::new(2, 0)
            ]
        );
    }

    #[test]
    fn split_refuses_zero_weights_and_overflow() {
        assert!(matches!(
            HostRange::new(0, 5).split(&[0, 0]),
            Err(SliceError::NoWeight { .. })
        ));
        assert!(matches!(
            HostRange::new(0, 5).split(&[]),
            Err(SliceError::NoWeight { .. })
        ));
        assert!(matches!(
            HostRange::new(u32::MAX, 2).split(&[1]),
            Err(SliceError::Overflow { .. })
        ));
        let top = HostRange::new(u32::MAX - 1, 1).split(&[1, 1]).unwrap();
        assert_eq!(top[1], HostRange::new(u32::MAX, 0));
    }

    #[test]
    fn check_partition_reports_gaps_overlaps_and_coverage() {
        let whole = HostRange::new(10, 10);
        let ok = [
            HostRange::new(10, 4),
            HostRange::new(0, 0),
            HostRange::new(14, 6),
        ];
        assert!(check_partition(&whole, &ok).is_ok());
        let overlap = [HostRange::new(10, 5), HostRange::new(14, 6)];
        assert!(matches!(
            check_partition(&whole, &overlap),
            Err(SliceError::Gap { part: 1, .. })
        ));
        let short = [HostRange::new(10, 5)];
        assert!(matches!(
            check_partition(&whole, &short),
            Err(SliceError::Coverage { got: 15, want: 20 })
        ));
    }

    fn weights() -> impl Strategy<Value = Vec<u32>> {
        prop::collection::vec(prop_oneof![Just(0u32), 1u32..64, 1u32..100_000], 1..24)
            .prop_filter("one positive weight", |w| w.iter().any(|x| *x > 0))
    }

    proptest! {
        #[test]
        fn split_parts_are_disjoint_and_cover(
            offset in 0u32..1_000_000,
            count in 0u32..5_000_000,
            w in weights(),
        ) {
            let whole = HostRange::new(offset, count);
            let parts = whole.split(&w).unwrap();
            prop_assert_eq!(parts.len(), w.len());
            prop_assert!(check_partition(&whole, &parts).is_ok());
            let sum: u64 = parts.iter().map(|p| u64::from(p.count)).sum();
            prop_assert_eq!(sum, u64::from(count));
            for (a, b) in parts.iter().zip(parts.iter().skip(1)) {
                prop_assert_eq!(a.end(), u64::from(b.offset));
            }
            let positive = w.iter().filter(|x| **x > 0).count();
            for (p, wi) in parts.iter().zip(&w) {
                prop_assert!(whole.covers(p));
                if *wi == 0 {
                    prop_assert!(p.is_empty());
                } else if count as usize >= positive {
                    prop_assert!(!p.is_empty());
                }
            }
        }

        #[test]
        fn split_is_proportional_within_one_host(
            count in 0u32..5_000_000,
            w in weights(),
        ) {
            let parts = HostRange::new(0, count).split(&w).unwrap();
            let positive = w.iter().filter(|x| **x > 0).count() as f64;
            let total: f64 = w.iter().map(|x| f64::from(*x)).sum();
            let share = (f64::from(count) - positive).max(0.0);
            for (p, wi) in parts.iter().zip(&w) {
                let floor: f64 = if *wi > 0 { 1.0 } else { 0.0 };
                let want = share * f64::from(*wi) / total;
                let got = f64::from(p.count) - floor.min(f64::from(p.count));
                prop_assert!((got - want).abs() <= 1.0 + 1e-6, "got {} want {}", got, want);
            }
        }

        #[test]
        fn agent_then_worker_slices_are_disjoint_and_cover_the_pool(
            count in 1u32..200_000,
            agents in weights(),
            workers in weights(),
        ) {
            let pool = HostRange::new(0, count);
            let mut all = Vec::new();
            for slice in pool.split(&agents).unwrap() {
                let ranges = slice.split(&workers).unwrap();
                prop_assert!(check_partition(&slice, &ranges).is_ok());
                all.extend(ranges);
            }
            prop_assert!(check_partition(&pool, &all).is_ok());
        }
    }
}
