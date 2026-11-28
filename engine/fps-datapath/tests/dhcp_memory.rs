// SPDX-License-Identifier: GPL-3.0-only
//! DHCP leases on the datapath over in memory links (TRN-37). No root is
//! needed, so these tests run in every job. The same run in the namespace
//! harness is `dhcp_harness.rs`.
//!
//! A router thread sits between the two arms of a lane. On the client arm it
//! is a DHCP server (host `i` of the pool `clients` gets `10.201.0.(100+i)`
//! with the router `10.201.0.1`), and it answers ARP for its two addresses and
//! forwards what the stack sends to its MAC address. The server pool is
//! static. The stack therefore reaches the server hosts only if the hosts of
//! the client arm lease their addresses, the arm learns its gateway from the
//! lease, and the sessions open from the leased addresses.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use fps_backend_afxdp::memory::{frame_pair, MemoryFrames};
use fps_backend_afxdp::FrameIo;
use fps_datapath::{
    split_pools, Datapath, DatapathConfig, DatapathError, DhcpSettings, Lane, NeighborConfig, Side,
};
use fps_hosts::dhcp::frame::{parse_udp4, udp4_frame};
use fps_hosts::dhcp::{Message, MessageType, Timing, CLIENT_PORT, SERVER_PORT};
use fps_hosts::wire::{arp_frame, parse_arp, ARP_REPLY, ARP_REQUEST, BROADCAST_MAC};
use fps_hosts::{DhcpConfig, HostPool, MacAddr, PoolSpec};
use fps_tcp_own::{HostSlice, Pools};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const FRAME: usize = 1600;
const MAC_A: MacAddr = MacAddr([0x02, 0xee, 0, 0, 0, 0x0a]);
const MAC_B: MacAddr = MacAddr([0x02, 0xee, 0, 0, 0, 0x0b]);
const ROUTER_A: Ipv4Addr = Ipv4Addr::new(10, 201, 0, 1);
const ROUTER_B: Ipv4Addr = Ipv4Addr::new(10, 202, 0, 1);

fn pools(clients: u32) -> Pools {
    let leasing = HostPool::dhcp("clients", clients, Some("02:aa:00:00:00:00")).unwrap();
    let servers = HostPool::parse(&PoolSpec {
        name: "servers",
        ipv4: Some("10.202.0.0/24"),
        count: 4,
        gateway: Some("10.202.0.1"),
        ..PoolSpec::default()
    })
    .unwrap();
    Pools {
        client: HostSlice::whole(leasing),
        server: HostSlice::whole(servers),
        vlan: None,
    }
}

/// The address that the server gives the host with MAC address `mac`.
fn lease_of(mac: MacAddr) -> Ipv4Addr {
    Ipv4Addr::new(10, 201, 0, 100 + mac.0[5])
}

fn fast() -> DhcpSettings {
    DhcpSettings {
        config: DhcpConfig {
            timing: Timing {
                start_delay_max: Duration::from_millis(30),
                initial_retransmit: Duration::from_millis(200),
                max_retransmit: Duration::from_millis(400),
                ..Timing::default()
            },
            seed: 7,
            ..DhcpConfig::default()
        },
        timeout: Duration::from_secs(10),
    }
}

fn config(dhcp: DhcpSettings) -> DatapathConfig {
    DatapathConfig {
        neighbors: Some(NeighborConfig::default()),
        dhcp: Some(dhcp),
        ..DatapathConfig::default()
    }
}

