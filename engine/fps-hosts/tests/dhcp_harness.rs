// SPDX-License-Identifier: GPL-3.0-only
//! The DHCP clients of a pool in the QA-01 namespace harness (HST-04).
//!
//! The test brings up the harness and starts this test binary twice with
//! `ip netns exec`, through the hidden test `helper_process`. In the router
//! namespace the helper is a small DHCP server on a plain UDP socket bound to
//! `cli0`. In the client namespace the helper opens `eth0` with AF_XDP, makes
//! a pool of eight hosts that lease their addresses, and runs [`DhcpHosts`]
//! and the neighbor service on the queue, as an agent worker does.
//!
//! The run proves the acceptance item of the package: every host obtains an
//! address, every host renews it by unicast at T1, and the addresses stay the
//! same. The renewal reply is a unicast datagram to the leased address, so the
//! router must resolve that address by ARP, and the neighbor responder answers
//! from the lease table. The test reads the neighbor table of the router at
//! the end and checks that each leased address maps to the MAC address of its
//! host.
//!
//! AF_XDP and the harness need root. As a normal user the test runs itself
//! again through `sudo -n` and skips with a message when that is not possible.
//! `CI=true` or `FPS_REQUIRE_DHCP=1` turns every skip into a failure, so a
//! runner that cannot run the test does not pass silently.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use fps_backend_afxdp::{open_port, FrameIo, PortConfig};
use fps_hosts::dhcp::{Message, MessageType, Timing, CLIENT_PORT, SERVER_PORT};
use fps_hosts::{
    DhcpConfig, DhcpHosts, HostPool, MacAllocator, NeighborService, NeighborTable, Responder,
};
use socket2::{Domain, Protocol, Socket, Type};

/// The marker of the lines that helper processes report on.
const MARK: &str = "fps-dhcp ";
const HOSTS: u32 = 8;
const MAC_BASE: &str = "02:aa:00:00:00:00";
const LEASE_SECS: u32 = 6;
const TEST: &str = "hosts_lease_and_renew_addresses_in_the_namespace_harness";

fn harness_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/harness/netns.sh")
}

fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "{cmd} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

fn required() -> bool {
    std::env::var("CI").is_ok_and(|v| v == "true")
        || std::env::var("FPS_REQUIRE_DHCP").is_ok_and(|v| v == "1")
}

fn skip(why: &str) {
    assert!(
        !required(),
        "the DHCP harness test cannot run here and CI requires it: {why}"
    );
    eprintln!("skipping the DHCP harness test: {why}");
}

fn is_root() -> bool {
    run("id", &["-u"]).is_ok_and(|s| s.trim() == "0")
}

/// One QA-01 harness, which `Drop` takes down again.
struct Harness {
    prefix: String,
    state_dir: PathBuf,
}

impl Harness {
    fn up(prefix: &str) -> Result<Self, String> {
        let state_dir = std::env::temp_dir().join(format!("fps-dhcp-netns-{prefix}"));
        let out = Command::new(harness_script())
            .arg("up")
            .env("FPS_NETNS", prefix)
            .env("FPS_STATE_DIR", &state_dir)
            .output()
            .map_err(|e| format!("netns.sh: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        Ok(Self {
            prefix: prefix.to_owned(),
            state_dir,
        })
    }

    fn ns(&self, arm: &str) -> String {
        format!("{}-{arm}", self.prefix)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = Command::new(harness_script())
            .arg("down")
            .env("FPS_NETNS", &self.prefix)
            .env("FPS_STATE_DIR", &self.state_dir)
            .status();
    }
}

/// A helper process in a namespace, with its report lines.
struct Helper {
    child: Child,
    lines: mpsc::Receiver<String>,
    stdin: Option<ChildStdin>,
    seen: Vec<String>,
}

