// SPDX-License-Identifier: GPL-3.0-only
//! `contracts/examples/tests/dhcp_clients.yaml` runs through an agent forced
//! onto AF_XDP, with a DHCP server in the router namespace (AGT-23, bug #708).
//!
//! Two network namespaces, as in `afxdp_router.rs` (AGT-26):
//!
//! ```text
//!   own namespace (the agent, AF_XDP on eth1 and eth2)       router namespace
//!     clients, 200 hosts, dhcp4          eth1 ---- r1  10.201.0.1/24  DHCP server
//!     servers 48.0.0.0/24, 1 host        eth2 ---- r2  48.0.0.254/24  (forwarding)
//! ```
//!
//! The own namespace holds no address. The plan is the file as the controller
//! compiles it (`tests/common`), with its phases shortened and its objective
//! lowered for the runner, and nothing else changed. The interfaces are the
//! `local/eth1` and `local/eth2` of the file.
//!
//! A helper in the router namespace runs the DHCP server (a host gets the next
//! free address of 10.201.0.10 to 10.201.0.209 and the router 10.201.0.1) and
//! a packet socket on `r2`, which records the source of every TCP SYN to the
//! server host. The test checks that:
//!
//! * the run passes with no session failed,
//! * the server leased every client and the last of them had its first ACK
//!   before the first SYN reached the server arm (every address is leased
//!   before traffic starts),
//! * every client address that the server side saw is one that the DHCP server
//!   leased, and none lies in 240.0.0.0/4, the virtual pool of the agent.
//!
//! The test needs root, `ip`, `sysctl`, and AF_XDP. As a normal user it runs
//! itself again through `sudo -n` and skips with a message when that is not
//! possible. `CI=true` or `FPS_REQUIRE_AFXDP=1` turns every skip into a
//! failure.

mod common;

use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use fps_agent::capabilities::AfXdpSupport;
use fps_agent::{detect, run, Outcome, RunOptions, SystemRoot};
use fps_hosts::dhcp::{Message, MessageType, CLIENT_PORT, SERVER_PORT};
use fps_stats::{metric, MetricId, SeriesValue, Snapshot};
use socket2::{Domain, Protocol, Socket, Type};

use common::{modules_of, plan_in};

const MARK: &str = "fps-agent-dhcp ";
const TEST: &str = "dhcp_clients_runs_on_a_forced_af_xdp_agent_and_leases_every_address";
const CLIENTS: usize = 200;
const SERVER_HOST: Ipv4Addr = Ipv4Addr::new(48, 0, 0, 1);

fn run_cmd(cmd: &str, args: &[&str]) -> Result<String, String> {
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
        || std::env::var("FPS_REQUIRE_AFXDP").is_ok_and(|v| v == "1")
}

fn skip(why: &str) {
    assert!(
        !required(),
        "the agent DHCP test cannot run here and CI requires it: {why}"
    );
    eprintln!("skipping the agent DHCP test: {why}");
}

fn say(msg: &str) {
    println!("{MARK}{msg}");
    let _ = std::io::stdout().flush();
}

/// The two namespaces, which `Drop` removes again.
struct Topology {
    own: String,
    router: String,
}

impl Topology {
    fn up(tag: &str) -> Result<Self, String> {
        let t = Self {
            own: format!("a23o{tag}"),
            router: format!("a23r{tag}"),
        };
        for ns in [&t.own, &t.router] {
            run_cmd("ip", &["netns", "add", ns])?;
        }
        let (o, r) = (t.own.as_str(), t.router.as_str());
        for (own_end, router_end) in [("eth1", "r1"), ("eth2", "r2")] {
            run_cmd(
                "ip",
                &[
                    "-n", o, "link", "add", own_end, "type", "veth", "peer", "name", router_end,
                    "netns", r,
                ],
            )?;
        }
        for (ns, dev) in [(o, "eth1"), (o, "eth2"), (r, "r1"), (r, "r2")] {
            run_cmd("ip", &["-n", ns, "link", "set", dev, "up"])?;
        }
        for (dev, address) in [("r1", "10.201.0.1/24"), ("r2", "48.0.0.254/24")] {
            run_cmd("ip", &["-n", r, "addr", "add", address, "dev", dev])?;
        }
        // The DHCP server answers a client that has no address by limited
        // broadcast, which needs a route out of the client arm.
        run_cmd(
            "ip",
            &["-n", r, "route", "add", "255.255.255.255/32", "dev", "r1"],
        )?;
        run_cmd(
            "ip",
            &[
                "netns",
                "exec",
                r,
                "sysctl",
                "-q",
                "-w",
                "net.ipv4.ip_forward=1",
            ],
        )?;
        Ok(t)
    }
}

