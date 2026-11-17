// SPDX-License-Identifier: GPL-3.0-only
//! The addresses that the hosts of a DHCP pool leased.
//!
//! A static pool maps host index to address by arithmetic. A DHCP pool has
//! only what the servers gave, so [`LeaseTable`] keeps one slot per host of
//! the slice and fills it from the leases of [`crate::dhcp::DhcpHosts`]. The
//! table is plain data. The thread that runs the DHCP clients owns it, and it
//! hands a clone to every consumer that needs the addresses at a moment: the
//! neighbor responder ([`crate::NeighborTable::add_leased`]), the workers that
//! pick the source host of a session, and the capture filter of the port.
//! [`LeaseTable::generation`] changes whenever the set of addresses changes, so
//! a consumer knows when its clone is out of date. A renewal that keeps the
//! address does not change it.

use std::collections::{BTreeSet, HashMap};
use std::net::Ipv4Addr;

use crate::pool::Ipv4Prefix;
use crate::slice::HostRange;

/// What a host learned from its lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostLease {
    /// The address of the host.
    pub addr: Ipv4Addr,
    /// The prefix length of the subnet mask, if the server sent a mask.
    pub prefix_len: Option<u8>,
    /// The first router, which is the gateway of the host.
    pub router: Option<Ipv4Addr>,
}

/// The leases of the hosts of one slice of a DHCP pool.
#[derive(Debug, Clone, Default)]
pub struct LeaseTable {
    range: HostRange,
    slots: Vec<Option<HostLease>>,
    by_addr: HashMap<Ipv4Addr, u32>,
    bound: usize,
    generation: u64,
}

impl LeaseTable {
    /// Returns a table with an empty slot for each host of `range`.
    pub fn new(range: HostRange) -> Self {
        Self {
            range,
            slots: vec![None; usize::try_from(range.count).unwrap_or(0)],
            by_addr: HashMap::new(),
            bound: 0,
            generation: 0,
        }
    }

    /// The host indexes of the table.
    pub fn range(&self) -> HostRange {
        self.range
    }

    /// A number that changes whenever an address is added, replaced, or lost,
    /// and does not change when a lease is renewed for the same address.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The number of hosts that hold an address.
    pub fn bound(&self) -> usize {
        self.bound
    }

    /// Reports whether every host of the slice holds an address.
    pub fn is_complete(&self) -> bool {
        self.bound == self.slots.len()
    }

    fn slot(&self, index: u32) -> Option<usize> {
        self.range
            .contains(index)
            .then(|| (index - self.range.offset) as usize)
    }

    /// The lease of host `index`, or `None` when the host holds none or lies
    /// outside the slice.
    pub fn get(&self, index: u32) -> Option<&HostLease> {
        self.slots.get(self.slot(index)?)?.as_ref()
    }

    /// The address of host `index`.
    pub fn addr_of(&self, index: u32) -> Option<Ipv4Addr> {
        self.get(index).map(|l| l.addr)
    }

    /// The router of host `index`.
    pub fn router_of(&self, index: u32) -> Option<Ipv4Addr> {
        self.get(index).and_then(|l| l.router)
    }

    /// The host that holds `addr`. When two hosts hold the same address, which
    /// a faulty server can cause, it is the one that leased it last.
    pub fn index_of(&self, addr: Ipv4Addr) -> Option<u32> {
        self.by_addr.get(&addr).copied()
    }

