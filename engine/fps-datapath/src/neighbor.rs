// SPDX-License-Identifier: GPL-3.0-only
//! The neighbor cache of a queue: next hop resolution for the frames of the
//! stack (TRN-35).
//!
//! A stack that [defers](fps_tcp_own::net::Station::defer_peers) its peers
//! writes [`NEXT_HOP_MAC`] in the Ethernet destination of every frame to a
//! peer. The cache sees the frame as it leaves the splitter and does what an
//! IP host does:
//!
//! 1. It picks the next hop of the destination address ([`Route`]): the
//!    destination itself when it lies in the prefix of the local pool, and
//!    otherwise the gateway of the pool. A pool without a gateway resolves the
//!    destination itself, which is right on a flat link and with proxy ARP.
//! 2. It looks the next hop up. A fresh entry gives the MAC address, and the
//!    frame leaves with it.
//! 3. Without an entry it sends an ARP request (IPv4, RFC 826) or a neighbor
//!    solicitation (IPv6, RFC 4861) from the host that wants to send, and
//!    holds the frame. The hold is bounded per next hop and in total, and the
//!    oldest frame goes first when it is full, as the queue of the kernel
//!    does.
//! 4. It sends the request again every [`NeighborConfig::retry_interval`] and
//!    gives up after [`NeighborConfig::max_requests`]. The held frames are
//!    dropped, and the next hop is not asked for again for
//!    [`NeighborConfig::failure_hold`], so a dead gateway costs one round of
//!    requests and not one per frame.
//! 5. An ARP reply, a neighbor advertisement, and a request from a next hop
//!    that has an entry give the MAC address. The held frames leave, and the
//!    entry lives for [`NeighborConfig::lifetime`] and is asked for again
//!    after that.
//!
//! The cache creates an entry only for a frame of the stack, so a stranger
//! cannot make it grow with unsolicited replies.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use fps_hosts::wire::{arp_request, ethernet_payload, ns_frame, ETHERTYPE_IPV4, ETHERTYPE_IPV6};
use fps_hosts::{HostPool, Ipv4Prefix, Ipv6Prefix, LeaseTable, MacAddr};
use fps_tcp_own::clock::Clock;
use fps_tcp_own::net::NEXT_HOP_MAC;
use fps_tcp_own::Follower;

use crate::split::SharedCounters;

/// Settings of the neighbor cache of a queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeighborConfig {
    /// How long a resolved entry is used before the next hop is asked again.
    /// The kernel keeps an entry reachable for about 30 seconds and probes
    /// it afterwards; this cache asks again, and holds the frames meanwhile.
    pub lifetime: Duration,
    /// How long the cache waits for an answer before it asks again. RFC 4861
    /// has 1 second, and so does the kernel.
    pub retry_interval: Duration,
    /// The requests sent for one resolution, the first included, before the
    /// cache gives up. RFC 4861 has 3 solicitations, the kernel 3 probes.
    pub max_requests: u32,
    /// How long a next hop that did not answer is not asked again. Frames to
    /// it are dropped meanwhile.
    pub failure_hold: Duration,
    /// The frames held for one next hop that is being resolved.
    pub max_held_per_hop: usize,
    /// The frames held for all next hops of the queue.
    pub max_held_total: usize,
    /// The next hops that the cache knows at most. A destination on the local
    /// prefix is a next hop of its own, so this bounds the memory of a large
    /// flat link.
    pub max_entries: usize,
}

impl Default for NeighborConfig {
    /// A lifetime of 60 seconds, a retry every second, 3 requests, 3 seconds
    /// of silence after a failure, 32 held frames per next hop and 1,024 in
    /// all, and 65,536 entries.
    fn default() -> Self {
        Self {
            lifetime: Duration::from_secs(60),
            retry_interval: Duration::from_secs(1),
            max_requests: 3,
            failure_hold: Duration::from_secs(3),
            max_held_per_hop: 32,
            max_held_total: 1024,
            max_entries: 65_536,
        }
    }
}

/// Where the frames of a queue go: the prefixes of the local pool and its
/// gateway. [`Route::of`] reads them from a host pool, and the gateway of
/// the pool comes from the plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Route {
    /// The IPv4 prefix of the local hosts.
    pub ipv4: Option<Ipv4Prefix>,
    /// The IPv6 prefix of the local hosts.
    pub ipv6: Option<Ipv6Prefix>,
    /// The IPv4 gateway, or `None` when the pool names none.
    pub gateway_v4: Option<Ipv4Addr>,
    /// The IPv6 gateway, or `None` when the pool names none.
    pub gateway_v6: Option<Ipv6Addr>,
}