impl Drop for Topology {
    fn drop(&mut self) {
        for ns in [&self.own, &self.router] {
            let _ = run_cmd("ip", &["netns", "del", ns]);
        }
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
    fn start(ns: &str, role: &str) -> Self {
        let exe = std::env::current_exe().expect("the test binary");
        let mut child = Command::new("ip")
            .args(["netns", "exec", ns])
            .arg(exe)
            .args([
                "--ignored",
                "--exact",
                "helper_process",
                "--nocapture",
                "--test-threads",
                "1",
            ])
            .env("FPS_AGENT_DHCP_ROLE", role)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("ip netns exec starts");
        let stdout = child.stdout.take().expect("piped stdout");
        let stdin = child.stdin.take();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                // libtest prints the test name before the first line.
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

    fn wait_line(&mut self, prefix: &str, timeout: Duration) -> Option<String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(l) = self.seen.iter().find(|l| l.starts_with(prefix)) {
                return Some(l.clone());
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left.max(Duration::from_millis(1))) {
                Ok(l) => self.seen.push(l),
                Err(_) => return None,
            }
        }
    }

    /// Closes stdin, which tells the router helper to stop, and collects the
    /// remaining lines and the exit status.
    fn finish(mut self) -> (bool, Vec<String>) {
        drop(self.stdin.take());
        let status = self.child.wait().expect("the helper ends");
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

fn addresses(line: &str, key: &str) -> BTreeSet<Ipv4Addr> {
    field(line, key)
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().unwrap_or_else(|_| panic!("{s} is no address")))
        .collect()
}

#[test]
fn dhcp_clients_runs_on_a_forced_af_xdp_agent_and_leases_every_address() {
    let root = run_cmd("id", &["-u"]).is_ok_and(|s| s.trim() == "0");
    if !root {
        if run_cmd("sudo", &["-n", "true"]).is_err() {
            return skip("not root and no passwordless sudo");
        }
        let exe = std::env::current_exe().expect("the test binary");
        let mut cmd = Command::new("sudo");
        cmd.args(["-n", "env"]);
        for key in ["CI", "FPS_REQUIRE_AFXDP"] {
            if let Ok(v) = std::env::var(key) {
                cmd.arg(format!("{key}={v}"));
            }
        }
        let status = cmd
            .arg(exe)
            .args(["--exact", TEST, "--nocapture"])
            .status()
            .expect("sudo starts");
        assert!(status.success(), "the test failed under sudo");
        return;
    }
    if let Err(e) = run_cmd("ip", &["-V"]) {
        return skip(&e);
    }
    match detect(&SystemRoot::host()).map(|h| h.af_xdp) {
        Ok(AfXdpSupport::Unsupported(_)) => return skip("the kernel has no AF_XDP sockets"),
        Ok(_) => {}
        Err(e) => return skip(&format!("capability detection failed: {e}")),
    }
    let tag = format!("{}", std::process::id() % 100_000);
    let topology = match Topology::up(&tag) {
        Ok(t) => t,
        Err(e) => return skip(&format!("the namespaces did not come up: {e}")),
    };

    let mut router = Helper::start(&topology.router, "router");
    router
        .wait_line("listening", Duration::from_secs(20))
        .expect("the router helper listens");
    let mut own = Helper::start(&topology.own, "run");
    let done = own.wait_line("run ", Duration::from_secs(180));
    let (own_ok, own_lines) = own.finish();
    let done = done.unwrap_or_else(|| panic!("the run reported nothing: {own_lines:?}"));
    assert!(own_ok, "the agent helper failed: {own_lines:?}");
    assert!(done.contains("outcome=pass"), "{done}");
    assert_eq!(field(&done, "failed"), "0", "{done}");
    assert!(field(&done, "ok").parse::<u64>().unwrap() > 0, "{done}");

    let (router_ok, router_lines) = router.finish();
    assert!(router_ok, "the router helper failed: {router_lines:?}");
    let server = router_lines
        .iter()
        .find(|l| l.starts_with("server "))
        .unwrap_or_else(|| panic!("no server line in {router_lines:?}"));
    let seen = router_lines
        .iter()
        .find(|l| l.starts_with("observer "))
        .unwrap_or_else(|| panic!("no observer line in {router_lines:?}"));
    assert_eq!(field(server, "leased"), CLIENTS.to_string(), "{server}");
    let leased = addresses(server, "addrs");
    let sources = addresses(seen, "srcs");
    assert!(!sources.is_empty(), "no SYN reached the server arm: {seen}");
    for s in &sources {
        assert!(leased.contains(s), "the server side saw {s}, not leased");
        assert!(
            s.octets()[0] < 240,
            "the server side saw {s}, in 240.0.0.0/4"
        );
    }
    // Every address is leased before traffic starts.
    let last_ack: u64 = field(server, "last_first_ack_ms").parse().unwrap();
    let first_syn: u64 = field(seen, "first_syn_ms").parse().unwrap();
    assert!(
        last_ack < first_syn,
        "the last lease ({last_ack} ms) came after the first session ({first_syn} ms)"
    );
}

// ---------------------------------------------------------------------------
// The helpers
// ---------------------------------------------------------------------------

#[test]
#[ignore = "started by the agent DHCP test inside a namespace"]
fn helper_process() {
    match std::env::var("FPS_AGENT_DHCP_ROLE").as_deref() {
        Ok("router") => helper_router(),
        Ok("run") => helper_run(),
        _ => {}
    }
}

/// The counter of `name` for `application` on `side` of one snapshot.
fn count(total: &Snapshot, name: MetricId, application: &str, side: &str) -> u64 {
    total
        .iter()
        .filter(|(k, _)| {
            k.metric() == name
                && k.labels()
                    .contains(&("application", application.to_owned()))
                && k.labels().contains(&("side", side.to_owned()))
        })
        .map(|(_, v)| match v {
            SeriesValue::Count(n) => *n,
            _ => 0,
        })
        .sum()
}

/// The file as the controller compiles it, with the phases shortened and the
/// objective lowered for the runner, and nothing else changed.
fn dhcp_plan(modules: &Path) -> serde_json::Value {
    let mut plan = plan_in("dhcp_clients.yaml", modules);
    plan["backend"] = "BACKEND_AF_XDP".into();
    plan["workers"][0]["rates"][0]["sessionsPerSecond"] = 20.into();
    plan["load"]["phases"] = serde_json::json!([
        { "name": "steady", "duration": "4s", "shape": "PHASE_SHAPE_HOLD",
          "startFraction": 1, "endFraction": 1 }
    ]);
    plan["warmup"] = "0s".into();
    plan
}

fn helper_run() {
    let modules = modules_of("dhcp_clients.yaml");
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fps-agent-dhcp");
    std::fs::create_dir_all(&dir).expect("plan dir");
    let path = dir.join("dhcp_clients_plan.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&dhcp_plan(&modules)).unwrap(),
    )
    .unwrap();
    let opts = RunOptions::new(path, modules);
    let mut out = Vec::new();
    let mut log = Vec::new();
    let summary = run(&opts, &mut out, &mut log);
    let log = String::from_utf8_lossy(&log).into_owned();
    for line in log.lines() {
        eprintln!("{MARK}log {line}");
    }
    let summary = summary.expect("the run on the datapath completes");
    let client = summary.client.as_ref().expect("client totals");
    let attempted = count(client, metric::SESSIONS_ATTEMPTED, "http1", "client");
    let ok = count(client, metric::SESSIONS_SUCCEEDED, "http1", "client");
    let failed = client.counter_total(metric::SESSIONS_FAILED);
    let dhcp: u64 = summary
        .lanes
        .iter()
        .map(|l| l.stats.client.dhcp_handled)
        .sum();
    say(&format!(
        "run outcome={} attempted={attempted} ok={ok} failed={failed} dhcp_handled={dhcp}",
        if summary.outcome == Outcome::Pass {
            "pass"
        } else {
            "fail"
        }
    ));
    assert_eq!(summary.outcome, Outcome::Pass, "{log}");
}

/// The router namespace: a DHCP server on `r1` and a packet socket on `r2`.
fn helper_router() {
    let start = Instant::now();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    std::thread::spawn(move || {
        // The parent closes our stdin when the run is over.
        let mut sink = Vec::new();
        let _ = std::io::stdin().read_to_end(&mut sink);
        flag.store(true, Ordering::SeqCst);
    });
    let seen: Arc<Mutex<(BTreeSet<Ipv4Addr>, Option<u64>)>> =
        Arc::new(Mutex::new((BTreeSet::new(), None)));
    let observer = {
        let (stop, seen) = (Arc::clone(&stop), Arc::clone(&seen));
        let ready = Arc::new(AtomicBool::new(false));
        let ready2 = Arc::clone(&ready);
        let handle = std::thread::spawn(move || observe(&stop, &seen, &ready2, start));
        while !ready.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        handle
    };
    let (leased, last_ack) = dhcp_server(&stop, start);
    observer.join().expect("the observer ends");
    let (sources, first_syn) = seen.lock().unwrap().clone();
    say(&format!(
        "server leased={} last_first_ack_ms={last_ack} addrs={}",
        leased.len(),
        join(leased.values())
    ));
    say(&format!(
        "observer first_syn_ms={} srcs={}",
        first_syn.unwrap_or(u64::MAX),
        join(sources.iter())
    ));
}

fn join<'a>(addrs: impl Iterator<Item = &'a Ipv4Addr>) -> String {
    addrs.map(ToString::to_string).collect::<Vec<_>>().join(",")
}

