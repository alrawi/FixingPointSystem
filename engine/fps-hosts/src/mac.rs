// SPDX-License-Identifier: GPL-3.0-only
//! MAC address allocation for the hosts of one port.
//!
//! The agent contract gives host index `i` of a pool the address
//! `mac_base + i`, so every agent that serves a slice of the pool computes the
//! same address without talking to the others. [`MacAllocator`] keeps that
//! rule and adds the guarantee that no two hosts of a port share an address.
//! It records every block of addresses that a port hands out, and it refuses
//! a block that overlaps an earlier one, that contains a multicast or the
//! all zero address, or that runs past 48 bits. Addresses that the port owns
//! already, such as the MAC address of the interface and of the gateway, are
//! reserved first and no host may take them.
//!
//! A pool without `mac_base` gets one from the allocator. The base is a
//! locally administered unicast address (`02:xx:xx:xx:xx:xx`) that starts at a
//! point derived from the pool name and moves up past every block that is
//! taken, so the same claims in the same order give the same base. Two agents
//! on one link agree on it only when they make the same claims, so the
//! controller should set `mac_base` for a pool that more than one agent
//! serves.

use crate::pool::{HostPool, MacAddr};
use crate::slice::HostRange;

/// The lowest address of the block that automatic bases come from.
const AUTO_FIRST: u64 = 0x0200_0000_0000;
/// The highest address of that block.
const AUTO_LAST: u64 = 0x02ff_ffff_ffff;
/// The multicast bit of the first octet, as a value of the 48 bit address.
const MULTICAST_BIT: u64 = 1 << 40;
const MAX_MAC: u64 = 0xffff_ffff_ffff;

/// Errors that the MAC allocator reports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MacError {
    /// A block of addresses overlaps one that is taken.
    #[error("{owner}: MAC address {mac} is already taken by {holder}")]
    Conflict {
        /// The pool or label that asked for the block.
        owner: String,
        /// The first address of the overlap.
        mac: MacAddr,
        /// The pool or label that holds the address.
        holder: String,
    },
    /// A block holds a multicast address, which no station can use as its own.
    #[error("{owner}: the block starting at {first} holds a multicast address")]
    Multicast {
        /// The pool or label that asked for the block.
        owner: String,
        /// The first address of the block.
        first: MacAddr,
    },
    /// A block holds the all zero address.
    #[error("{owner}: the block holds the all zero MAC address")]
    Zero {
        /// The pool or label that asked for the block.
        owner: String,
    },
    /// A block runs past `ff:ff:ff:ff:ff:ff`.
    #[error("{owner}: {count} MAC addresses from {first} run past 48 bits")]
    Overflow {
        /// The pool or label that asked for the block.
        owner: String,
        /// The first address of the block.
        first: MacAddr,
        /// The number of addresses.
        count: u64,
    },
    /// The slice does not lie inside the pool.
    #[error("{pool}: slice {offset}+{count} lies outside the pool of {pool_count} hosts")]
    SliceOutsidePool {
        /// The pool name.
        pool: String,
        /// The first host index of the slice.
        offset: u32,
        /// The number of hosts of the slice.
        count: u32,
        /// The number of hosts of the pool.
        pool_count: u32,
    },
    /// The slice holds no host.
    #[error("{pool}: the slice holds no host")]
    EmptySlice {
        /// The pool name.
        pool: String,
    },
    /// No free block of the automatic range holds the pool.
    #[error("{pool}: no free block of {count} locally administered MAC addresses is left")]
    AutoExhausted {
        /// The pool name.
        pool: String,
        /// The number of addresses the pool needs.
        count: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Claim {
    owner: String,
    first: u64,
    /// Inclusive.
    last: u64,
}

/// The MAC addresses of one slice of a host pool. Host index `i` of the slice
/// has the address `mac_of(i)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacBlock {
    pool: String,
    range: HostRange,
    /// The address of host index `range.offset`.
    first: u64,
}

impl MacBlock {
    /// The name of the pool.
    pub fn pool(&self) -> &str {
        &self.pool
    }

    /// The host indexes that the block covers.
    pub fn range(&self) -> HostRange {
        self.range
    }

    /// The address of the first host of the slice.
    pub fn first(&self) -> MacAddr {
        MacAddr::from_u64(self.first)
    }