impl Route {
    /// The route of the hosts that lease their addresses (TRN-37): the subnet
    /// of the first lease whose server sent a mask, and the router of the
    /// first lease that names one. The hosts of an arm share a link, so one
    /// subnet and one router serve the arm; a lease that disagrees is not
    /// followed.
    #[must_use]
    pub fn of_leases(leases: &LeaseTable) -> Self {
        Self {
            ipv4: leases.subnets().into_iter().next(),
            ipv6: None,
            gateway_v4: leases.iter().find_map(|(_, l)| l.router),
            gateway_v6: None,
        }
    }

    /// The route of the hosts of `pool`: its prefixes and its gateway, which
    /// serves the family that it belongs to.
    #[must_use]
    pub fn of(pool: &HostPool) -> Self {
        Self {
            ipv4: pool.ipv4(),
            ipv6: pool.ipv6(),
            gateway_v4: match pool.gateway() {
                Some(IpAddr::V4(g)) => Some(g),
                _ => None,
            },
            gateway_v6: match pool.gateway() {
                Some(IpAddr::V6(g)) => Some(g),
                _ => None,
            },
        }
    }

    /// The next hop of a packet to `destination`: the destination when it is
    /// on the local prefix (or a link local IPv6 address), the gateway of the
    /// family when it is not, and the destination again when there is no
    /// gateway.
    #[must_use]
    pub fn next_hop(&self, destination: IpAddr) -> IpAddr {
        match destination {
            IpAddr::V4(d) => {
                if self.ipv4.is_some_and(|p| p.contains(d)) {
                    return destination;
                }
                self.gateway_v4.map_or(destination, IpAddr::V4)
            }
            IpAddr::V6(d) => {
                let link_local = d.segments()[0] & 0xffc0 == 0xfe80;
                if link_local || self.ipv6.is_some_and(|p| p.contains(d)) {
                    return destination;
                }
                self.gateway_v6.map_or(destination, IpAddr::V6)
            }
        }
    }
}

/// What the cache did with a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Send the frame to this MAC address.
    Send(MacAddr),
    /// The cache holds the frame now.
    Held,
    /// The cache dropped the frame.
    Dropped,
}

/// The source of a frame, which the request for its next hop is sent from.
#[derive(Clone, Copy, Debug)]
struct Source {
    mac: MacAddr,
    ip: IpAddr,
}

enum Entry {
    Resolving {
        held: VecDeque<Vec<u8>>,
        source: Source,
        requests: u32,
        next_at: i64,
    },
    Reachable {
        mac: MacAddr,
        expires: i64,
    },
    Failed {
        until: i64,
    },
}

/// The addresses of an IP frame of the stack.
struct Addresses {
    source: Source,
    destination: IpAddr,
}

fn mac_at(frame: &[u8], at: usize) -> Option<MacAddr> {
    Some(MacAddr(frame.get(at..at + 6)?.try_into().ok()?))
}

/// Reads the source MAC address and the two IP addresses of an Ethernet frame
/// that carries IPv4 or IPv6, or returns `None` for anything else.
fn addresses(frame: &[u8]) -> Option<Addresses> {
    let (pos, ethertype, _, _) = ethernet_payload(frame)?;
    let ip = frame.get(pos..)?;
    let (src, dst) = match ethertype {
        ETHERTYPE_IPV4 if ip.first()? >> 4 == 4 => (
            IpAddr::V4(Ipv4Addr::from(<[u8; 4]>::try_from(ip.get(12..16)?).ok()?)),
            IpAddr::V4(Ipv4Addr::from(<[u8; 4]>::try_from(ip.get(16..20)?).ok()?)),
        ),
        ETHERTYPE_IPV6 if ip.first()? >> 4 == 6 => (
            IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(ip.get(8..24)?).ok()?)),
            IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(ip.get(24..40)?).ok()?)),
        ),
        _ => return None,
    };
    Some(Addresses {
        source: Source {
            mac: mac_at(frame, 6)?,
            ip: src,
        },
        destination: dst,
    })
}

/// Whether the frame waits for its next hop: it carries [`NEXT_HOP_MAC`].
pub(crate) fn needs_next_hop(frame: &[u8]) -> bool {
    frame.get(..6) == Some(&NEXT_HOP_MAC.0[..])
}

/// A MAC address that a neighbor can have: not zero and not a group address.
fn usable(mac: MacAddr) -> bool {
    mac.0 != [0; 6] && mac.0[0] & 1 == 0
}

