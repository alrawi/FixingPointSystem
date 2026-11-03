// SPDX-License-Identifier: GPL-3.0-only
//! Host pools and the mapping from host index to address.
//!
//! Host index `i` of a pool maps to the `i`-th address of its prefix, counting
//! from the first address after the network address and skipping the gateway,
//! and for IPv4 also skipping the broadcast address. Host index `i` uses the
//! MAC address `mac_base + i`, so addresses stay unique across agents.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use crate::slice::HostRange;

/// Errors that parsing a host pool or mapping a host index reports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PoolError {
    /// A prefix is not `address/length`, or its host bits are not zero.
    #[error("host pool {pool}: invalid {family} prefix '{value}': {reason}")]
    InvalidPrefix {
        /// The pool name.
        pool: String,
        /// `ipv4` or `ipv6`.
        family: &'static str,
        /// The text of the prefix.
        value: String,
        /// Why the prefix is invalid.
        reason: &'static str,
    },
    /// The gateway is not an IP address.
    #[error("host pool {pool}: invalid gateway '{value}'")]
    InvalidGateway {
        /// The pool name.
        pool: String,
        /// The text of the gateway.
        value: String,
    },
    /// The MAC base is not six colon separated hex octets.
    #[error("host pool {pool}: invalid mac_base '{value}'")]
    InvalidMac {
        /// The pool name.
        pool: String,
        /// The text of the MAC base.
        value: String,
    },
    /// The pool has neither an IPv4 nor an IPv6 prefix.
    #[error("host pool {pool}: no ipv4 or ipv6 prefix")]
    NoPrefix {
        /// The pool name.
        pool: String,
    },
    /// The pool has no hosts.
    #[error("host pool {pool}: count must be at least 1")]
    Empty {
        /// The pool name.
        pool: String,
    },
    /// A prefix has fewer usable addresses than the pool has hosts.
    #[error("host pool {pool}: {family} prefix has {capacity} usable addresses for {count} hosts")]
    TooSmall {
        /// The pool name.
        pool: String,
        /// `ipv4` or `ipv6`.
        family: &'static str,
        /// The host count of the pool.
        count: u32,
        /// The usable addresses of the prefix.
        capacity: u128,
    },
    /// The MAC addresses of the pool run past `ff:ff:ff:ff:ff:ff`.
    #[error("host pool {pool}: mac_base plus {count} hosts overflows 48 bits")]
    MacOverflow {
        /// The pool name.
        pool: String,
        /// The host count of the pool.
        count: u32,
    },
    /// A host index lies outside the pool.
    #[error("host pool {pool}: host index {index} outside pool of {count} hosts")]
    IndexOutOfRange {
        /// The pool name.
        pool: String,
        /// The requested host index.
        index: u32,
        /// The host count of the pool.
        count: u32,
    },
}

/// Why a prefix text is invalid, without the pool context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefixParseError(&'static str);

impl fmt::Display for PrefixParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for PrefixParseError {}

fn split_prefix(s: &str, max_len: u8) -> Result<(&str, u8), PrefixParseError> {
    let (addr, len) = s
        .split_once('/')
        .ok_or(PrefixParseError("missing prefix length"))?;
    let len: u8 = len
        .parse()
        .map_err(|_| PrefixParseError("prefix length is not a number"))?;
    if len > max_len {
        return Err(PrefixParseError("prefix length too large"));
    }
    Ok((addr, len))
}

/// An IPv4 prefix whose host bits are zero, such as `10.10.0.0/16`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ipv4Prefix {
    network: Ipv4Addr,
    len: u8,
}

impl Ipv4Prefix {
    /// Returns the prefix, or an error when `len` exceeds 32 or `network` has
    /// host bits set.
    pub fn new(network: Ipv4Addr, len: u8) -> Result<Self, PrefixParseError> {
        if len > 32 {
            return Err(PrefixParseError("prefix length too large"));
        }
        if u32::from(network) & !Self::mask(len) != 0 {
            return Err(PrefixParseError("host bits are not zero"));
        }
        Ok(Self { network, len })
    }

    fn mask(len: u8) -> u32 {
        u32::MAX.checked_shl(32 - u32::from(len)).unwrap_or(0)
    }

    /// The network address.
    pub fn network(&self) -> Ipv4Addr {
        self.network
    }

    /// The prefix length.
    pub fn prefix_len(&self) -> u8 {
        self.len
    }