    /// The address of host `index`, or `None` when the index lies outside the
    /// slice.
    pub fn mac_of(&self, index: u32) -> Option<MacAddr> {
        self.range
            .contains(index)
            .then(|| MacAddr::from_u64(self.first + u64::from(index - self.range.offset)))
    }

    /// The host index that owns `mac`, or `None` when no host of the slice
    /// does.
    pub fn host_of(&self, mac: MacAddr) -> Option<u32> {
        let m = mac.to_u64();
        let delta = m.checked_sub(self.first)?;
        let index = u64::from(self.range.offset) + delta;
        let index = u32::try_from(index).ok()?;
        self.range.contains(index).then_some(index)
    }
}

/// The ledger of the MAC addresses of one port.
#[derive(Debug, Clone, Default)]
pub struct MacAllocator {
    claims: Vec<Claim>,
}

impl MacAllocator {
    /// Returns an allocator with no address taken.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserves one address that a station of the port owns already, such as
    /// the interface or the gateway, so that no host receives it.
    ///
    /// Reserving the same address twice is an error, because a second owner of
    /// an address is a conflict.
    pub fn reserve(&mut self, label: &str, mac: MacAddr) -> Result<(), MacError> {
        let m = mac.to_u64();
        self.check_block(label, m, 1)?;
        self.claims.push(Claim {
            owner: label.to_owned(),
            first: m,
            last: m,
        });
        Ok(())
    }

    /// Claims the addresses of the hosts of `range` in `pool` and returns them.
    ///
    /// A pool with `mac_base` uses `mac_base + i`, as the contract says. A pool
    /// without one gets an automatic base for the whole pool, and the claim
    /// records only the slice.
    pub fn claim(&mut self, pool: &HostPool, range: HostRange) -> Result<MacBlock, MacError> {
        let name = pool.name();
        if range.is_empty() {
            return Err(MacError::EmptySlice {
                pool: name.to_owned(),
            });
        }
        if range.end() > u64::from(pool.count()) {
            return Err(MacError::SliceOutsidePool {
                pool: name.to_owned(),
                offset: range.offset,
                count: range.count,
                pool_count: pool.count(),
            });
        }
        let base = match pool.mac_base() {
            Some(b) => b.to_u64(),
            None => self.auto_base(name, pool.count())?,
        };
        let first = base + u64::from(range.offset);
        let count = u64::from(range.count);
        self.check_block(name, first, count)?;
        self.claims.push(Claim {
            owner: name.to_owned(),
            first,
            last: first + count - 1,
        });
        Ok(MacBlock {
            pool: name.to_owned(),
            range,
            first,
        })
    }

    /// The number of addresses that the ledger holds.
    pub fn taken(&self) -> u64 {
        self.claims.iter().map(|c| c.last - c.first + 1).sum()
    }

    fn check_block(&self, owner: &str, first: u64, count: u64) -> Result<(), MacError> {
        let first_mac = MacAddr::from_u64(first & MAX_MAC);
        let last = match first.checked_add(count - 1) {
            Some(l) if l <= MAX_MAC => l,
            _ => {
                return Err(MacError::Overflow {
                    owner: owner.to_owned(),
                    first: first_mac,
                    count,
                })
            }
        };
        if first == 0 {
            return Err(MacError::Zero {
                owner: owner.to_owned(),
            });
        }
        if has_multicast(first, last) {
            return Err(MacError::Multicast {
                owner: owner.to_owned(),
                first: first_mac,
            });
        }
        if let Some((claim, at)) = self.overlap(first, last) {
            return Err(MacError::Conflict {
                owner: owner.to_owned(),
                mac: MacAddr::from_u64(at),
                holder: claim.owner.clone(),
            });
        }
        Ok(())
    }

    /// The first claim that overlaps `first..=last` and the first address of
    /// the overlap.
    fn overlap(&self, first: u64, last: u64) -> Option<(&Claim, u64)> {
        self.claims
            .iter()
            .filter(|c| c.first <= last && first <= c.last)
            .min_by_key(|c| c.first.max(first))
            .map(|c| (c, c.first.max(first)))
    }

