// SPDX-License-Identifier: GPL-3.0-only
//! Tests of the neighbor service over the in memory frame pair of the AF_XDP
//! backend, and the 20,000 host proof of HST-03.
//!
//! A router end of the pair asks for every host of a pool, the service on the
//! other end answers, and the test writes every frame of both directions to a
//! pcap file. The realism host identity check scores that capture through
//! `tests/score_hosts.py`. The tests need `python3` and need no root.

use std::io::Write;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use fps_backend_afxdp::memory::{frame_pair, MemoryFrames};
use fps_backend_afxdp::FrameIo;
use fps_hosts::wire::{arp_request, ns_frame, parse_arp, parse_neighbor_advert, ARP_REPLY};
use fps_hosts::{
    HostPool, MacAddr, MacAllocator, NeighborService, NeighborTable, PoolSpec, Responder,
};

const ROUTER_MAC: MacAddr = MacAddr([0x02, 0xee, 0, 0, 0, 0x01]);
const ROUTER_V4: Ipv4Addr = Ipv4Addr::new(10, 20, 0, 1);
const ROUTER_V6: Ipv6Addr = Ipv6Addr::new(0x2001, 0xdb8, 0x20, 0xffff, 0, 0, 0, 1);
const BATCH: usize = 64;

fn pool(
    name: &str,
    v4: Option<&str>,
    v6: Option<&str>,
    count: u32,
    mac_base: Option<&str>,
) -> HostPool {
    HostPool::parse(&PoolSpec {
        name,
        ipv4: v4,
        ipv6: v6,
        count,
        // The router is the gateway, so no host owns its address.
        gateway: v4.map(|_| "10.20.0.1"),
        mac_base,
    })
    .unwrap()
}

/// Builds a responder for the given pools, with the router MAC reserved.
fn responder(pools: &[&HostPool]) -> Responder {
    let mut alloc = MacAllocator::new();
    alloc.reserve("router", ROUTER_MAC).unwrap();
    let mut table = NeighborTable::new();
    for p in pools {
        let block = alloc.claim(p, p.range()).unwrap();
        table.add(p, &block).unwrap();
    }
    Responder::new(table)
}

/// The frames of a capture, written as a pcap file one microsecond apart.
struct Capture {
    frames: Vec<Vec<u8>>,
}

impl Capture {
    fn new() -> Self {
        Self { frames: Vec::new() }
    }

    fn write(&self, path: &Path) {
        let mut out = Vec::new();
        out.extend_from_slice(&0xa1b2_c3d4u32.to_le_bytes());
        out.extend_from_slice(&[2, 0, 4, 0]);
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&65535u32.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        for (i, f) in self.frames.iter().enumerate() {
            let t = 1_700_000_000u32 + (i / 1_000_000) as u32;
            let us = (i % 1_000_000) as u32;
            out.extend_from_slice(&t.to_le_bytes());
            out.extend_from_slice(&us.to_le_bytes());
            out.extend_from_slice(&(f.len() as u32).to_le_bytes());
            out.extend_from_slice(&(f.len() as u32).to_le_bytes());
            out.extend_from_slice(f);
        }
        std::fs::File::create(path)
            .and_then(|mut f| f.write_all(&out))
            .unwrap();
    }
}

fn capture_path(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("hst-03");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{name}.pcap"))
}