/// The neighbor cache of one queue.
pub(crate) struct Neighbors {
    config: NeighborConfig,
    route: Route,
    vlans: Vec<u16>,
    clock: Arc<dyn Clock>,
    counters: Arc<SharedCounters>,
    entries: HashMap<IpAddr, Entry>,
    /// The next hops in state `Resolving`, which the timer pass visits.
    resolving: Vec<IpAddr>,
    held: usize,
    /// The leases that the route follows, with the generation it was built at.
    leases: Option<(Follower, u64)>,
}

fn micros(d: Duration) -> i64 {
    i64::try_from(d.as_micros()).unwrap_or(i64::MAX)
}

impl Neighbors {
    pub(crate) fn new(
        config: NeighborConfig,
        route: Route,
        vlan: Option<u16>,
        clock: Arc<dyn Clock>,
        counters: Arc<SharedCounters>,
    ) -> Self {
        Self {
            config,
            route,
            vlans: vlan.into_iter().collect(),
            clock,
            counters,
            entries: HashMap::new(),
            resolving: Vec::new(),
            held: 0,
            leases: None,
        }
    }

    /// Makes the route follow the leases of `follower` (TRN-37). The route
    /// is built from the leases at once and again whenever their generation
    /// moves. Entries that were resolved stay: they are keyed by next hop.
    pub(crate) fn follow(&mut self, follower: Follower) {
        self.route = Route::of_leases(follower.current().leases());
        let seen = follower.current().generation();
        self.leases = Some((follower, seen));
    }

    /// Rebuilds the route when the leases moved.
    pub(crate) fn sync(&mut self) {
        let Some((follower, seen)) = self.leases.as_mut() else {
            return;
        };
        if follower.board().generation() == *seen {
            return;
        }
        let snapshot = follower.current();
        *seen = snapshot.generation();
        self.route = Route::of_leases(snapshot.leases());
    }

    fn bump(&self, counter: &std::sync::atomic::AtomicU64, by: usize) {
        counter.fetch_add(by as u64, Ordering::Relaxed);
    }

    fn publish_entries(&self) {
        self.counters
            .neighbor_entries
            .store(self.entries.len() as u64, Ordering::Relaxed);
    }

    /// The request for `hop`, sent from `source`, or `None` when the families
    /// of the two addresses differ.
    fn request(&self, source: Source, hop: IpAddr) -> Option<Vec<u8>> {
        match (source.ip, hop) {
            (IpAddr::V4(s), IpAddr::V4(h)) => Some(arp_request(source.mac, s, h, &self.vlans)),
            (IpAddr::V6(s), IpAddr::V6(h)) => Some(ns_frame(source.mac, s, h, &self.vlans)),
            _ => None,
        }
    }

    /// Drops the entries that expired, so a full table can take a new one.
    fn prune(&mut self, now: i64) {
        let before = self.entries.len();
        self.entries.retain(|_, e| match e {
            Entry::Reachable { expires, .. } => now < *expires,
            Entry::Failed { until } => now < *until,
            Entry::Resolving { .. } => true,
        });
        let gone = before - self.entries.len();
        self.bump(&self.counters.neighbor_expired, gone);
    }

    /// The MAC address of the next hop of `frame` when the entry is fresh, and
    /// `None` when the cache has to ask or wait. It changes nothing.
    pub(crate) fn cached(&self, frame: &[u8]) -> Option<MacAddr> {
        let found = addresses(frame)?;
        let hop = self.route.next_hop(found.destination);
        match self.entries.get(&hop) {
            Some(Entry::Reachable { mac, expires }) if self.clock.now_micros() < *expires => {
                Some(*mac)
            }
            _ => None,
        }
    }