    /// The broadcast address, which is the last address of the prefix.
    pub fn broadcast(&self) -> Ipv4Addr {
        Ipv4Addr::from(u32::from(self.network) | !Self::mask(self.len))
    }

    /// Reports whether `addr` lies inside the prefix.
    pub fn contains(&self, addr: Ipv4Addr) -> bool {
        u32::from(addr) & Self::mask(self.len) == u32::from(self.network)
    }

    /// The addresses strictly between the network and the broadcast address.
    fn inner(&self) -> u128 {
        (1u128 << (32 - u32::from(self.len))).saturating_sub(2)
    }
}

impl FromStr for Ipv4Prefix {
    type Err = PrefixParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (addr, len) = split_prefix(s, 32)?;
        let network = addr
            .parse::<Ipv4Addr>()
            .map_err(|_| PrefixParseError("address is not IPv4"))?;
        Self::new(network, len)
    }
}

impl fmt::Display for Ipv4Prefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.network, self.len)
    }
}

/// An IPv6 prefix whose host bits are zero, such as `2001:db8:10::/64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ipv6Prefix {
    network: Ipv6Addr,
    len: u8,
}

impl Ipv6Prefix {
    /// Returns the prefix, or an error when `len` exceeds 128 or `network` has
    /// host bits set.
    pub fn new(network: Ipv6Addr, len: u8) -> Result<Self, PrefixParseError> {
        if len > 128 {
            return Err(PrefixParseError("prefix length too large"));
        }
        if u128::from(network) & !Self::mask(len) != 0 {
            return Err(PrefixParseError("host bits are not zero"));
        }
        Ok(Self { network, len })
    }

    fn mask(len: u8) -> u128 {
        u128::MAX.checked_shl(128 - u32::from(len)).unwrap_or(0)
    }

    /// The network address.
    pub fn network(&self) -> Ipv6Addr {
        self.network
    }

    /// The prefix length.
    pub fn prefix_len(&self) -> u8 {
        self.len
    }

    /// Reports whether `addr` lies inside the prefix.
    pub fn contains(&self, addr: Ipv6Addr) -> bool {
        u128::from(addr) & Self::mask(self.len) == u128::from(self.network)
    }

    /// The addresses after the network address.
    fn inner(&self) -> u128 {
        !Self::mask(self.len)
    }
}

impl FromStr for Ipv6Prefix {
    type Err = PrefixParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (addr, len) = split_prefix(s, 128)?;
        let network = addr
            .parse::<Ipv6Addr>()
            .map_err(|_| PrefixParseError("address is not IPv6"))?;
        Self::new(network, len)
    }
}

impl fmt::Display for Ipv6Prefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.network, self.len)
    }
}

/// A 48 bit MAC address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    fn to_u64(self) -> u64 {
        self.0
            .iter()
            .fold(0u64, |acc, b| (acc << 8) | u64::from(*b))
    }

    fn from_u64(v: u64) -> Self {
        let b = v.to_be_bytes();
        Self([b[2], b[3], b[4], b[5], b[6], b[7]])
    }
}

impl FromStr for MacAddr {
    type Err = PrefixParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut out = [0u8; 6];
        let mut parts = s.split(':');
        for byte in &mut out {
            let part = parts.next().ok_or(PrefixParseError("too few octets"))?;
            if part.len() != 2 {
                return Err(PrefixParseError("octet is not two hex digits"));
            }
            *byte = u8::from_str_radix(part, 16)
                .map_err(|_| PrefixParseError("octet is not two hex digits"))?;
        }
        if parts.next().is_some() {
            return Err(PrefixParseError("too many octets"));
        }
        Ok(Self(out))
    }
}

impl fmt::Display for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = self.0;
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5]
        )
    }
}

/// The text fields of one host pool, as the `host_pool` definition of
/// `contracts/schemas/test-config.schema.json` and the `HostPoolSlice` message
/// carry them. Absent fields are `None`.
#[derive(Debug, Clone, Copy, Default)]
pub struct PoolSpec<'a> {
    /// The pool name.
    pub name: &'a str,
    /// The IPv4 prefix, such as `10.10.0.0/16`.
    pub ipv4: Option<&'a str>,
    /// The IPv6 prefix, such as `2001:db8::/64`.
    pub ipv6: Option<&'a str>,
    /// The number of hosts in the whole pool.
    pub count: u32,
    /// The gateway address, which host indexes skip.
    pub gateway: Option<&'a str>,
    /// The MAC address of host index 0.
    pub mac_base: Option<&'a str>,
}