/// Sends `requests` through the pair in batches, lets the services answer,
/// and returns the replies. Every frame of both directions goes into `cap`.
fn exchange(
    router: &mut MemoryFrames,
    services: &mut [(&mut MemoryFrames, NeighborService<'_>)],
    requests: &[Vec<u8>],
    cap: &mut Capture,
) -> Vec<Vec<u8>> {
    let mut replies = Vec::new();
    for chunk in requests.chunks(BATCH) {
        let refs: Vec<&[u8]> = chunk.iter().map(Vec::as_slice).collect();
        assert_eq!(router.send_batch(&refs).unwrap(), chunk.len());
        cap.frames.extend(chunk.iter().cloned());
        for (io, service) in services.iter_mut() {
            let mut unexpected = 0;
            while io.wait_readable(Duration::ZERO).unwrap() {
                service.poll(&mut **io, &mut |_| unexpected += 1).unwrap();
            }
            assert_eq!(
                unexpected, 0,
                "a neighbor request reached the pass callback"
            );
        }
        while router
            .recv_batch(BATCH * 2, &mut |f| {
                replies.push(f.to_vec());
                cap.frames.push(f.to_vec());
            })
            .unwrap()
            > 0
        {}
    }
    replies
}

fn score(capture: &Path, args: &[&str]) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/score_hosts.py");
    let out = Command::new("python3")
        .arg("-I")
        .arg(&script)
        .arg(capture)
        .args(args)
        .output()
        .expect("python3 is needed to run the realism host identity check");
    assert!(
        out.status.success(),
        "host identity check failed for {}\nstdout: {}\nstderr: {}",
        capture.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn twenty_thousand_ipv4_hosts_pass_the_host_identity_check() {
    let p = pool(
        "web",
        Some("10.20.0.0/16"),
        None,
        20_000,
        Some("02:aa:00:00:00:00"),
    );
    let r = responder(&[&p]);
    let (mut router, mut queue) = frame_pair(1024, 1500);
    let mut services = [(&mut queue, NeighborService::new(&r, BATCH))];
    let requests: Vec<Vec<u8>> = (0..20_000)
        .map(|i| arp_request(ROUTER_MAC, ROUTER_V4, p.ipv4_addr(i).unwrap().unwrap(), &[]))
        .collect();
    let mut cap = Capture::new();
    let replies = exchange(&mut router, &mut services, &requests, &mut cap);
    assert_eq!(replies.len(), 20_000);
    for (i, f) in replies.iter().enumerate() {
        let a = parse_arp(f).unwrap();
        let i = i as u32;
        assert_eq!(a.op, ARP_REPLY);
        assert_eq!(a.sender_ip, p.ipv4_addr(i).unwrap().unwrap());
        assert_eq!(Some(a.sender_mac), p.mac(i).unwrap());
    }
    assert_eq!(services[0].1.stats().arp_replies, 20_000);
    let path = capture_path("ipv4-20000");
    cap.write(&path);
    // The 20,000 hosts and the router are 20,001 hosts and MAC addresses.
    score(&path, &["--hosts", "20001", "--macs", "20001"]);
}

#[test]
fn twenty_thousand_ipv6_hosts_pass_the_host_identity_check() {
    let p = pool(
        "web6",
        None,
        Some("2001:db8:20::/64"),
        20_000,
        Some("02:bb:00:00:00:00"),
    );
    let r = responder(&[&p]);
    let (mut router, mut queue) = frame_pair(1024, 1500);
    let mut services = [(&mut queue, NeighborService::new(&r, BATCH))];
    let requests: Vec<Vec<u8>> = (0..20_000)
        .map(|i| ns_frame(ROUTER_MAC, ROUTER_V6, p.ipv6_addr(i).unwrap().unwrap(), &[]))
        .collect();
    let mut cap = Capture::new();
    let replies = exchange(&mut router, &mut services, &requests, &mut cap);
    assert_eq!(replies.len(), 20_000);
    for (i, f) in replies.iter().enumerate() {
        let na = parse_neighbor_advert(f).unwrap();
        let i = i as u32;
        assert!(na.checksum_ok);
        assert_eq!(na.target, p.ipv6_addr(i).unwrap().unwrap());
        assert_eq!(na.target_lla, p.mac(i).unwrap());
    }
    let path = capture_path("ipv6-20000");
    cap.write(&path);
    score(&path, &["--hosts", "20001", "--macs", "20001"]);
}

#[test]
fn two_pools_with_an_allocated_base_share_one_port_without_conflicts() {
    // The second pool has no mac_base, so the allocator picks a block of its own.
    let a = pool(
        "a",
        Some("10.20.0.0/17"),
        None,
        10_000,
        Some("02:aa:00:00:00:00"),
    );
    let b = pool("b", Some("10.20.128.0/17"), None, 10_000, None);
    let r = responder(&[&a, &b]);
    assert_eq!(r.table().hosts(), 20_000);
    let (mut router, mut queue) = frame_pair(1024, 1500);
    let mut services = [(&mut queue, NeighborService::new(&r, BATCH))];
    let mut requests = Vec::new();
    for i in 0..10_000 {
        for p in [&a, &b] {
            requests.push(arp_request(
                ROUTER_MAC,
                ROUTER_V4,
                p.ipv4_addr(i).unwrap().unwrap(),
                &[],
            ));
        }
    }
    let mut cap = Capture::new();
    let replies = exchange(&mut router, &mut services, &requests, &mut cap);
    assert_eq!(replies.len(), 20_000);
    let path = capture_path("two-pools-20000");
    cap.write(&path);
    score(&path, &["--hosts", "20001", "--macs", "20001"]);
}

#[test]
fn the_check_sees_two_responders_that_claim_one_address() {
    // Two agents that both answer for the same 50 addresses with different MAC
    // bases, which is the conflict that a shared ledger prevents.
    let a = pool(
        "a",
        Some("10.20.0.0/24"),
        None,
        50,
        Some("02:aa:00:00:00:00"),
    );
    let b = pool(
        "b",
        Some("10.20.0.0/24"),
        None,
        50,
        Some("02:cc:00:00:00:00"),
    );
    let (ra, rb) = (responder(&[&a]), responder(&[&b]));
    let (mut router, mut qa) = frame_pair(256, 1500);
    let (mut router2, mut qb) = frame_pair(256, 1500);
    let mut cap = Capture::new();
    let requests: Vec<Vec<u8>> = (0..50)
        .map(|i| arp_request(ROUTER_MAC, ROUTER_V4, a.ipv4_addr(i).unwrap().unwrap(), &[]))
        .collect();
    let mut sa = [(&mut qa, NeighborService::new(&ra, BATCH))];
    let mut sb = [(&mut qb, NeighborService::new(&rb, BATCH))];
    exchange(&mut router, &mut sa, &requests, &mut cap);
    exchange(&mut router2, &mut sb, &requests, &mut cap);
    let path = capture_path("conflict-50");
    cap.write(&path);
    score(&path, &["--hosts", "51", "--expect-conflicts"]);
}

#[test]
fn a_shared_ledger_refuses_the_second_responder() {
    let a = pool(
        "a",
        Some("10.20.0.0/24"),
        None,
        50,
        Some("02:aa:00:00:00:00"),
    );
    let b = pool(
        "b",
        Some("10.20.1.0/24"),
        None,
        50,
        Some("02:aa:00:00:00:20"),
    );
    let mut alloc = MacAllocator::new();
    alloc.claim(&a, a.range()).unwrap();
    let err = alloc.claim(&b, b.range()).unwrap_err();
    assert!(err.to_string().contains("already taken by a"), "{err}");
}

#[test]
fn other_frames_pass_and_requests_for_others_are_dropped() {
    let p = pool(
        "p",
        Some("10.20.0.0/24"),
        None,
        10,
        Some("02:aa:00:00:00:00"),
    );
    let r = responder(&[&p]);
    let (mut router, mut queue) = frame_pair(64, 1500);
    let mut service = NeighborService::new(&r, 8);
    let mine = p.ipv4_addr(3).unwrap().unwrap();
    let stranger = Ipv4Addr::new(10, 20, 0, 200);
    let mut ipv4 = arp_request(ROUTER_MAC, ROUTER_V4, mine, &[]);
    ipv4[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
    let frames = [
        arp_request(ROUTER_MAC, ROUTER_V4, mine, &[]),
        arp_request(ROUTER_MAC, ROUTER_V4, stranger, &[]),
        ipv4,
        vec![1, 2, 3],
    ];
    let refs: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
    router.send_batch(&refs).unwrap();
    let mut passed = Vec::new();
    let n = service
        .poll(&mut queue, &mut |f| passed.push(f.to_vec()))
        .unwrap();
    assert_eq!(n, 4);
    assert_eq!(passed.len(), 2);
    let s = service.stats();
    assert_eq!(
        (s.arp_replies, s.not_for_us, s.passed, s.malformed),
        (1, 1, 2, 0)
    );
    let mut got = 0;
    router.recv_batch(8, &mut |_| got += 1).unwrap();
    assert_eq!(got, 1);
}

#[test]
fn a_full_transmit_queue_drops_replies_and_counts_them() {
    let p = pool(
        "p",
        Some("10.20.0.0/24"),
        None,
        100,
        Some("02:aa:00:00:00:00"),
    );
    let r = responder(&[&p]);
    // The reply direction holds 4 frames and nobody reads it.
    let (mut router, mut queue) = frame_pair(4, 1500);
    let mut service = NeighborService::new(&r, 4);
    for round in 0..3 {
        // Requests are read each round, so only the reply direction fills up.
        let frames: Vec<Vec<u8>> = (0..4)
            .map(|i| {
                arp_request(
                    ROUTER_MAC,
                    ROUTER_V4,
                    p.ipv4_addr(round * 4 + i).unwrap().unwrap(),
                    &[],
                )
            })
            .collect();
        let refs: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
        assert_eq!(router.send_batch(&refs).unwrap(), 4);
        assert_eq!(service.poll(&mut queue, &mut |_| {}).unwrap(), 4);
    }
    let s = service.stats();
    assert_eq!(s.arp_replies, 12);
    assert_eq!(s.tx_dropped, 8);
    let mut got = 0;
    router.recv_batch(64, &mut |_| got += 1).unwrap();
    assert_eq!(got, 4);
}

#[test]
fn a_queue_that_is_too_small_for_a_reply_is_refused() {
    let p = pool(
        "p",
        Some("10.20.0.0/24"),
        None,
        10,
        Some("02:aa:00:00:00:00"),
    );
    let r = responder(&[&p]);
    let (_router, mut queue) = frame_pair(4, 60);
    let mut service = NeighborService::new(&r, 4);
    let err = service.poll(&mut queue, &mut |_| {}).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn services_on_two_queues_share_one_responder() {
    let p = pool(
        "p",
        Some("10.20.0.0/24"),
        None,
        100,
        Some("02:aa:00:00:00:00"),
    );
    let r = responder(&[&p]);
    let (mut r0, mut q0) = frame_pair(64, 1500);
    let (mut r1, mut q1) = frame_pair(64, 1500);
    std::thread::scope(|s| {
        for (router, queue) in [(&mut r0, &mut q0), (&mut r1, &mut q1)] {
            let r = &r;
            let p = &p;
            s.spawn(move || {
                let mut service = NeighborService::new(r, 16);
                let req = arp_request(ROUTER_MAC, ROUTER_V4, p.ipv4_addr(7).unwrap().unwrap(), &[]);
                router.send_batch(&[&req]).unwrap();
                assert!(queue.wait_readable(Duration::from_secs(5)).unwrap());
                service.poll(queue, &mut |_| {}).unwrap();
                let mut got = Vec::new();
                assert!(router.wait_readable(Duration::from_secs(5)).unwrap());
                router.recv_batch(4, &mut |f| got.push(f.to_vec())).unwrap();
                assert_eq!(
                    parse_arp(&got[0]).unwrap().sender_mac,
                    p.mac(7).unwrap().unwrap()
                );
            });
        }
    });
}