    /// Decides what happens to a frame that carries [`NEXT_HOP_MAC`]. A
    /// request that the decision needs goes to `requests`.
    ///
    /// The cache takes the frame when it holds it, so the caller passes it by
    /// value only when the cache may keep it: `frame` is copied for a hold.
    pub(crate) fn route_frame(
        &mut self,
        frame: &[u8],
        requests: &mut VecDeque<Vec<u8>>,
    ) -> Outcome {
        let now = self.clock.now_micros();
        let Some(found) = addresses(frame) else {
            self.bump(&self.counters.neighbor_dropped, 1);
            return Outcome::Dropped;
        };
        let hop = self.route.next_hop(found.destination);
        match self.entries.get(&hop) {
            Some(Entry::Reachable { mac, expires }) if now < *expires => {
                return Outcome::Send(*mac);
            }
            Some(Entry::Resolving { .. }) => return self.hold(hop, frame),
            Some(Entry::Failed { until }) if now < *until => {
                self.bump(&self.counters.neighbor_dropped, 1);
                return Outcome::Dropped;
            }
            Some(_) => {
                // An entry that ran out: ask again.
                self.entries.remove(&hop);
                self.bump(&self.counters.neighbor_expired, 1);
            }
            None => {}
        }
        if self.entries.len() >= self.config.max_entries {
            self.prune(now);
            if self.entries.len() >= self.config.max_entries {
                self.bump(&self.counters.neighbor_dropped, 1);
                self.bump(&self.counters.neighbor_table_full, 1);
                return Outcome::Dropped;
            }
        }
        let Some(request) = self.request(found.source, hop) else {
            self.bump(&self.counters.neighbor_dropped, 1);
            return Outcome::Dropped;
        };
        self.entries.insert(
            hop,
            Entry::Resolving {
                held: VecDeque::new(),
                source: found.source,
                requests: 1,
                next_at: now.saturating_add(micros(self.config.retry_interval)),
            },
        );
        self.resolving.push(hop);
        requests.push_back(request);
        self.bump(&self.counters.neighbor_requests, 1);
        self.publish_entries();
        self.hold(hop, frame)
    }

    /// Adds `frame` to the frames held for `hop`, within the bounds.
    fn hold(&mut self, hop: IpAddr, frame: &[u8]) -> Outcome {
        let (per_hop, total) = (self.config.max_held_per_hop, self.config.max_held_total);
        let mut dropped = 0usize;
        let mut added = false;
        if let Some(Entry::Resolving { held, .. }) = self.entries.get_mut(&hop) {
            if per_hop == 0 {
                dropped += 1;
            } else {
                if held.len() >= per_hop {
                    held.pop_front();
                    dropped += 1;
                    self.held = self.held.saturating_sub(1);
                }
                if self.held >= total {
                    // The queue as a whole is full: this frame goes.
                    dropped += 1;
                } else {
                    held.push_back(frame.to_vec());
                    self.held += 1;
                    added = true;
                }
            }
        }
        self.bump(&self.counters.neighbor_dropped, dropped);
        if added {
            self.bump(&self.counters.neighbor_held, 1);
            Outcome::Held
        } else {
            Outcome::Dropped
        }
    }

    /// Runs the timers: asks again where the answer is late, and gives up
    /// where the requests ran out. Requests go to `requests`.
    pub(crate) fn poll(&mut self, requests: &mut VecDeque<Vec<u8>>) {
        if self.resolving.is_empty() {
            return;
        }
        let now = self.clock.now_micros();
        let mut keep = Vec::with_capacity(self.resolving.len());
        for hop in std::mem::take(&mut self.resolving) {
            let Some(Entry::Resolving {
                held,
                source,
                requests: sent,
                next_at,
            }) = self.entries.get_mut(&hop)
            else {
                continue;
            };
            if now < *next_at {
                keep.push(hop);
                continue;
            }
            if *sent >= self.config.max_requests {
                let lost = held.len();
                self.held = self.held.saturating_sub(lost);
                self.entries.insert(
                    hop,
                    Entry::Failed {
                        until: now.saturating_add(micros(self.config.failure_hold)),
                    },
                );
                self.counters
                    .neighbor_failed
                    .fetch_add(1, Ordering::Relaxed);
                self.counters
                    .neighbor_dropped
                    .fetch_add(lost as u64, Ordering::Relaxed);
                continue;
            }
            *sent += 1;
            *next_at = now.saturating_add(micros(self.config.retry_interval));
            let source = *source;
            if let Some(request) = self.request(source, hop) {
                requests.push_back(request);
                self.counters
                    .neighbor_requests
                    .fetch_add(1, Ordering::Relaxed);
            }
            keep.push(hop);
        }
        self.resolving = keep;
        self.publish_entries();
    }