impl Helper {
    fn start(ns: &str, env: &[(&str, &str)]) -> Self {
        let exe = std::env::current_exe().unwrap();
        let mut cmd = Command::new("ip");
        cmd.args(["netns", "exec", ns]).arg(exe).args([
            "--ignored",
            "--exact",
            "helper_process",
            "--nocapture",
            "--test-threads",
            "1",
        ]);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let stdin = child.stdin.take();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                // libtest prints the test name on the same line before the
                // output of the helper.
                if let Some(at) = line.find(MARK) {
                    if tx.send(line[at + MARK.len()..].to_owned()).is_err() {
                        break;
                    }
                }
            }
        });
        Self {
            child,
            lines,
            stdin,
            seen: Vec::new(),
        }
    }

    /// Waits for a report line that starts with `prefix` and returns it.
    fn wait_line(&mut self, prefix: &str, timeout: Duration) -> String {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(l) = self.seen.iter().find(|l| l.starts_with(prefix)) {
                return l.clone();
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left.max(Duration::from_millis(1))) {
                Ok(l) => self.seen.push(l),
                Err(_) => panic!(
                    "no report line {prefix:?} within {timeout:?}, saw {:?}",
                    self.seen
                ),
            }
        }
    }

    /// Closes stdin, which tells the server to stop, and collects the
    /// remaining lines and the exit status.
    fn finish(mut self) -> (bool, Vec<String>) {
        drop(self.stdin.take());
        let status = self.child.wait().unwrap();
        while let Ok(l) = self.lines.recv_timeout(Duration::from_millis(500)) {
            self.seen.push(l);
        }
        for l in &self.seen {
            eprintln!("{MARK}{l}");
        }
        (status.success(), std::mem::take(&mut self.seen))
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn field<'a>(line: &'a str, key: &str) -> &'a str {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
        .unwrap_or_else(|| panic!("no field {key} in {line:?}"))
}

fn host_mac(index: u32) -> String {
    format!("02:aa:00:00:{:02x}:{:02x}", index >> 8, index & 0xff)
}

/// The address that the server gives the host with MAC `mac`.
fn host_addr(index: u32) -> Ipv4Addr {
    Ipv4Addr::new(10, 201, 0, 100 + u8::try_from(index).unwrap())
}

#[test]
fn hosts_lease_and_renew_addresses_in_the_namespace_harness() {
    if !is_root() {
        if run("sudo", &["-n", "true"]).is_err() {
            return skip("not root and no passwordless sudo");
        }
        let exe = std::env::current_exe().unwrap();
        // sudo drops the environment, so the child learns from the flag below
        // whether a skip is a failure.
        let require = format!("FPS_REQUIRE_DHCP={}", u8::from(required()));
        let status = Command::new("sudo")
            .args(["-n", "env", "FPS_DHCP_UNDER_SUDO=1", &require])
            .arg(exe)
            .args(["--exact", TEST, "--nocapture"])
            .status()
            .unwrap();
        assert!(status.success(), "the test failed under sudo");
        return;
    }
    if let Err(e) = run("ip", &["-V"]) {
        return skip(&e);
    }
    let prefix = format!("dhcp{}", std::process::id() % 100_000);
    let h = match Harness::up(&prefix) {
        Ok(h) => h,
        Err(e) => return skip(&format!("the namespace harness did not come up: {e}")),
    };
    let (client, router) = (h.ns("client"), h.ns("router"));
    // The harness configures the client arm as a host. The hosts of the pool
    // own the arm, so the test removes that configuration first.
    run("ip", &["-n", &client, "route", "flush", "dev", "eth0"]).unwrap();
    run("ip", &["-n", &client, "addr", "flush", "dev", "eth0"]).unwrap();

    let mut server = Helper::start(
        &router,
        &[
            ("FPS_DHCP_ROLE", "server"),
            ("FPS_DHCP_IFACE", "cli0"),
            ("FPS_DHCP_SERVER_IP", "10.201.0.1"),
        ],
    );
    server.wait_line("listening", Duration::from_secs(20));
    let mut hosts = Helper::start(
        &client,
        &[("FPS_DHCP_ROLE", "client"), ("FPS_DHCP_IFACE", "eth0")],
    );
    let first = hosts.wait_line("", Duration::from_secs(20));
    if first.starts_with("unsupported") {
        return skip(&format!("the client helper could not open AF_XDP: {first}"));
    }

    // Every host leases an address.
    let bound = hosts.wait_line("bound", Duration::from_secs(30));
    assert_eq!(field(&bound, "count"), HOSTS.to_string(), "{bound}");
    let want: Vec<String> = (0..HOSTS).map(|i| host_addr(i).to_string()).collect();
    assert_eq!(field(&bound, "addrs"), want.join(","), "{bound}");

    // Every host renews at T1, which is half the lease, and keeps its address.
    let started = Instant::now();
    let renewed = hosts.wait_line("renewed", Duration::from_secs(40));
    assert!(
        started.elapsed() >= Duration::from_secs(u64::from(LEASE_SECS) / 2 - 1),
        "a renewal came after {:?}, before T1",
        started.elapsed()
    );
    assert_eq!(field(&renewed, "count"), HOSTS.to_string(), "{renewed}");
    assert_eq!(field(&renewed, "lost"), "0", "{renewed}");
    assert_eq!(field(&renewed, "same_addresses"), "true", "{renewed}");
    assert_eq!(field(&renewed, "same_generation"), "true", "{renewed}");
    let released = hosts.wait_line("released", Duration::from_secs(20));
    assert_eq!(field(&released, "n"), HOSTS.to_string(), "{released}");
    assert_eq!(field(&released, "rebinds"), "0", "{released}");
    assert_eq!(field(&released, "foreign"), "0", "{released}");
    assert_eq!(field(&released, "malformed"), "0", "{released}");
    let (ok, lines) = hosts.finish();
    assert!(ok, "the client helper failed: {lines:?}");

    // The router resolved each leased address to the MAC address of its host,
    // which only the lease table of the client could answer.
    let neigh = run("ip", &["-n", &router, "-4", "neigh", "show", "dev", "cli0"]).unwrap();
    for i in 0..HOSTS {
        let line = neigh
            .lines()
            .find(|l| l.starts_with(&format!("{} ", host_addr(i))))
            .unwrap_or_else(|| panic!("the router has no neighbor entry for host {i}: {neigh}"));
        assert!(
            line.contains(&format!("lladdr {}", host_mac(i))),
            "host {i}: {line}"
        );
    }

    let (ok, lines) = server.finish();
    assert!(ok, "the server helper failed: {lines:?}");
    let s = lines.iter().find(|l| l.starts_with("server")).unwrap();
    let n = u64::from(HOSTS);
    assert_eq!(field(s, "selecting").parse::<u64>().unwrap(), n, "{s}");
    assert!(field(s, "discovers").parse::<u64>().unwrap() >= n, "{s}");
    assert!(field(s, "renewals").parse::<u64>().unwrap() >= n, "{s}");
    assert_eq!(field(s, "rebinds"), "0", "{s}");
    assert_eq!(field(s, "releases").parse::<u64>().unwrap(), n, "{s}");
}