/// Records the source of every TCP SYN that the router sends to the server
/// host, in the order of arrival of the first one.
fn observe(
    stop: &AtomicBool,
    seen: &Mutex<(BTreeSet<Ipv4Addr>, Option<u64>)>,
    ready: &AtomicBool,
    start: Instant,
) {
    // ETH_P_ALL in network byte order.
    let all = i32::from(3u16.to_be());
    let sock = Socket::new(Domain::PACKET, Type::RAW, Some(Protocol::from(all)))
        .expect("a packet socket needs root");
    sock.bind_device(Some(b"r2")).expect("bind r2");
    sock.set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    ready.store(true, Ordering::SeqCst);
    let mut buf = [0u8; 2048];
    while !stop.load(Ordering::SeqCst) {
        let Ok(n) = (&sock).read(&mut buf) else {
            continue;
        };
        if let Some(src) = syn_to_server(&buf[..n]) {
            let mut seen = seen.lock().unwrap();
            seen.0.insert(src);
            seen.1
                .get_or_insert(u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX));
        }
    }
}

/// The source of an IPv4 TCP SYN (without ACK) to the server host.
fn syn_to_server(f: &[u8]) -> Option<Ipv4Addr> {
    if f.get(12..14)? != [0x08, 0x00] {
        return None;
    }
    let ip = f.get(14..)?;
    let ihl = usize::from(ip.first()? & 0x0f) * 4;
    if *ip.get(9)? != 6 {
        return None;
    }
    let src = Ipv4Addr::from(<[u8; 4]>::try_from(ip.get(12..16)?).ok()?);
    let dst = Ipv4Addr::from(<[u8; 4]>::try_from(ip.get(16..20)?).ok()?);
    let flags = *ip.get(ihl + 13)?;
    (dst == SERVER_HOST && flags & 0x12 == 0x02).then_some(src)
}