    /// Takes an answer: `mac` is the MAC address of `hop`. A resolution that
    /// waits finishes, and its held frames go to `released` with the MAC
    /// address in place. A known entry is refreshed, and a stranger that no
    /// frame asked for is ignored.
    pub(crate) fn learn(&mut self, hop: IpAddr, mac: MacAddr, released: &mut VecDeque<Vec<u8>>) {
        if !usable(mac) {
            return;
        }
        let now = self.clock.now_micros();
        let expires = now.saturating_add(micros(self.config.lifetime));
        match self.entries.get_mut(&hop) {
            Some(Entry::Resolving { held, .. }) => {
                let frames = std::mem::take(held);
                self.held = self.held.saturating_sub(frames.len());
                self.counters
                    .neighbor_released
                    .fetch_add(frames.len() as u64, Ordering::Relaxed);
                for mut frame in frames {
                    if let Some(dst) = frame.get_mut(..6) {
                        dst.copy_from_slice(&mac.0);
                    }
                    released.push_back(frame);
                }
                self.entries.insert(hop, Entry::Reachable { mac, expires });
                self.counters
                    .neighbor_resolved
                    .fetch_add(1, Ordering::Relaxed);
            }
            Some(entry @ (Entry::Reachable { .. } | Entry::Failed { .. })) => {
                let revived = matches!(entry, Entry::Failed { .. });
                *entry = Entry::Reachable { mac, expires };
                let counter = if revived {
                    &self.counters.neighbor_resolved
                } else {
                    &self.counters.neighbor_refreshed
                };
                counter.fetch_add(1, Ordering::Relaxed);
            }
            None => {}
        }
        self.publish_entries();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use fps_hosts::wire::{parse_arp, ARP_REQUEST, ICMP6_NS};
    use fps_hosts::{HostPool, PoolSpec};
    use fps_tcp_own::clock::ManualClock;

    const US: i64 = 1_000_000;
    const OWN_MAC: MacAddr = MacAddr([0x02, 0xaa, 0, 0, 0, 1]);
    const GW_MAC: MacAddr = MacAddr([0x02, 0xbb, 0, 0, 0, 9]);

    fn pool(v4: Option<&str>, v6: Option<&str>, gateway: Option<&str>) -> HostPool {
        HostPool::parse(&PoolSpec {
            name: "c",
            ipv4: v4,
            ipv6: v6,
            count: 4,
            gateway,
            ..PoolSpec::default()
        })
        .expect("pool")
    }

    struct Rig {
        cache: Neighbors,
        clock: Arc<ManualClock>,
        counters: Arc<SharedCounters>,
        out: VecDeque<Vec<u8>>,
    }

    fn rig(config: NeighborConfig, route: Route, vlan: Option<u16>) -> Rig {
        let clock = Arc::new(ManualClock::new());
        let counters = Arc::new(SharedCounters::default());
        let cache = Neighbors::new(
            config,
            route,
            vlan,
            Arc::clone(&clock) as Arc<dyn Clock>,
            Arc::clone(&counters),
        );
        Rig {
            cache,
            clock,
            counters,
            out: VecDeque::new(),
        }
    }

    fn v4_route() -> Route {
        Route::of(&pool(Some("10.0.0.0/24"), None, Some("10.0.0.1")))
    }

    /// An IPv4 frame of the stack, with the marker as its destination.
    fn frame_v4(src: [u8; 4], dst: [u8; 4], vlan: Option<u16>) -> Vec<u8> {
        let mut f = NEXT_HOP_MAC.0.to_vec();
        f.extend_from_slice(&OWN_MAC.0);
        if let Some(id) = vlan {
            f.extend_from_slice(&[0x81, 0x00]);
            f.extend_from_slice(&id.to_be_bytes());
        }
        f.extend_from_slice(&[0x08, 0x00, 0x45, 0, 0, 40, 0, 0, 0, 0, 64, 6, 0, 0]);
        f.extend_from_slice(&src);
        f.extend_from_slice(&dst);
        f.resize(60, 0);
        f
    }

    fn frame_v6(src: Ipv6Addr, dst: Ipv6Addr) -> Vec<u8> {
        let mut f = NEXT_HOP_MAC.0.to_vec();
        f.extend_from_slice(&OWN_MAC.0);
        f.extend_from_slice(&[0x86, 0xdd, 0x60, 0, 0, 0, 0, 20, 6, 64]);
        f.extend_from_slice(&src.octets());
        f.extend_from_slice(&dst.octets());
        f.resize(74, 0);
        f
    }

    const HOST: [u8; 4] = [10, 0, 0, 2];
    const GATEWAY: [u8; 4] = [10, 0, 0, 1];
    const FAR: [u8; 4] = [10, 9, 0, 5];

    #[test]
    fn the_next_hop_is_the_destination_on_the_link_and_the_gateway_beyond_it() {
        let route = v4_route();
        let on_link = IpAddr::from([10, 0, 0, 77]);
        let far = IpAddr::from(FAR);
        assert_eq!(route.next_hop(on_link), on_link);
        assert_eq!(route.next_hop(far), IpAddr::from(GATEWAY));

        let flat = Route::of(&pool(Some("10.0.0.0/24"), None, None));
        assert_eq!(flat.next_hop(far), far, "no gateway: the peer itself");

        let route6 = Route::of(&pool(None, Some("fd00:1::/64"), Some("fd00:1::1")));
        let inside: Ipv6Addr = "fd00:1::55".parse().unwrap();
        let outside: Ipv6Addr = "fd00:9::5".parse().unwrap();
        let link_local: Ipv6Addr = "fe80::1".parse().unwrap();
        assert_eq!(route6.next_hop(inside.into()), IpAddr::from(inside));
        assert_eq!(
            route6.next_hop(outside.into()),
            IpAddr::from("fd00:1::1".parse::<Ipv6Addr>().unwrap())
        );
        assert_eq!(route6.next_hop(link_local.into()), IpAddr::from(link_local));
        // A gateway of the other family does not serve this one.
        let mixed = Route::of(&pool(
            Some("10.0.0.0/24"),
            Some("fd00:1::/64"),
            Some("10.0.0.1"),
        ));
        assert_eq!(mixed.next_hop(outside.into()), IpAddr::from(outside));
    }

    #[test]
    fn a_frame_to_an_unknown_gateway_is_held_while_an_arp_request_asks_for_it() {
        let mut r = rig(NeighborConfig::default(), v4_route(), Some(7));
        let frame = frame_v4(HOST, FAR, Some(7));
        assert_eq!(r.cache.route_frame(&frame, &mut r.out), Outcome::Held);
        assert_eq!(r.out.len(), 1, "one request");
        let ask = parse_arp(r.out.front().unwrap()).expect("an ARP frame");
        assert_eq!(ask.op, ARP_REQUEST);
        assert_eq!(ask.sender_mac, OWN_MAC, "from the host that sends");
        assert_eq!(ask.sender_ip, Ipv4Addr::from(HOST));
        assert_eq!(ask.target_ip, Ipv4Addr::from(GATEWAY), "for the gateway");
        assert_eq!(ask.vlan_tags, 1, "on the VLAN of the port");
        assert_eq!(ask.vlans[0], 7);

        // A second frame joins the resolution and sends no second request.
        let second = frame_v4(HOST, [10, 8, 0, 8], Some(7));
        assert_eq!(r.cache.route_frame(&second, &mut r.out), Outcome::Held);
        assert_eq!(r.out.len(), 1);
        assert_eq!(r.cache.cached(&frame), None);

        // The answer releases both, in order, with the MAC address in place.
        let mut released = VecDeque::new();
        r.cache.learn(IpAddr::from(GATEWAY), GW_MAC, &mut released);
        assert_eq!(released.len(), 2);
        assert_eq!(released[0][..6], GW_MAC.0);
        assert_eq!(released[0][6..], frame[6..], "nothing else changed");
        assert_eq!(released[1][22..], second[22..]);
        assert_eq!(r.cache.cached(&frame), Some(GW_MAC));
        assert_eq!(
            r.cache.route_frame(&second, &mut r.out),
            Outcome::Send(GW_MAC)
        );
        let c = r.counters.snapshot();
        assert_eq!(
            (c.neighbor_requests, c.neighbor_resolved, c.neighbor_held),
            (1, 1, 2)
        );
        assert_eq!(
            (c.neighbor_released, c.neighbor_failed, c.neighbor_dropped),
            (2, 0, 0)
        );
        assert_eq!(c.neighbor_entries, 1);
    }

    #[test]
    fn a_destination_on_the_link_is_asked_for_itself() {
        let mut r = rig(NeighborConfig::default(), v4_route(), None);
        let frame = frame_v4(HOST, [10, 0, 0, 33], None);
        assert_eq!(r.cache.route_frame(&frame, &mut r.out), Outcome::Held);
        let ask = parse_arp(r.out.front().unwrap()).unwrap();
        assert_eq!(ask.target_ip, Ipv4Addr::new(10, 0, 0, 33));
    }

    #[test]
    fn an_unanswered_request_is_repeated_and_then_the_resolution_fails() {
        let config = NeighborConfig {
            retry_interval: Duration::from_secs(1),
            max_requests: 3,
            failure_hold: Duration::from_secs(5),
            ..NeighborConfig::default()
        };
        let mut r = rig(config, v4_route(), None);
        let frame = frame_v4(HOST, FAR, None);
        assert_eq!(r.cache.route_frame(&frame, &mut r.out), Outcome::Held);
        r.out.clear();

        r.clock.advance_micros(US / 2);
        r.cache.poll(&mut r.out);
        assert!(r.out.is_empty(), "too early to repeat");

        r.clock.advance_micros(US / 2);
        r.cache.poll(&mut r.out);
        assert_eq!(r.out.len(), 1, "second request");
        r.clock.advance_micros(US);
        r.cache.poll(&mut r.out);
        assert_eq!(r.out.len(), 2, "third request");
        let c = r.counters.snapshot();
        assert_eq!((c.neighbor_requests, c.neighbor_failed), (3, 0));

        // After the last request one more interval passes, then it gives up.
        r.clock.advance_micros(US);
        r.cache.poll(&mut r.out);
        assert_eq!(r.out.len(), 2, "no fourth request");
        let c = r.counters.snapshot();
        assert_eq!(
            (c.neighbor_failed, c.neighbor_dropped),
            (1, 1),
            "the held frame"
        );

        // The next hop is not asked for again until the failure hold passes.
        assert_eq!(r.cache.route_frame(&frame, &mut r.out), Outcome::Dropped);
        assert_eq!(r.out.len(), 2);
        assert_eq!(r.counters.snapshot().neighbor_dropped, 2);
        r.clock.advance_micros(5 * US);
        assert_eq!(r.cache.route_frame(&frame, &mut r.out), Outcome::Held);
        assert_eq!(r.out.len(), 3, "asked again");
        assert_eq!(r.counters.snapshot().neighbor_expired, 1);
    }

    #[test]
    fn a_late_answer_after_a_failure_still_gives_the_address() {
        let mut r = rig(NeighborConfig::default(), v4_route(), None);
        let frame = frame_v4(HOST, FAR, None);
        r.cache.route_frame(&frame, &mut r.out);
        for _ in 0..4 {
            r.clock.advance_micros(US);
            r.cache.poll(&mut r.out);
        }
        assert_eq!(r.counters.snapshot().neighbor_failed, 1);
        let mut released = VecDeque::new();
        r.cache.learn(IpAddr::from(GATEWAY), GW_MAC, &mut released);
        assert!(released.is_empty(), "the held frame is gone");
        assert_eq!(r.cache.cached(&frame), Some(GW_MAC));
        assert_eq!(r.counters.snapshot().neighbor_resolved, 1);
    }

    #[test]
    fn an_entry_ages_out_and_is_asked_for_again() {
        let config = NeighborConfig {
            lifetime: Duration::from_secs(10),
            ..NeighborConfig::default()
        };
        let mut r = rig(config, v4_route(), None);
        let frame = frame_v4(HOST, FAR, None);
        r.cache.route_frame(&frame, &mut r.out);
        r.cache
            .learn(IpAddr::from(GATEWAY), GW_MAC, &mut VecDeque::new());
        r.out.clear();

        r.clock.advance_micros(9 * US);
        assert_eq!(
            r.cache.route_frame(&frame, &mut r.out),
            Outcome::Send(GW_MAC)
        );
        // An answer refreshes the entry, and the lifetime starts again.
        let other = MacAddr([0x02, 0xbb, 0, 0, 0, 10]);
        r.cache
            .learn(IpAddr::from(GATEWAY), other, &mut VecDeque::new());
        assert_eq!(r.counters.snapshot().neighbor_refreshed, 1);
        r.clock.advance_micros(9 * US);
        assert_eq!(r.cache.cached(&frame), Some(other), "refreshed and fresh");
        r.clock.advance_micros(2 * US);
        assert_eq!(r.cache.cached(&frame), None, "aged out");
        assert_eq!(r.cache.route_frame(&frame, &mut r.out), Outcome::Held);
        assert_eq!(r.out.len(), 1, "one new request");
        assert_eq!(r.counters.snapshot().neighbor_expired, 1);
    }

    #[test]
    fn the_hold_is_bounded_per_next_hop_and_in_total() {
        let config = NeighborConfig {
            max_held_per_hop: 3,
            max_held_total: 4,
            ..NeighborConfig::default()
        };
        let mut r = rig(config.clone(), Route::default(), None);
        // Five frames to one peer: the oldest two go.
        for n in 0..5u8 {
            let mut f = frame_v4(HOST, [10, 0, 0, 100], None);
            f[18] = n; // the identification field tells them apart
            assert_eq!(r.cache.route_frame(&f, &mut r.out), Outcome::Held);
        }
        assert_eq!(r.counters.snapshot().neighbor_dropped, 2);
        let mut released = VecDeque::new();
        r.cache
            .learn(IpAddr::from([10, 0, 0, 100]), GW_MAC, &mut released);
        let ids: Vec<u8> = released.iter().map(|f| f[18]).collect();
        assert_eq!(ids, [2, 3, 4], "the newest three stay, in order");

        // The total limit: 3 frames for one peer and 2 for another is too many.
        let mut r = rig(config, Route::default(), None);
        for _ in 0..3 {
            r.cache
                .route_frame(&frame_v4(HOST, [10, 0, 0, 100], None), &mut r.out);
        }
        let to_other = frame_v4(HOST, [10, 0, 0, 101], None);
        assert_eq!(r.cache.route_frame(&to_other, &mut r.out), Outcome::Held);
        assert_eq!(
            r.cache.route_frame(&to_other, &mut r.out),
            Outcome::Dropped,
            "the queue as a whole is full"
        );
    }

    #[test]
    fn answers_that_no_frame_asked_for_are_ignored() {
        let mut r = rig(NeighborConfig::default(), v4_route(), None);
        let mut released = VecDeque::new();
        r.cache
            .learn(IpAddr::from([10, 0, 0, 200]), GW_MAC, &mut released);
        assert_eq!(r.cache.entries.len(), 0, "no entry from a stranger");
        // A group address or zero is never a neighbor.
        let frame = frame_v4(HOST, FAR, None);
        r.cache.route_frame(&frame, &mut r.out);
        for bad in [
            MacAddr([0; 6]),
            MacAddr([0x01, 0, 0x5e, 0, 0, 1]),
            MacAddr([0xff; 6]),
        ] {
            r.cache.learn(IpAddr::from(GATEWAY), bad, &mut released);
        }
        assert!(released.is_empty());
        assert_eq!(r.cache.cached(&frame), None);
        assert_eq!(r.counters.snapshot().neighbor_resolved, 0);
    }

    #[test]
    fn the_table_is_bounded_and_makes_room_by_forgetting_what_expired() {
        let config = NeighborConfig {
            max_entries: 2,
            lifetime: Duration::from_secs(10),
            ..NeighborConfig::default()
        };
        let mut r = rig(config, Route::default(), None);
        for last in [1u8, 2] {
            let f = frame_v4(HOST, [10, 0, 0, last], None);
            r.cache.route_frame(&f, &mut r.out);
            r.cache
                .learn(IpAddr::from([10, 0, 0, last]), GW_MAC, &mut VecDeque::new());
        }
        let third = frame_v4(HOST, [10, 0, 0, 3], None);
        assert_eq!(r.cache.route_frame(&third, &mut r.out), Outcome::Dropped);
        let c = r.counters.snapshot();
        assert_eq!((c.neighbor_table_full, c.neighbor_dropped), (1, 1));
        r.clock.advance_micros(11 * US);
        assert_eq!(r.cache.route_frame(&third, &mut r.out), Outcome::Held);
        assert_eq!(r.cache.entries.len(), 1, "the expired ones went");
    }

    #[test]
    fn ipv6_asks_with_a_neighbor_solicitation_for_the_gateway() {
        let route = Route::of(&pool(None, Some("fd00:1::/64"), Some("fd00:1::1")));
        let mut r = rig(NeighborConfig::default(), route, None);
        let src: Ipv6Addr = "fd00:1::2".parse().unwrap();
        let dst: Ipv6Addr = "fd00:2::9".parse().unwrap();
        let gateway: Ipv6Addr = "fd00:1::1".parse().unwrap();
        let frame = frame_v6(src, dst);
        assert_eq!(r.cache.route_frame(&frame, &mut r.out), Outcome::Held);
        let ask = r.out.front().unwrap();
        let (pos, ethertype, _, _) = ethernet_payload(ask).unwrap();
        assert_eq!(ethertype, ETHERTYPE_IPV6);
        let ip = &ask[pos..];
        assert_eq!(ip[40], ICMP6_NS);
        assert_eq!(ip[48..64], gateway.octets(), "the target is the gateway");
        assert_eq!(ip[8..24], src.octets(), "from the host");
        let mut released = VecDeque::new();
        r.cache.learn(IpAddr::V6(gateway), GW_MAC, &mut released);
        assert_eq!(released.len(), 1);
        assert_eq!(released[0][..6], GW_MAC.0);
        assert_eq!(r.cache.cached(&frame), Some(GW_MAC));
    }

    #[test]
    fn a_frame_that_is_not_ip_is_dropped() {
        let mut r = rig(NeighborConfig::default(), v4_route(), None);
        let mut f = NEXT_HOP_MAC.0.to_vec();
        f.extend_from_slice(&OWN_MAC.0);
        f.extend_from_slice(&[0x88, 0xb5]);
        f.resize(60, 0);
        assert!(needs_next_hop(&f));
        assert_eq!(r.cache.route_frame(&f, &mut r.out), Outcome::Dropped);
        assert!(r.out.is_empty());
        assert_eq!(r.counters.snapshot().neighbor_dropped, 1);
        assert!(!needs_next_hop(&frame_v4(HOST, FAR, None)[6..]));
    }
}