// ---------------------------------------------------------------------------
// The helper processes
// ---------------------------------------------------------------------------

fn say(msg: &str) {
    println!("{MARK}{msg}");
    let _ = std::io::stdout().flush();
}

fn env_var(key: &str) -> String {
    std::env::var(key).unwrap_or_default()
}

#[test]
#[ignore = "started by the harness test inside a namespace"]
fn helper_process() {
    let Ok(role) = std::env::var("FPS_DHCP_ROLE") else {
        return;
    };
    match role.as_str() {
        "server" => helper_server(&env_var("FPS_DHCP_IFACE"), &env_var("FPS_DHCP_SERVER_IP")),
        "client" => helper_client(&env_var("FPS_DHCP_IFACE")),
        other => panic!("unknown role {other}"),
    }
}

/// A DHCP server on a UDP socket. It gives the host with MAC `…:xx` the
/// address `10.201.0.(100 + xx)`, answers a client with the broadcast flag by
/// broadcast and any other by unicast to `ciaddr`, and counts what it saw.
fn helper_server(iface: &str, server_ip: &str) {
    let server_ip: Ipv4Addr = server_ip.parse().unwrap();
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();
    sock.set_reuse_address(true).unwrap();
    sock.set_broadcast(true).unwrap();
    sock.bind_device(Some(iface.as_bytes())).unwrap();
    sock.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, SERVER_PORT)).into())
        .unwrap();
    let sock: UdpSocket = sock.into();
    sock.set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    std::thread::spawn(move || {
        // The parent closes our stdin when the test is over.
        let mut sink = Vec::new();
        let _ = std::io::stdin().read_to_end(&mut sink);
        flag.store(true, Ordering::SeqCst);
    });
    say("listening");

    let (mut discovers, mut selecting, mut renewals, mut rebinds, mut releases, mut acks) =
        (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let mut buf = [0u8; 1500];
    let mut drained = false;
    loop {
        let (n, from) = match sock.recv_from(&mut buf) {
            Ok(r) => r,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                if stop.load(Ordering::SeqCst) {
                    if drained {
                        break;
                    }
                    // One more pass after the stop, for datagrams in flight.
                    drained = true;
                }
                continue;
            }
            Err(e) => panic!("recv_from: {e}"),
        };
        let Ok(req) = Message::parse(&buf[..n]) else {
            continue;
        };
        let from_ip = from.ip();
        let mac = req.chaddr.0;
        let addr = Ipv4Addr::new(10, 201, 0, 100u8.wrapping_add(mac[5]));
        let kind = match req.kind {
            MessageType::Discover => {
                discovers += 1;
                MessageType::Offer
            }
            MessageType::Request => {
                if req.ciaddr.is_unspecified() {
                    selecting += 1;
                } else if from_ip == std::net::IpAddr::V4(req.ciaddr) {
                    renewals += 1;
                } else {
                    rebinds += 1;
                }
                acks += 1;
                MessageType::Ack
            }
            MessageType::Release => {
                assert_eq!(req.ciaddr, addr);
                releases += 1;
                continue;
            }
            _ => continue,
        };
        let mut reply = Message::new(kind, req.xid, req.chaddr);
        reply.yiaddr = addr;
        reply.options.server_id = Some(server_ip);
        reply.options.lease_secs = Some(LEASE_SECS);
        reply.options.subnet_mask = Some(Ipv4Addr::new(255, 255, 255, 0));
        reply.options.routers = vec![server_ip];
        let to = if req.wants_broadcast() {
            Ipv4Addr::BROADCAST
        } else {
            req.ciaddr
        };
        sock.send_to(&reply.encode(), SocketAddr::from((to, CLIENT_PORT)))
            .unwrap();
    }
    say(&format!(
        "server discovers={discovers} selecting={selecting} renewals={renewals} \
         rebinds={rebinds} releases={releases} acks={acks}"
    ));
}