/// The DHCP reply of the test server to `frame`, if it is a message for it.
fn dhcp_answer(frame: &[u8]) -> Option<(Vec<u8>, MacAddr, Ipv4Addr)> {
    let udp = parse_udp4(frame).filter(|u| u.dst_port == SERVER_PORT)?;
    let req = Message::parse(udp.payload).ok()?;
    let kind = match req.kind {
        MessageType::Discover => MessageType::Offer,
        MessageType::Request => MessageType::Ack,
        _ => return None,
    };
    let addr = lease_of(req.chaddr);
    let mut reply = Message::new(kind, req.xid, req.chaddr);
    reply.yiaddr = addr;
    reply.options.server_id = Some(ROUTER_A);
    reply.options.lease_secs = Some(3600);
    reply.options.subnet_mask = Some(Ipv4Addr::new(255, 255, 255, 0));
    reply.options.routers = vec![ROUTER_A];
    let (dst_mac, dst_ip) = if req.wants_broadcast() {
        (BROADCAST_MAC, Ipv4Addr::BROADCAST)
    } else {
        (req.chaddr, req.ciaddr)
    };
    let out = udp4_frame(
        dst_mac,
        MAC_A,
        &[],
        ROUTER_A,
        dst_ip,
        SERVER_PORT,
        CLIENT_PORT,
        &reply.encode(),
    );
    Some((out, req.chaddr, addr))
}

fn arp_reply(request: &[u8], mac: MacAddr, ip: Ipv4Addr) -> Option<Vec<u8>> {
    let arp = parse_arp(request).filter(|a| a.op == ARP_REQUEST && a.target_ip == ip)?;
    Some(arp_frame(
        ARP_REPLY,
        arp.sender_mac,
        mac,
        &[],
        mac,
        ip,
        arp.sender_mac,
        arp.sender_ip,
    ))
}

fn dst_ip(frame: &[u8]) -> Option<Ipv4Addr> {
    if frame.get(12..14) != Some(&[0x08, 0x00]) {
        return None;
    }
    Some(Ipv4Addr::from(
        <[u8; 4]>::try_from(frame.get(30..34)?).ok()?,
    ))
}

/// The router and DHCP server between the two arms of one lane.
struct Router {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// The hosts that were given an address, by address.
    leased: Arc<Mutex<HashMap<Ipv4Addr, MacAddr>>>,
}

impl Router {
    fn start(
        mut a: MemoryFrames,
        mut b: MemoryFrames,
        servers: HashMap<Ipv4Addr, MacAddr>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let leased = Arc::new(Mutex::new(HashMap::new()));
        let (flag, table) = (Arc::clone(&stop), Arc::clone(&leased));
        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                let (mut from_a, mut from_b) = (Vec::new(), Vec::new());
                a.recv_batch(64, &mut |f| from_a.push(f.to_vec())).unwrap();
                b.recv_batch(64, &mut |f| from_b.push(f.to_vec())).unwrap();
                let idle = from_a.is_empty() && from_b.is_empty();
                for f in &from_a {
                    if let Some((reply, mac, addr)) = dhcp_answer(f) {
                        table.lock().unwrap().insert(addr, mac);
                        let _ = a.send_batch(&[&reply]);
                    } else if let Some(r) = arp_reply(f, MAC_A, ROUTER_A) {
                        let _ = a.send_batch(&[&r]);
                    } else if f.get(..6) == Some(&MAC_A.0[..]) {
                        if let Some(mac) = dst_ip(f).and_then(|ip| servers.get(&ip)) {
                            let mut out = f.clone();
                            out[..6].copy_from_slice(&mac.0);
                            out[6..12].copy_from_slice(&MAC_B.0);
                            let _ = b.send_batch(&[&out]);
                        }
                    }
                }
                for f in &from_b {
                    if let Some(r) = arp_reply(f, MAC_B, ROUTER_B) {
                        let _ = b.send_batch(&[&r]);
                    } else if f.get(..6) == Some(&MAC_B.0[..]) {
                        let mac = dst_ip(f).and_then(|ip| table.lock().unwrap().get(&ip).copied());
                        if let Some(mac) = mac {
                            let mut out = f.clone();
                            out[..6].copy_from_slice(&mac.0);
                            out[6..12].copy_from_slice(&MAC_A.0);
                            let _ = a.send_batch(&[&out]);
                        }
                    }
                }
                let _ = a.flush();
                let _ = b.flush();
                if idle {
                    std::thread::sleep(Duration::from_micros(50));
                }
            }
        });
        Self {
            stop,
            thread: Some(thread),
            leased,
        }
    }
}