/// A parsed host pool that maps host indexes to addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPool {
    name: String,
    ipv4: Option<Ipv4Prefix>,
    ipv6: Option<Ipv6Prefix>,
    count: u32,
    gateway: Option<IpAddr>,
    mac_base: Option<MacAddr>,
}

impl HostPool {
    /// Parses a pool and checks that every prefix holds `count` hosts and that
    /// the MAC range fits in 48 bits.
    pub fn parse(spec: &PoolSpec<'_>) -> Result<Self, PoolError> {
        let pool = spec.name.to_owned();
        let prefix_err = |family, value: &str, e: PrefixParseError| PoolError::InvalidPrefix {
            pool: pool.clone(),
            family,
            value: value.to_owned(),
            reason: e.0,
        };
        let ipv4 = spec
            .ipv4
            .map(|v| {
                v.parse::<Ipv4Prefix>()
                    .map_err(|e| prefix_err("ipv4", v, e))
            })
            .transpose()?;
        let ipv6 = spec
            .ipv6
            .map(|v| {
                v.parse::<Ipv6Prefix>()
                    .map_err(|e| prefix_err("ipv6", v, e))
            })
            .transpose()?;
        let gateway = spec
            .gateway
            .filter(|g| !g.is_empty())
            .map(|g| {
                g.parse::<IpAddr>().map_err(|_| PoolError::InvalidGateway {
                    pool: pool.clone(),
                    value: g.to_owned(),
                })
            })
            .transpose()?;
        let mac_base = spec
            .mac_base
            .filter(|m| !m.is_empty())
            .map(|m| {
                m.parse::<MacAddr>().map_err(|_| PoolError::InvalidMac {
                    pool: pool.clone(),
                    value: m.to_owned(),
                })
            })
            .transpose()?;
        let out = Self {
            name: pool,
            ipv4,
            ipv6,
            count: spec.count,
            gateway,
            mac_base,
        };
        out.check()?;
        Ok(out)
    }

    fn check(&self) -> Result<(), PoolError> {
        if self.ipv4.is_none() && self.ipv6.is_none() {
            return Err(PoolError::NoPrefix {
                pool: self.name.clone(),
            });
        }
        if self.count == 0 {
            return Err(PoolError::Empty {
                pool: self.name.clone(),
            });
        }
        for (family, capacity) in [
            ("ipv4", self.ipv4_capacity()),
            ("ipv6", self.ipv6_capacity()),
        ] {
            if let Some(capacity) = capacity {
                if capacity < u128::from(self.count) {
                    return Err(PoolError::TooSmall {
                        pool: self.name.clone(),
                        family,
                        count: self.count,
                        capacity,
                    });
                }
            }
        }
        if let Some(base) = self.mac_base {
            if base.to_u64() + u64::from(self.count) - 1 > 0xffff_ffff_ffff {
                return Err(PoolError::MacOverflow {
                    pool: self.name.clone(),
                    count: self.count,
                });
            }
        }
        Ok(())
    }

    fn gateway_v4(&self) -> Option<u32> {
        match (self.gateway, self.ipv4) {
            (Some(IpAddr::V4(g)), Some(p)) if p.contains(g) => Some(u32::from(g)),
            _ => None,
        }
    }

    fn gateway_v6(&self) -> Option<u128> {
        match (self.gateway, self.ipv6) {
            (Some(IpAddr::V6(g)), Some(p)) if p.contains(g) => Some(u128::from(g)),
            _ => None,
        }
    }

    /// The usable IPv4 addresses of the prefix, or `None` without IPv4.
    pub fn ipv4_capacity(&self) -> Option<u128> {
        let p = self.ipv4?;
        let net = u32::from(p.network());
        let skip = self
            .gateway_v4()
            .is_some_and(|g| g > net && g < u32::from(p.broadcast()));
        Some(p.inner() - u128::from(skip))
    }

    /// The usable IPv6 addresses of the prefix, or `None` without IPv6.
    pub fn ipv6_capacity(&self) -> Option<u128> {
        let p = self.ipv6?;
        let net = u128::from(p.network());
        let skip = self.gateway_v6().is_some_and(|g| g > net);
        Some(p.inner() - u128::from(skip))
    }

    /// The pool name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The number of hosts in the whole pool.
    pub fn count(&self) -> u32 {
        self.count
    }

    /// The IPv4 prefix, if any.
    pub fn ipv4(&self) -> Option<Ipv4Prefix> {
        self.ipv4
    }