/// Builds the responder for the leases of `hosts`, as an agent worker does
/// when the generation of the lease table changes.
fn responder_for(pool: &HostPool, hosts: &DhcpHosts) -> Responder {
    let mut alloc = MacAllocator::new();
    let block = alloc.claim(pool, hosts.leases().range()).unwrap();
    let mut table = NeighborTable::new();
    table.add_leased(pool, &block, hosts.leases()).unwrap();
    Responder::new(table)
}

fn helper_client(iface: &str) {
    let opened = match open_port(&PortConfig::new(iface)) {
        Ok(p) => p,
        Err(e) if e.raw_os_error() == Some(97) => {
            say(&format!("unsupported AF_XDP: {e}"));
            return;
        }
        Err(e) => panic!("open_port: {e:?}"),
    };
    say("opened");
    let mut queue = opened.queues.into_iter().next().unwrap();
    let pool = HostPool::dhcp("clients", HOSTS, Some(MAC_BASE)).unwrap();
    let mut alloc = MacAllocator::new();
    let block = alloc.claim(&pool, pool.range()).unwrap();
    let config = DhcpConfig {
        timing: Timing {
            start_delay_max: Duration::from_millis(300),
            initial_retransmit: Duration::from_secs(1),
            max_retransmit: Duration::from_secs(4),
            min_renew_retry: Duration::from_secs(1),
            ..Timing::default()
        },
        seed: 1,
        ..DhcpConfig::default()
    };
    let t0 = Instant::now();
    let mut hosts = DhcpHosts::new(&pool, &block, &config, Duration::ZERO).unwrap();
    let mut responder = responder_for(&pool, &hosts);
    let mut seen = hosts.leases().generation();
    let mut at_bound: Option<(u64, Vec<Option<Ipv4Addr>>)> = None;
    loop {
        let now = t0.elapsed();
        assert!(now < Duration::from_secs(60), "no renewal after {now:?}");
        if hosts.leases().generation() != seen {
            responder = responder_for(&pool, &hosts);
            seen = hosts.leases().generation();
        }
        let mut svc = NeighborService::new(&responder, 64);
        svc.poll(&mut queue, &mut |f| {
            hosts.handle_frame(f, now);
        })
        .unwrap();
        hosts.send_due(&mut queue, now).unwrap();
        if at_bound.is_none() && hosts.leases().is_complete() {
            let addrs: Vec<Option<Ipv4Addr>> =
                (0..HOSTS).map(|i| hosts.leases().addr_of(i)).collect();
            let text: Vec<String> = addrs.iter().flatten().map(ToString::to_string).collect();
            say(&format!(
                "bound count={} addrs={}",
                hosts.leases().bound(),
                text.join(",")
            ));
            at_bound = Some((hosts.leases().generation(), addrs));
        }
        if at_bound.is_some() && hosts.stats().renewed >= u64::from(HOSTS) {
            break;
        }
        let _ = queue.wait_readable(Duration::from_millis(5));
    }
    let (generation, addrs) = at_bound.unwrap();
    let now_addrs: Vec<Option<Ipv4Addr>> = (0..HOSTS).map(|i| hosts.leases().addr_of(i)).collect();
    let stats = hosts.stats();
    say(&format!(
        "renewed count={} lost={} same_addresses={} same_generation={}",
        stats.renewed,
        stats.lost,
        now_addrs == addrs,
        hosts.leases().generation() == generation
    ));
    let now = t0.elapsed();
    let made = hosts.release_all(&mut queue, now).unwrap();
    let stats = hosts.stats();
    say(&format!(
        "released n={made} discovers={} requests={} renews={} rebinds={} foreign={} malformed={}",
        stats.discovers,
        stats.requests,
        stats.renews,
        stats.rebinds,
        stats.foreign,
        stats.malformed
    ));
}
