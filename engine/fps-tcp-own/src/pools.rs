// SPDX-License-Identifier: GPL-3.0-only
//! The host pools of an [`crate::OwnStack`]: which addresses each side serves
//! and the neighbor tables that follow from them.
//!
//! A side serves a slice of a pool of `fps-hosts` ([`HostSlice`]). Host `i` of
//! the pool has the `i`-th usable address of the prefix and the MAC address
//! `mac_base + i`. [`Pools::stations`] claims the MAC addresses of both sides
//! from one ledger, so two sides cannot share a MAC address, and builds the
//! [`Station`] of each side with the other side as its neighbor table.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use std::sync::Arc;

use fps_hosts::{
    HostPool, HostRange, LeaseTable, MacAllocator, MacBlock, MacError, NeighborError,
    NeighborTable, PoolError as HostPoolError, PoolSpec,
};

use crate::leases::{BoardError, LeaseBoard, Leases};
use crate::net::{NetError, Station};

/// Why a pool setup was rejected.
#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    /// A pool of the setup is not valid.
    #[error("host pool: {0}")]
    Pool(#[from] HostPoolError),
    /// The MAC addresses of the two sides conflict, or a slice is empty.
    #[error("MAC addresses: {0}")]
    Mac(#[from] MacError),
    /// The neighbor table refused a slice, for example a pool without IPv4.
    #[error("neighbor table: {0}")]
    Neighbor(#[from] NeighborError),
    /// The VLAN identifier or another link setting is not valid.
    #[error("link: {0}")]
    Link(#[from] NetError),
    /// A pool has no IPv4 prefix where the caller needs IPv4 addresses.
    #[error("pool {0} has no IPv4 prefix")]
    NoIpv4(String),
    /// A lease board could not be made for a DHCP pool.
    #[error("leases: {0}")]
    Board(#[from] BoardError),
}

/// The hosts of one side: a contiguous slice of a pool.
#[derive(Clone, Debug)]
pub struct HostSlice {
    /// The pool of the test configuration.
    pub pool: HostPool,
    /// The hosts of the pool that this side serves.
    pub range: HostRange,
}

impl HostSlice {
    /// A slice of all `pool.count()` hosts of `pool`.
    pub fn whole(pool: HostPool) -> Self {
        let range = HostRange::new(0, pool.count());
        Self { pool, range }
    }

    /// The number of hosts of the slice.
    pub fn hosts(&self) -> u32 {
        self.range.count
    }

    /// The IPv4 address of the `i`-th host of the slice from the lease table
    /// of a DHCP pool (TRN-37), or `None` when the host holds no lease. A
    /// plain struct cannot carry the table, because callers build slices by
    /// hand, so the table is a parameter.
    pub fn address_leased(&self, i: u32, leases: &LeaseTable) -> Option<u32> {
        if i >= self.range.count {
            return None;
        }
        leases.addr_of(self.range.offset + i).map(u32::from)
    }

    /// The host of the slice that holds `ip` in `leases`, as an index into the
    /// slice.
    pub fn index_of_leased(&self, ip: IpAddr, leases: &LeaseTable) -> Option<u32> {
        let IpAddr::V4(a) = ip else { return None };
        let relative = leases.index_of(a)?.checked_sub(self.range.offset)?;
        (relative < self.range.count).then_some(relative)
    }

    /// The IPv4 address of the `i`-th host of the slice, by arithmetic and
    /// without a list, so a pool of a million hosts costs no memory here.
    pub fn address(&self, i: u32) -> Option<u32> {
        if i >= self.range.count {
            return None;
        }
        let addr = self.pool.ipv4_addr(self.range.offset + i).ok()??;
        Some(u32::from(addr))
    }

    /// The IPv6 address of the `i`-th host of the slice, by arithmetic.
    pub fn address6(&self, i: u32) -> Option<Ipv6Addr> {
        if i >= self.range.count {
            return None;
        }
        self.pool.ipv6_addr(self.range.offset + i).ok()?
    }

    /// The address that the stack uses for the `i`-th host when the caller
    /// does not name a family: the IPv4 address when the pool has one, and
    /// otherwise the IPv6 address.
    pub fn ip(&self, i: u32) -> Option<IpAddr> {
        self.address(i)
            .map(|a| IpAddr::V4(Ipv4Addr::from(a)))
            .or_else(|| self.address6(i).map(IpAddr::V6))
    }

    /// The host of the slice that owns `ip`, as an index into the slice, or
    /// `None` when the slice does not serve the address.
    pub fn index_of(&self, ip: IpAddr) -> Option<u32> {
        let index = match ip {
            IpAddr::V4(a) => self.pool.index_of_ipv4(a)?,
            IpAddr::V6(a) => self.pool.index_of_ipv6(a)?,
        };
        let relative = index.checked_sub(self.range.offset)?;
        (relative < self.range.count).then_some(relative)
    }

    /// The IPv4 addresses of the hosts of the slice, in host order.
    pub fn addresses(&self) -> Result<Vec<u32>, PoolError> {
        (self.range.offset..self.range.offset + self.range.count)
            .map(|i| match self.pool.ipv4_addr(i)? {
                Some(a) => Ok(u32::from(a)),
                None => Err(PoolError::NoIpv4(self.pool.name().to_owned())),
            })
            .collect()
    }
}

/// The pools of both sides and the VLAN of the link between them.
#[derive(Clone, Debug)]
pub struct Pools {
    /// The hosts of the client side, which open connections.
    pub client: HostSlice,
    /// The hosts of the server side, which listen.
    pub server: HostSlice,
    /// The 802.1Q VLAN of both ports, or `None` for untagged frames.
    pub vlan: Option<u16>,
}

/// The neighbor tables of both sides: the hosts that each side serves with
/// the MAC addresses that [`Pools::tables`] gave them.
#[derive(Clone, Debug)]
pub struct Tables {
    /// The hosts of the client side.
    pub client: NeighborTable,
    /// The hosts of the server side.
    pub server: NeighborTable,
}

/// The MAC blocks of both sides.
#[derive(Clone, Debug)]
pub struct Blocks {
    /// The block of the client slice.
    pub client: MacBlock,
    /// The block of the server slice.
    pub server: MacBlock,
}

/// Adds the hosts of `slice` to `table`: static hosts by prefix, leased hosts
/// with an empty lease table, which the lease board of the side fills later.
fn add_slice(
    table: &mut NeighborTable,
    slice: &HostSlice,
    block: &MacBlock,
) -> Result<(), PoolError> {
    if slice.pool.is_dhcp() {
        table.add_leased(&slice.pool, block, &LeaseTable::new(slice.range))?;
    } else {
        table.add(&slice.pool, block)?;
    }
    Ok(())
}

/// The stations of both sides.
pub struct Built {
    /// The link layer of the client side.
    pub client: Station,
    /// The link layer of the server side.
    pub server: Station,
}

impl Pools {
    /// One client host at 10.0.0.1 and one server host at 10.0.1.1, untagged.
    /// This is the pair that the stack had before pools.
    #[allow(clippy::expect_used)]
    pub fn pair() -> Self {
        // The prefixes are constants and a unit test builds this value.
        Self::sized("10.0.0.0/24", 1, "10.0.1.0/24", 1, None).expect("the constant pair is valid")
    }

    /// One client host at `fd00:1::1` and one server host at `fd00:2::1`,
    /// untagged: the IPv6 counterpart of [`Pools::pair`].
    #[allow(clippy::expect_used)]
    pub fn pair_v6() -> Self {
        // The prefixes are constants and a unit test builds this value.
        Self::sized_v6("fd00:1::/64", 1, "fd00:2::/64", 1, None)
            .expect("the constant pair is valid")
    }

    /// `client_hosts` hosts in the IPv6 prefix `client_prefix` and
    /// `server_hosts` hosts in `server_prefix`, with MAC addresses from a base
    /// that `fps-hosts` derives from the pool name. The stack then speaks IPv6
    /// only on both sides, and host `i` of a pool has the `i`-th usable
    /// address of its prefix. A pool that has both families is built from a
    /// `HostPool` with both prefixes and [`HostSlice::whole`].
    pub fn sized_v6(
        client_prefix: &str,
        client_hosts: u32,
        server_prefix: &str,
        server_hosts: u32,
        vlan: Option<u16>,
    ) -> Result<Self, PoolError> {
        let pool = |name, prefix, count| {
            HostPool::parse(&PoolSpec {
                name,
                ipv6: Some(prefix),
                count,
                ..PoolSpec::default()
            })
        };
        Ok(Self {
            client: HostSlice::whole(pool("client", client_prefix, client_hosts)?),
            server: HostSlice::whole(pool("server", server_prefix, server_hosts)?),
            vlan,
        })
    }

    /// `client_hosts` client hosts in a prefix of 10.0.0.0 and `server_hosts`
    /// server hosts in a prefix of 11.0.0.0, each as long as the pool needs,
    /// untagged unless `vlan` is set. A pool of 20,000 hosts gets a /17.
    pub fn counts(
        client_hosts: u32,
        server_hosts: u32,
        vlan: Option<u16>,
    ) -> Result<Self, PoolError> {
        let prefix = |base: &str, hosts: u32| {
            // The network and broadcast addresses are not hosts.
            let bits = (u64::from(hosts) + 2)
                .next_power_of_two()
                .trailing_zeros()
                .max(2);
            format!("{base}/{}", 32 - bits.min(30))
        };
        Self::sized(
            &prefix("10.0.0.0", client_hosts),
            client_hosts,
            &prefix("11.0.0.0", server_hosts),
            server_hosts,
            vlan,
        )
    }

    /// `client_hosts` hosts in `client_prefix` and `server_hosts` hosts in
    /// `server_prefix`, with MAC addresses from a base that `fps-hosts`
    /// derives from the pool name. The setup is invalid when a prefix is too
    /// short for its hosts.
    pub fn sized(
        client_prefix: &str,
        client_hosts: u32,
        server_prefix: &str,
        server_hosts: u32,
        vlan: Option<u16>,
    ) -> Result<Self, PoolError> {
        let pool = |name, prefix, count| {
            HostPool::parse(&PoolSpec {
                name,
                ipv4: Some(prefix),
                count,
                ..PoolSpec::default()
            })
        };
        Ok(Self {
            client: HostSlice::whole(pool("client", client_prefix, client_hosts)?),
            server: HostSlice::whole(pool("server", server_prefix, server_hosts)?),
            vlan,
        })
    }

    /// Claims the MAC addresses of both sides from one ledger (the client
    /// block first, then the server block) and returns the neighbor table of
    /// each side. This is the one place that assigns MAC addresses to the
    /// hosts of the pools: [`Pools::stations`] and a datapath that answers
    /// ARP and neighbor discovery both take their tables from it, so they
    /// cannot disagree.
    pub fn tables(&self) -> Result<Tables, PoolError> {
        let blocks = self.blocks()?;
        let mut client = NeighborTable::new();
        add_slice(&mut client, &self.client, &blocks.client)?;
        let mut server = NeighborTable::new();
        add_slice(&mut server, &self.server, &blocks.server)?;
        Ok(Tables { client, server })
    }

    /// The MAC blocks of both sides, claimed from one ledger in the order of
    /// [`Pools::tables`]. The DHCP clients of a side need their block so that
    /// the MAC address of a client is the one the tables hold.
    pub fn blocks(&self) -> Result<Blocks, PoolError> {
        let mut ledger = MacAllocator::new();
        let client = ledger.claim(&self.client.pool, self.client.range)?;
        let server = ledger.claim(&self.server.pool, self.server.range)?;
        Ok(Blocks { client, server })
    }

    /// The lease boards of the sides whose pool leases (TRN-37), empty for
    /// static pools.
    pub fn leases(&self) -> Result<Leases, PoolError> {
        let blocks = self.blocks()?;
        let board = |slice: &HostSlice, block: MacBlock| -> Result<_, PoolError> {
            slice
                .pool
                .is_dhcp()
                .then(|| LeaseBoard::new(slice.pool.clone(), block).map(Arc::new))
                .transpose()
                .map_err(PoolError::from)
        };
        Ok(Leases {
            client: board(&self.client, blocks.client)?,
            server: board(&self.server, blocks.server)?,
        })
    }

    /// Builds the station of each side from [`Pools::tables`], with the other
    /// side as its neighbor table.
    pub fn stations(&self) -> Result<Built, PoolError> {
        self.stations_with(&Leases::default())
    }

    /// Like [`Pools::stations`], with the stations of a DHCP side reading the
    /// addresses of that side from its lease board (TRN-37).
    pub fn stations_with(&self, leases: &Leases) -> Result<Built, PoolError> {
        let tables = self.tables()?;
        let station = |local: NeighborTable,
                       peers: NeighborTable,
                       own: &Option<Arc<LeaseBoard>>,
                       other: &Option<Arc<LeaseBoard>>|
         -> Result<Station, PoolError> {
            let mut s = Station::new(local, peers, self.vlan)?;
            if let Some(b) = own {
                s = s.with_local_leases(Arc::clone(b));
            }
            if let Some(b) = other {
                s = s.with_peer_leases(Arc::clone(b));
            }
            Ok(s)
        };
        Ok(Built {
            client: station(
                tables.client.clone(),
                tables.server.clone(),
                &leases.client,
                &leases.server,
            )?,
            server: station(tables.server, tables.client, &leases.server, &leases.client)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(pool: &HostSlice, i: u32) -> Ipv4Addr {
        Ipv4Addr::from(pool.address(i).expect("host in the slice"))
    }

    #[test]
    fn tables_give_each_host_one_distinct_mac_address() {
        let pools = Pools::counts(3, 2, None).expect("pools");
        let tables = pools.tables().expect("tables");
        let mut seen = std::collections::HashSet::new();
        for i in 0..3 {
            let mac = tables.client.lookup_v4(v4(&pools.client, i));
            assert!(seen.insert(mac.expect("client host")), "client host {i}");
        }
        for i in 0..2 {
            let mac = tables.server.lookup_v4(v4(&pools.server, i));
            assert!(seen.insert(mac.expect("server host")), "server host {i}");
        }
        assert_eq!(seen.len(), 5, "no MAC address is shared by two hosts");
        assert!(tables
            .client
            .lookup_v4(Ipv4Addr::new(172, 16, 0, 1))
            .is_none());
        assert!(
            tables.client.lookup_v4(v4(&pools.server, 0)).is_none(),
            "a side serves only its own hosts"
        );
    }

    #[test]
    fn stations_use_the_macs_of_the_tables() {
        for pools in [
            Pools::counts(4, 4, Some(100)).expect("pools"),
            Pools::sized_v6("fd00:1::/64", 4, "fd00:2::/64", 4, None).expect("pools"),
        ] {
            let tables = pools.tables().expect("tables");
            let built = pools.stations().expect("stations");
            for i in 0..4 {
                let client = pools.client.ip(i).expect("client host");
                let server = pools.server.ip(i).expect("server host");
                let (c, s) = match (client, server) {
                    (IpAddr::V4(c), IpAddr::V4(s)) => {
                        (tables.client.lookup_v4(c), tables.server.lookup_v4(s))
                    }
                    (IpAddr::V6(c), IpAddr::V6(s)) => {
                        (tables.client.lookup_v6(c), tables.server.lookup_v6(s))
                    }
                    _ => unreachable!("the pools of one setup have one family"),
                };
                assert!(c.is_some() && s.is_some());
                assert_eq!(built.client.local_mac(client), c);
                assert_eq!(built.server.local_mac(server), s);
                assert_eq!(built.client.resolve(server), s, "neighbor of the client");
                assert_eq!(built.server.resolve(client), c, "neighbor of the server");
            }
        }
    }

    #[test]
    fn tables_are_the_same_on_every_call() {
        let pools = Pools::counts(2, 2, None).expect("pools");
        let (a, b) = (pools.tables().expect("a"), pools.tables().expect("b"));
        let host = v4(&pools.server, 1);
        assert_eq!(a.server.lookup_v4(host), b.server.lookup_v4(host));
    }
}