impl Drop for Router {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn routed_lane(pools: &Pools) -> (Lane, Router) {
    let (client, a) = frame_pair(2048, FRAME);
    let (server, b) = frame_pair(2048, FRAME);
    let tables = pools.tables().unwrap();
    let mut servers = HashMap::new();
    for i in 0..pools.server.hosts() {
        let Some(IpAddr::V4(ip)) = pools.server.ip(i) else {
            continue;
        };
        servers.insert(ip, tables.server.lookup_v4(ip).unwrap());
    }
    let lane = Lane {
        client: Box::new(client),
        server: Box::new(server),
        pools: pools.clone(),
    };
    (lane, Router::start(a, b, servers))
}

#[tokio::test(flavor = "current_thread")]
async fn every_lease_binds_and_sessions_from_the_leased_addresses_reach_the_static_pool() {
    let all = pools(6);
    let slices = split_pools(&all, 2).unwrap();
    let (mut lanes, mut routers) = (Vec::new(), Vec::new());
    for p in &slices {
        let (lane, router) = routed_lane(p);
        lanes.push(lane);
        routers.push(router);
    }
    let dp = Datapath::start(lanes, config(fast())).unwrap();
    let report = dp.wait_leases().unwrap();
    assert!(report.is_complete(), "{report:?}");
    assert_eq!((report.bound(), report.hosts()), (6, 6));
    assert_eq!(report.arms.len(), 2, "one leasing arm per lane");
    assert!(report
        .arms
        .iter()
        .all(|a| a.side == Side::Client && a.bound == 3));

    let mut listener = dp.listen(9200).unwrap();
    let server = tokio::spawn(async move {
        let mut peers = Vec::new();
        for _ in 0..6 {
            let (mut s, peer) = listener.accept().await.unwrap();
            peers.push(peer.ip());
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4000];
                s.read_exact(&mut buf).await.unwrap();
                s.write_all(&buf).await.unwrap();
                s.shutdown().await.unwrap();
            });
        }
        peers
    });
    let connector = dp.connector();
    let mut sources = Vec::new();
    for i in 0..6u8 {
        let source = IpAddr::V4(Ipv4Addr::new(10, 201, 0, 100 + i));
        // Lane 0 serves hosts 0 to 2 and lane 1 hosts 3 to 5, and each lane
        // has its own server hosts.
        let lane_servers = &slices[usize::from(i / 3)].server;
        let dst = SocketAddr::new(lane_servers.ip(0).unwrap(), 9200);
        let mut session = connector.connect(source, dst).await.unwrap();
        let data: Vec<u8> = (0..4000).map(|n| (n % 251) as u8 ^ i).collect();
        session.write_all(&data).await.unwrap();
        let mut back = Vec::new();
        session.read_to_end(&mut back).await.unwrap();
        assert_eq!(back, data, "host {i}");
        sources.push(source);
    }
    let mut peers = server.await.unwrap();
    peers.sort();
    sources.sort();
    assert_eq!(peers, sources, "the servers saw the leased addresses");
    for (lane, stats) in dp.stats().iter().enumerate() {
        let c = &stats.client;
        assert_eq!(c.dhcp_unsupported, 0, "lane {lane}: {c:?}");
        assert!(c.dhcp_handled >= 3, "lane {lane}: {c:?}");
        // The route came from the lease: the arm asked for its router, which
        // is not a host of any pool, and nothing else.
        assert_eq!(c.neighbor_resolved, 1, "lane {lane}: {c:?}");
        assert_eq!(c.neighbor_failed, 0, "lane {lane}: {c:?}");
    }
    let reports = dp.stop();
    assert!(reports.iter().all(|r| !r.panicked));
    for r in &routers {
        assert_eq!(r.leased.lock().unwrap().len(), 3);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_leased_host_answers_arp_for_its_address_and_no_other() {
    let all = pools(2);
    let (client, mut tap) = frame_pair(512, FRAME);
    let (server, _unused) = frame_pair(512, FRAME);
    let lane = Lane {
        client: Box::new(client),
        server: Box::new(server),
        pools: all,
    };
    let dp = Datapath::start(vec![lane], config(fast())).unwrap();
    // Play the server by hand until both hosts hold their address.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut macs = HashMap::new();
    while !(macs.len() == 2 && dp.leases().report().is_complete()) {
        assert!(Instant::now() < deadline, "no DHCP exchange: {macs:?}");
        let mut inbox = Vec::new();
        tap.recv_batch(64, &mut |f| inbox.push(f.to_vec())).unwrap();
        for f in &inbox {
            if let Some((reply, mac, addr)) = dhcp_answer(f) {
                macs.insert(addr, mac);
                tap.send_batch(&[&reply]).unwrap();
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let ask = |tap: &mut MemoryFrames, target: Ipv4Addr| -> Option<MacAddr> {
        let request = arp_frame(
            ARP_REQUEST,
            BROADCAST_MAC,
            MAC_A,
            &[],
            MAC_A,
            ROUTER_A,
            MacAddr([0; 6]),
            target,
        );
        tap.send_batch(&[&request]).unwrap();
        let deadline = Instant::now() + Duration::from_millis(600);
        while Instant::now() < deadline {
            let mut found = None;
            tap.recv_batch(16, &mut |f| {
                if let Some(a) = parse_arp(f).filter(|a| a.op == ARP_REPLY && a.sender_ip == target)
                {
                    found = Some(a.sender_mac);
                }
            })
            .unwrap();
            if found.is_some() {
                return found;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        None
    };
    for (addr, mac) in &macs {
        assert_eq!(ask(&mut tap, *addr), Some(*mac), "{addr}");
    }
    assert_eq!(ask(&mut tap, Ipv4Addr::new(10, 201, 0, 150)), None);
    assert_eq!(ask(&mut tap, Ipv4Addr::new(10, 201, 0, 1)), None);
}

#[test]
fn with_no_dhcp_server_the_lease_phase_ends_at_its_timeout_and_names_dhcp_and_the_count() {
    let all = pools(3);
    let (client, _a) = frame_pair(512, FRAME);
    let (server, _b) = frame_pair(512, FRAME);
    let lane = Lane {
        client: Box::new(client),
        server: Box::new(server),
        pools: all,
    };
    let mut settings = fast();
    settings.timeout = Duration::from_millis(400);
    let dp = Datapath::start(vec![lane], config(settings)).unwrap();
    let started = Instant::now();
    let error = dp.wait_leases().unwrap_err();
    assert!(started.elapsed() >= Duration::from_millis(400), "{error}");
    assert!(started.elapsed() < Duration::from_secs(5), "{error}");
    let text = error.to_string();
    assert!(text.contains("dhcp4"), "{text}");
    assert!(text.contains("0 of 3"), "{text}");
    assert!(text.contains("clients"), "{text}");
    assert!(
        matches!(
            error,
            DatapathError::LeaseTimeout {
                bound: 0,
                hosts: 3,
                ..
            }
        ),
        "{error:?}"
    );
    // A connect from an address that no host leased is refused.
    let source = IpAddr::V4(Ipv4Addr::new(10, 201, 0, 100));
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(
            dp.connector()
                .connect(source, SocketAddr::new(IpAddr::V4(ROUTER_B), 80)),
        );
    assert!(matches!(result, Err(DatapathError::UnknownSource(_))));
}

#[test]
fn a_pool_that_leases_needs_the_dhcp_settings() {
    let (client, _a) = frame_pair(64, FRAME);
    let (server, _b) = frame_pair(64, FRAME);
    let lane = Lane {
        client: Box::new(client),
        server: Box::new(server),
        pools: pools(2),
    };
    let error = Datapath::start(vec![lane], DatapathConfig::default()).unwrap_err();
    assert!(matches!(error, DatapathError::DhcpOff { .. }), "{error:?}");
    assert!(error.to_string().contains("dhcp4"));
}

#[test]
fn static_pools_have_an_empty_lease_phase() {
    let (client, _a) = frame_pair(64, FRAME);
    let (server, _b) = frame_pair(64, FRAME);
    let lane = Lane {
        client: Box::new(client),
        server: Box::new(server),
        pools: Pools::counts(2, 2, None).unwrap(),
    };
    let dp = Datapath::start(vec![lane], DatapathConfig::default()).unwrap();
    let report = dp.wait_leases().unwrap();
    assert!(report.arms.is_empty() && report.is_complete());
}