    /// The IPv6 prefix, if any.
    pub fn ipv6(&self) -> Option<Ipv6Prefix> {
        self.ipv6
    }

    /// The gateway address, if the pool names one.
    pub fn gateway(&self) -> Option<IpAddr> {
        self.gateway
    }

    /// The range of every host index of the pool.
    pub fn range(&self) -> HostRange {
        HostRange::new(0, self.count)
    }

    fn check_index(&self, index: u32) -> Result<(), PoolError> {
        if index >= self.count {
            return Err(PoolError::IndexOutOfRange {
                pool: self.name.clone(),
                index,
                count: self.count,
            });
        }
        Ok(())
    }

    /// The IPv4 address of host `index`, or `None` when the pool has no IPv4
    /// prefix.
    pub fn ipv4_addr(&self, index: u32) -> Result<Option<Ipv4Addr>, PoolError> {
        self.check_index(index)?;
        let Some(p) = self.ipv4 else {
            return Ok(None);
        };
        // The capacity check in `parse` keeps the candidate below the
        // broadcast address, so the sum fits in 32 bits.
        let mut cand = u64::from(u32::from(p.network())) + 1 + u64::from(index);
        if let Some(g) = self.gateway_v4() {
            if u64::from(g) > u64::from(u32::from(p.network())) && u64::from(g) <= cand {
                cand += 1;
            }
        }
        Ok(u32::try_from(cand).ok().map(Ipv4Addr::from))
    }

    /// The IPv6 address of host `index`, or `None` when the pool has no IPv6
    /// prefix.
    pub fn ipv6_addr(&self, index: u32) -> Result<Option<Ipv6Addr>, PoolError> {
        self.check_index(index)?;
        let Some(p) = self.ipv6 else {
            return Ok(None);
        };
        let net = u128::from(p.network());
        // The capacity check in `parse` keeps the candidate inside the prefix.
        let mut cand = net + 1 + u128::from(index);
        if let Some(g) = self.gateway_v6() {
            if g > net && g <= cand {
                cand += 1;
            }
        }
        Ok(Some(Ipv6Addr::from(cand)))
    }