    /// The hosts that hold an address, in host order, with their leases.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &HostLease)> {
        let offset = self.range.offset;
        self.slots
            .iter()
            .enumerate()
            .filter_map(move |(i, l)| Some((offset + u32::try_from(i).ok()?, l.as_ref()?)))
    }

    /// The addresses of the hosts that hold one, as host prefixes of length 32
    /// in address order, without duplicates. The capture filter of a DHCP port
    /// keeps the sessions of these prefixes.
    pub fn host_prefixes(&self) -> Vec<Ipv4Prefix> {
        let addrs: BTreeSet<u32> = self.iter().map(|(_, l)| u32::from(l.addr)).collect();
        addrs
            .into_iter()
            .filter_map(|a| Ipv4Prefix::new(Ipv4Addr::from(a), 32).ok())
            .collect()
    }

    /// The subnets that the masks of the leases name, in address order,
    /// without duplicates. A lease without a usable mask adds none.
    pub fn subnets(&self) -> Vec<Ipv4Prefix> {
        let nets: BTreeSet<(u32, u8)> = self
            .iter()
            .filter_map(|(_, l)| {
                let len = l.prefix_len?;
                let mask = u32::MAX.checked_shl(32 - u32::from(len)).unwrap_or(0);
                Some((u32::from(l.addr) & mask, len))
            })
            .collect();
        nets.into_iter()
            .filter_map(|(n, len)| Ipv4Prefix::new(Ipv4Addr::from(n), len).ok())
            .collect()
    }

    /// The number of addresses that more than one host holds.
    pub fn duplicates(&self) -> usize {
        self.bound - self.by_addr.len().min(self.bound)
    }

    /// Stores the lease of host `index`. It returns whether the address set
    /// changed, which a renewal for the same address does not do. An index
    /// outside the slice is ignored.
    pub(crate) fn set(&mut self, index: u32, lease: HostLease) -> bool {
        let Some(at) = self.slot(index) else {
            return false;
        };
        let Some(slot) = self.slots.get_mut(at) else {
            return false;
        };
        let old = slot.replace(lease);
        let changed = match old {
            Some(o) if o.addr == lease.addr => {
                // The same address: only the mask or the router may differ.
                false
            }
            Some(o) => {
                if self.by_addr.get(&o.addr) == Some(&index) {
                    self.by_addr.remove(&o.addr);
                }
                true
            }
            None => {
                self.bound += 1;
                true
            }
        };
        self.by_addr.insert(lease.addr, index);
        if changed {
            self.generation += 1;
        }
        changed
    }

    /// Forgets the lease of host `index` and reports whether there was one.
    pub(crate) fn clear(&mut self, index: u32) -> bool {
        let Some(old) = self
            .slot(index)
            .and_then(|at| self.slots.get_mut(at))
            .and_then(Option::take)
        else {
            return false;
        };
        if self.by_addr.get(&old.addr) == Some(&index) {
            self.by_addr.remove(&old.addr);
        }
        self.bound -= 1;
        self.generation += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease(last: u8) -> HostLease {
        HostLease {
            addr: Ipv4Addr::new(10, 0, 0, last),
            prefix_len: Some(24),
            router: Some(Ipv4Addr::new(10, 0, 0, 1)),
        }
    }

    #[test]
    fn a_new_table_is_empty_and_incomplete() {
        let t = LeaseTable::new(HostRange::new(100, 3));
        assert_eq!(t.bound(), 0);
        assert!(!t.is_complete());
        assert_eq!(t.generation(), 0);
        assert_eq!(t.iter().count(), 0);
        assert!(t.host_prefixes().is_empty());
        assert!(t.subnets().is_empty());
        assert_eq!(t.range(), HostRange::new(100, 3));
    }

    #[test]
    fn leases_are_stored_by_host_index_with_the_slice_offset() {
        let mut t = LeaseTable::new(HostRange::new(100, 3));
        assert!(t.set(101, lease(11)));
        assert!(t.set(100, lease(10)));
        assert_eq!(t.addr_of(100), Some(Ipv4Addr::new(10, 0, 0, 10)));
        assert_eq!(t.addr_of(101), Some(Ipv4Addr::new(10, 0, 0, 11)));
        assert_eq!(t.addr_of(102), None);
        assert_eq!(t.addr_of(99), None);
        assert_eq!(t.addr_of(103), None);
        assert_eq!(t.router_of(100), Some(Ipv4Addr::new(10, 0, 0, 1)));
        assert_eq!(t.index_of(Ipv4Addr::new(10, 0, 0, 11)), Some(101));
        assert_eq!(t.bound(), 2);
        let hosts: Vec<u32> = t.iter().map(|(i, _)| i).collect();
        assert_eq!(hosts, [100, 101]);
        assert!(t.set(102, lease(12)));
        assert!(t.is_complete());
        // An index outside the slice changes nothing.
        assert!(!t.set(103, lease(13)));
        assert!(!t.clear(103));
        assert_eq!(t.bound(), 3);
    }

    #[test]
    fn a_renewal_for_the_same_address_keeps_the_generation() {
        let mut t = LeaseTable::new(HostRange::new(0, 2));
        assert!(t.set(0, lease(10)));
        let g = t.generation();
        let mut renewed = lease(10);
        renewed.router = Some(Ipv4Addr::new(10, 0, 0, 254));
        assert!(!t.set(0, renewed));
        assert_eq!(t.generation(), g);
        assert_eq!(t.router_of(0), Some(Ipv4Addr::new(10, 0, 0, 254)));
    }

    #[test]
    fn a_new_address_for_a_host_replaces_the_old_one() {
        let mut t = LeaseTable::new(HostRange::new(0, 2));
        t.set(0, lease(10));
        let g = t.generation();
        assert!(t.set(0, lease(20)));
        assert!(t.generation() > g);
        assert_eq!(t.index_of(Ipv4Addr::new(10, 0, 0, 10)), None);
        assert_eq!(t.index_of(Ipv4Addr::new(10, 0, 0, 20)), Some(0));
        assert_eq!(t.bound(), 1);
    }

    #[test]
    fn clearing_a_lease_frees_the_address() {
        let mut t = LeaseTable::new(HostRange::new(0, 2));
        t.set(0, lease(10));
        let g = t.generation();
        assert!(t.clear(0));
        assert!(!t.clear(0));
        assert!(t.generation() > g);
        assert_eq!(t.index_of(Ipv4Addr::new(10, 0, 0, 10)), None);
        assert_eq!(t.bound(), 0);
    }

    #[test]
    fn two_hosts_with_one_address_are_counted() {
        let mut t = LeaseTable::new(HostRange::new(0, 3));
        t.set(0, lease(10));
        t.set(1, lease(10));
        t.set(2, lease(12));
        assert_eq!(t.bound(), 3);
        assert_eq!(t.duplicates(), 1);
        assert_eq!(t.index_of(Ipv4Addr::new(10, 0, 0, 10)), Some(1));
        // The first host leaves, and the second keeps the address.
        t.clear(0);
        assert_eq!(t.index_of(Ipv4Addr::new(10, 0, 0, 10)), Some(1));
        assert_eq!(t.duplicates(), 0);
    }

    #[test]
    fn host_prefixes_and_subnets_cover_the_leases_once() {
        let mut t = LeaseTable::new(HostRange::new(0, 4));
        t.set(0, lease(30));
        t.set(1, lease(10));
        t.set(2, lease(10));
        t.set(
            3,
            HostLease {
                addr: Ipv4Addr::new(10, 0, 1, 5),
                prefix_len: None,
                router: None,
            },
        );
        let hosts: Vec<String> = t.host_prefixes().iter().map(ToString::to_string).collect();
        assert_eq!(hosts, ["10.0.0.10/32", "10.0.0.30/32", "10.0.1.5/32"]);
        let nets: Vec<String> = t.subnets().iter().map(ToString::to_string).collect();
        assert_eq!(nets, ["10.0.0.0/24"]);
    }

    #[test]
    fn a_slice_of_zero_hosts_is_a_complete_empty_table() {
        let t = LeaseTable::new(HostRange::new(5, 0));
        assert!(t.is_complete());
        assert_eq!(t.addr_of(5), None);
    }
}