    /// Finds the base of a pool without `mac_base`: the lowest free block of
    /// `count` addresses at or above a point that the pool name selects,
    /// wrapping once to the start of the automatic range.
    fn auto_base(&self, pool: &str, count: u32) -> Result<u64, MacError> {
        let span = u64::from(count);
        let room = AUTO_LAST - AUTO_FIRST + 1;
        let exhausted = || MacError::AutoExhausted {
            pool: pool.to_owned(),
            count,
        };
        if span > room {
            return Err(exhausted());
        }
        let start = AUTO_FIRST + (fnv1a(pool.as_bytes()) % (room - span + 1));
        let mut candidate = start;
        let mut wrapped = false;
        loop {
            let last = candidate + span - 1;
            if last > AUTO_LAST {
                if wrapped {
                    return Err(exhausted());
                }
                wrapped = true;
                candidate = AUTO_FIRST;
                continue;
            }
            match self.overlap(candidate, last) {
                None => return Ok(candidate),
                Some((claim, _)) => {
                    candidate = claim.last + 1;
                    if wrapped && candidate > start {
                        return Err(exhausted());
                    }
                }
            }
        }
    }
}

/// Reports whether any address of `first..=last` has the multicast bit.
fn has_multicast(first: u64, last: u64) -> bool {
    if first & MULTICAST_BIT != 0 {
        return true;
    }
    // The next block with the bit set starts at the next odd multiple of
    // 2^40, and `first` has the bit clear, so it lies above `first`.
    let next_multicast = ((first >> 41) << 41) | MULTICAST_BIT;
    last >= next_multicast
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::PoolSpec;

    fn pool(name: &str, count: u32, mac_base: Option<&str>) -> HostPool {
        HostPool::parse(&PoolSpec {
            name,
            ipv4: None,
            ipv6: Some("2001:db8::/64"),
            count,
            gateway: None,
            mac_base,
        })
        .unwrap()
    }

    fn mac(s: &str) -> MacAddr {
        s.parse().unwrap()
    }

    #[test]
    fn explicit_base_follows_the_contract() {
        let mut a = MacAllocator::new();
        let p = pool("p", 300, Some("02:00:00:00:00:fe"));
        let b = a.claim(&p, HostRange::new(0, 300)).unwrap();
        assert_eq!(b.mac_of(0), Some(mac("02:00:00:00:00:fe")));
        assert_eq!(b.mac_of(2), Some(mac("02:00:00:00:01:00")));
        assert_eq!(b.mac_of(300), None);
        assert_eq!(b.host_of(mac("02:00:00:00:01:00")), Some(2));
        assert_eq!(b.host_of(mac("02:00:00:00:00:fd")), None);
        assert_eq!(b.host_of(mac("02:00:00:00:02:fe")), None);
        assert_eq!(p.mac(2).unwrap(), b.mac_of(2));
    }

    #[test]
    fn slice_uses_the_address_of_its_pool_index() {
        let mut a = MacAllocator::new();
        let p = pool("p", 100, Some("02:00:00:00:00:00"));
        let b = a.claim(&p, HostRange::new(40, 10)).unwrap();
        assert_eq!(b.mac_of(40), p.mac(40).unwrap());
        assert_eq!(b.mac_of(49), p.mac(49).unwrap());
        assert_eq!(b.mac_of(39), None);
        assert_eq!(b.host_of(mac("02:00:00:00:00:29")), Some(41));
        assert_eq!(a.taken(), 10);
    }

    #[test]
    fn overlapping_pools_conflict() {
        let mut a = MacAllocator::new();
        a.claim(
            &pool("a", 100, Some("02:00:00:00:00:00")),
            HostRange::new(0, 100),
        )
        .unwrap();
        let err = a
            .claim(
                &pool("b", 10, Some("02:00:00:00:00:63")),
                HostRange::new(0, 10),
            )
            .unwrap_err();
        assert_eq!(
            err,
            MacError::Conflict {
                owner: "b".into(),
                mac: mac("02:00:00:00:00:63"),
                holder: "a".into()
            }
        );
        // The refused claim leaves the ledger as it was.
        assert_eq!(a.taken(), 100);
        a.claim(
            &pool("c", 10, Some("02:00:00:00:00:64")),
            HostRange::new(0, 10),
        )
        .unwrap();
    }

    #[test]
    fn reserved_addresses_are_refused_to_hosts() {
        let mut a = MacAllocator::new();
        a.reserve("gateway", mac("02:00:00:00:00:0a")).unwrap();
        let err = a
            .claim(
                &pool("p", 20, Some("02:00:00:00:00:00")),
                HostRange::new(0, 20),
            )
            .unwrap_err();
        assert!(matches!(err, MacError::Conflict { holder, .. } if holder == "gateway"));
        assert!(a.reserve("again", mac("02:00:00:00:00:0a")).is_err());
    }

    #[test]
    fn multicast_zero_and_overflow_are_refused() {
        let mut a = MacAllocator::new();
        let err = a
            .claim(
                &pool("m", 1, Some("01:00:5e:00:00:01")),
                HostRange::new(0, 1),
            )
            .unwrap_err();
        assert!(matches!(err, MacError::Multicast { .. }));
        // A block that starts below and runs into the multicast half.
        let err = a
            .claim(
                &pool("x", 4, Some("00:ff:ff:ff:ff:fe")),
                HostRange::new(0, 4),
            )
            .unwrap_err();
        assert!(matches!(err, MacError::Multicast { .. }));
        let err = a
            .claim(
                &pool("z", 1, Some("00:00:00:00:00:00")),
                HostRange::new(0, 1),
            )
            .unwrap_err();
        assert!(matches!(err, MacError::Zero { .. }));
        let err = a.reserve("bcast", mac("ff:ff:ff:ff:ff:ff")).unwrap_err();
        assert!(matches!(err, MacError::Multicast { .. }));
        assert_eq!(a.taken(), 0);
    }

    #[test]
    fn slice_checks() {
        let mut a = MacAllocator::new();
        let p = pool("p", 10, Some("02:00:00:00:00:00"));
        assert!(matches!(
            a.claim(&p, HostRange::new(5, 6)),
            Err(MacError::SliceOutsidePool { .. })
        ));
        assert!(matches!(
            a.claim(&p, HostRange::new(5, 0)),
            Err(MacError::EmptySlice { .. })
        ));
    }

    #[test]
    fn automatic_bases_are_unicast_local_and_disjoint() {
        let mut a = MacAllocator::new();
        let mut blocks = Vec::new();
        for i in 0..20 {
            let p = pool(&format!("pool-{i}"), 20_000, None);
            blocks.push(a.claim(&p, p.range()).unwrap());
        }
        for b in &blocks {
            let first = b.first().0;
            assert_eq!(first[0], 0x02, "locally administered unicast");
        }
        for (i, x) in blocks.iter().enumerate() {
            for y in &blocks[i + 1..] {
                let (xf, yf) = (x.first().to_u64(), y.first().to_u64());
                assert!(xf + 20_000 <= yf || yf + 20_000 <= xf, "{xf:x} {yf:x}");
            }
        }
    }

    #[test]
    fn automatic_base_is_deterministic_and_moves_past_taken_blocks() {
        let p = pool("web", 1000, None);
        let first = MacAllocator::new().claim(&p, p.range()).unwrap().first();
        let again = MacAllocator::new().claim(&p, p.range()).unwrap().first();
        assert_eq!(first, again);
        // Take exactly the block that the pool would get, and the pool moves.
        let mut a = MacAllocator::new();
        a.reserve("squatter", first).unwrap();
        let moved = a.claim(&p, p.range()).unwrap().first();
        assert!(moved.to_u64() > first.to_u64());
        assert_ne!(moved, first);
    }

    #[test]
    fn automatic_base_wraps_and_exhausts() {
        let p = pool("big", u32::MAX, None);
        let mut a = MacAllocator::new();
        // The automatic block holds 2^40 addresses, so the pool fits.
        a.claim(&p, p.range()).unwrap();
        let mut b = MacAllocator::new();
        // Fill the automatic range with 256 blocks that each leave one address.
        for part in 0..256u32 {
            let base = mac(&format!("02:{part:02x}:00:00:00:00"));
            let q = pool(&format!("f{part}"), u32::MAX, Some(&base.to_string()));
            // 2^32 - 1 addresses per part, so each leaves one free address.
            b.claim(&q, q.range()).unwrap();
        }
        // 256 free single addresses remain, so a pool of two cannot fit.
        let two = pool("two", 2, None);
        assert!(matches!(
            b.claim(&two, two.range()),
            Err(MacError::AutoExhausted { .. })
        ));
        let one = pool("one", 1, None);
        let m = b.claim(&one, one.range()).unwrap().first();
        assert_eq!(m.0[0], 0x02);
        assert_eq!(m.0[2..], [0xff, 0xff, 0xff, 0xff]);
    }
}