    /// The MAC address of host `index`, or `None` when the pool has no MAC base.
    pub fn mac(&self, index: u32) -> Result<Option<MacAddr>, PoolError> {
        self.check_index(index)?;
        Ok(self
            .mac_base
            .map(|b| MacAddr::from_u64(b.to_u64() + u64::from(index))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(ipv4: Option<&str>, ipv6: Option<&str>, count: u32, gw: Option<&str>) -> HostPool {
        HostPool::parse(&PoolSpec {
            name: "p",
            ipv4,
            ipv6,
            count,
            gateway: gw,
            mac_base: None,
        })
        .unwrap()
    }

    #[test]
    fn ipv4_index_zero_is_first_address_after_network() {
        let p = pool(Some("16.0.0.0/24"), None, 254, None);
        assert_eq!(p.ipv4_addr(0).unwrap(), Some(Ipv4Addr::new(16, 0, 0, 1)));
        assert_eq!(
            p.ipv4_addr(253).unwrap(),
            Some(Ipv4Addr::new(16, 0, 0, 254))
        );
        assert_eq!(p.ipv6_addr(0).unwrap(), None);
    }

    #[test]
    fn ipv4_skips_gateway() {
        let p = pool(Some("10.10.0.0/16"), None, 20000, Some("10.10.0.1"));
        assert_eq!(p.ipv4_addr(0).unwrap(), Some(Ipv4Addr::new(10, 10, 0, 2)));
        let p = pool(Some("10.0.0.0/29"), None, 5, Some("10.0.0.3"));
        let got: Vec<_> = (0..5).map(|i| p.ipv4_addr(i).unwrap().unwrap()).collect();
        let want: Vec<_> = [1, 2, 4, 5, 6].map(|d| Ipv4Addr::new(10, 0, 0, d)).to_vec();
        assert_eq!(got, want);
    }

    #[test]
    fn ipv4_capacity_excludes_network_broadcast_and_gateway() {
        let spec = |count, gw| PoolSpec {
            name: "p",
            ipv4: Some("10.0.0.0/29"),
            count,
            gateway: gw,
            ..PoolSpec::default()
        };
        assert!(HostPool::parse(&spec(6, None)).is_ok());
        assert!(matches!(
            HostPool::parse(&spec(7, None)),
            Err(PoolError::TooSmall { capacity: 6, .. })
        ));
        assert!(matches!(
            HostPool::parse(&spec(6, Some("10.0.0.6"))),
            Err(PoolError::TooSmall { capacity: 5, .. })
        ));
        // A gateway outside the prefix costs no address.
        assert!(HostPool::parse(&spec(6, Some("192.0.2.1"))).is_ok());
        let tiny = PoolSpec {
            ipv4: Some("10.0.0.0/31"),
            ..spec(1, None)
        };
        assert!(matches!(
            HostPool::parse(&tiny),
            Err(PoolError::TooSmall { capacity: 0, .. })
        ));
    }

    #[test]
    fn ipv6_uses_last_address_and_skips_gateway() {
        let p = pool(None, Some("2001:db8::/125"), 6, Some("2001:db8::1"));
        assert_eq!(
            p.ipv6_addr(0).unwrap(),
            Some("2001:db8::2".parse().unwrap())
        );
        assert_eq!(
            p.ipv6_addr(5).unwrap(),
            Some("2001:db8::7".parse().unwrap())
        );
        assert!(HostPool::parse(&PoolSpec {
            name: "p",
            ipv6: Some("2001:db8::/125"),
            count: 7,
            gateway: Some("2001:db8::1"),
            ..PoolSpec::default()
        })
        .is_err());
    }

    #[test]
    fn dual_stack_pool_checks_both_families() {
        let p = pool(Some("10.0.0.0/24"), Some("2001:db8::/120"), 254, None);
        assert_eq!(p.ipv4_addr(9).unwrap(), Some(Ipv4Addr::new(10, 0, 0, 10)));
        assert_eq!(
            p.ipv6_addr(9).unwrap(),
            Some("2001:db8::a".parse().unwrap())
        );
        let err = HostPool::parse(&PoolSpec {
            name: "p",
            ipv4: Some("10.0.0.0/24"),
            ipv6: Some("2001:db8::/125"),
            count: 100,
            ..PoolSpec::default()
        })
        .unwrap_err();
        assert!(matches!(err, PoolError::TooSmall { family: "ipv6", .. }));
    }

    #[test]
    fn rejects_bad_input_with_pool_context() {
        let base = PoolSpec {
            name: "users",
            count: 1,
            ..PoolSpec::default()
        };
        assert!(matches!(
            HostPool::parse(&base),
            Err(PoolError::NoPrefix { .. })
        ));
        for bad in [
            "10.0.0.1/24",
            "10.0.0.0",
            "10.0.0.0/33",
            "::/8",
            "10.0.0.0/x",
        ] {
            let err = HostPool::parse(&PoolSpec {
                ipv4: Some(bad),
                ..base
            })
            .unwrap_err();
            assert!(err
                .to_string()
                .starts_with("host pool users: invalid ipv4 prefix"));
        }
        assert!(matches!(
            HostPool::parse(&PoolSpec {
                ipv4: Some("10.0.0.0/24"),
                count: 0,
                ..base
            }),
            Err(PoolError::Empty { .. })
        ));
        assert!(matches!(
            HostPool::parse(&PoolSpec {
                ipv4: Some("10.0.0.0/24"),
                gateway: Some("gw"),
                ..base
            }),
            Err(PoolError::InvalidGateway { .. })
        ));
        let p = pool(Some("10.0.0.0/24"), None, 3, None);
        assert!(matches!(
            p.ipv4_addr(3),
            Err(PoolError::IndexOutOfRange { index: 3, .. })
        ));
    }

    #[test]
    fn mac_adds_index_to_base() {
        let p = HostPool::parse(&PoolSpec {
            name: "p",
            ipv4: Some("16.0.0.0/24"),
            count: 254,
            mac_base: Some("02:00:10:00:00:ff"),
            ..PoolSpec::default()
        })
        .unwrap();
        assert_eq!(p.mac(1).unwrap().unwrap().to_string(), "02:00:10:00:01:00");
        let overflow = HostPool::parse(&PoolSpec {
            name: "p",
            ipv4: Some("16.0.0.0/24"),
            count: 2,
            mac_base: Some("ff:ff:ff:ff:ff:ff"),
            ..PoolSpec::default()
        });
        assert!(matches!(overflow, Err(PoolError::MacOverflow { .. })));
        for bad in [
            "02:00:10:00:00",
            "02:00:10:00:00:00:00",
            "2:0:10:0:0:1",
            "zz:00:00:00:00:00",
        ] {
            assert!(bad.parse::<MacAddr>().is_err(), "{bad}");
        }
    }
}