/// A DHCP server on `r1`: the next free address of 10.201.0.10 on for each
/// client MAC, the router 10.201.0.1, a /24 mask. Returns the leases by MAC
/// and the time of the first ACK of the last client to get one.
fn dhcp_server(stop: &AtomicBool, start: Instant) -> (HashMap<[u8; 6], Ipv4Addr>, u64) {
    let server_ip = Ipv4Addr::new(10, 201, 0, 1);
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();
    sock.set_reuse_address(true).unwrap();
    sock.set_broadcast(true).unwrap();
    sock.bind_device(Some(b"r1")).unwrap();
    sock.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, SERVER_PORT)).into())
        .unwrap();
    let sock: UdpSocket = sock.into();
    sock.set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    say("listening");
    let mut leases: HashMap<[u8; 6], Ipv4Addr> = HashMap::new();
    let mut acked: BTreeSet<[u8; 6]> = BTreeSet::new();
    let mut last_first_ack = 0u64;
    let mut buf = [0u8; 1500];
    while !stop.load(Ordering::SeqCst) {
        let Ok((n, _)) = sock.recv_from(&mut buf) else {
            continue;
        };
        let Ok(req) = Message::parse(&buf[..n]) else {
            continue;
        };
        let mac = req.chaddr.0;
        let next = u8::try_from(10 + leases.len()).expect("fewer than 246 clients");
        let addr = *leases
            .entry(mac)
            .or_insert_with(|| Ipv4Addr::new(10, 201, 0, next));
        let kind = match req.kind {
            MessageType::Discover => MessageType::Offer,
            MessageType::Request => {
                if acked.insert(mac) {
                    last_first_ack = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
                }
                MessageType::Ack
            }
            _ => continue,
        };
        let mut reply = Message::new(kind, req.xid, req.chaddr);
        reply.yiaddr = addr;
        reply.options.server_id = Some(server_ip);
        reply.options.lease_secs = Some(300);
        reply.options.subnet_mask = Some(Ipv4Addr::new(255, 255, 255, 0));
        reply.options.routers = vec![server_ip];
        let to = if req.wants_broadcast() {
            Ipv4Addr::BROADCAST
        } else {
            req.ciaddr
        };
        let _ = sock.send_to(&reply.encode(), SocketAddr::from((to, CLIENT_PORT)));
    }
    (leases, last_first_ack)
}
