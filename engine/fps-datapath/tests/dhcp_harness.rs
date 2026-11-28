// SPDX-License-Identifier: GPL-3.0-only
//! The datapath leases its client addresses in the QA-01 namespace harness
//! and runs own stack sessions from them through the router (TRN-37).
//!
//! The test brings up the harness (`tests/harness/netns.sh`), moves the
//! interface of the server arm into the client namespace as `eth1`, so that
//! one process owns both arms of a lane, and starts this test binary twice
//! with `ip netns exec`, through the hidden test `helper_process`:
//!
//! ```text
//!   router namespace                      client namespace (the datapath)
//!     DHCP server on cli0 (10.201.0.1) --- eth0  client pool, 8 hosts, dhcp4
//!     forwards 10.201.0.0/24 <-> 10.202.0.0/24
//!                                      --- eth1  server pool 10.202.0.0/24, 4 hosts
//! ```
//!
//! The datapath runs with next hop resolution on and a client pool that
//! leases. Nothing names the router: the gateway of the client arm comes from
//! the lease (`routers` option), and the static server pool names 10.202.0.1.
//! The helper waits for the lease phase, checks that all eight leases are
//! bound, and opens one session of 64 KiB from every leased address to a
//! server host, which echoes it. No session may fail.
//!
//! The harness needs root. As a normal user the test runs itself again through
//! `sudo -n` and skips with a message when that is not possible. `CI=true` or
//! `FPS_REQUIRE_AFXDP=1` turns every skip into a failure.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use fps_backend_afxdp::{open_port, BindMode, FrameIo, OpenedPort, PortConfig, QueueConfig};
use fps_datapath::{Datapath, DatapathConfig, DhcpSettings, Lane, NeighborConfig};
use fps_hosts::dhcp::{Message, MessageType, Timing, CLIENT_PORT, SERVER_PORT};
use fps_hosts::{DhcpConfig, HostPool, PoolSpec};
use fps_tcp_own::{HostSlice, Pools};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The marker of the lines that helper processes report on.
const MARK: &str = "fps-dpdhcp ";
const HOSTS: u32 = 8;
const SERVERS: u32 = 4;
const MAC_BASE: &str = "02:aa:00:00:00:00";
const LEASE_SECS: u32 = 300;
const PORT: u16 = 9400;
const BYTES: usize = 64 * 1024;
const TEST: &str = "a_datapath_leases_its_client_addresses_and_runs_sessions_through_the_router";

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
        || std::env::var("FPS_REQUIRE_AFXDP").is_ok_and(|v| v == "1")
}

fn skip(why: &str) {
    assert!(
        !required(),
        "the datapath DHCP harness test cannot run here and CI requires it: {why}"
    );
    eprintln!("skipping the datapath DHCP harness test: {why}");
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
        let state_dir = std::env::temp_dir().join(format!("fps-dpdhcp-netns-{prefix}"));
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

fn pattern(seed: u8) -> Vec<u8> {
    (0..BYTES).map(|i| (i % 251) as u8 ^ seed).collect()
}

fn pools() -> Pools {
    let clients = HostPool::dhcp("clients", HOSTS, Some(MAC_BASE)).unwrap();
    let servers = HostPool::parse(&PoolSpec {
        name: "servers",
        ipv4: Some("10.202.0.0/24"),
        count: SERVERS,
        gateway: Some("10.202.0.1"),
        ..PoolSpec::default()
    })
    .unwrap();
    Pools {
        client: HostSlice::whole(clients),
        server: HostSlice::whole(servers),
        vlan: None,
    }
}

fn open(interface: &str) -> Result<OpenedPort, String> {
    let config = PortConfig {
        interface: interface.to_owned(),
        queues: vec![0],
        queue: QueueConfig {
            mode: BindMode::Auto,
            ..QueueConfig::default()
        },
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match open_port(&config) {
            Ok(p) => return Ok(p),
            Err(e) if e.raw_os_error() == Some(16) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) if e.raw_os_error() == Some(97) => {
                return Err(format!("unsupported AF_XDP: {e}"))
            }
            Err(e) => panic!("opening {interface}: {e:?}"),
        }
    }
}

/// The datapath in the client namespace: lease phase, then one echo session
/// from every leased address.
fn helper_client() {
    let mut a = match open("eth0") {
        Ok(p) => p,
        Err(e) => return say(&e),
    };
    let mut b = match open("eth1") {
        Ok(p) => p,
        Err(e) => return say(&e),
    };
    say("opened");
    let lane = Lane {
        client: Box::new(a.queues.remove(0)) as Box<dyn FrameIo>,
        server: Box::new(b.queues.remove(0)) as Box<dyn FrameIo>,
        pools: pools(),
    };
    let config = DatapathConfig {
        neighbors: Some(NeighborConfig::default()),
        dhcp: Some(DhcpSettings {
            config: DhcpConfig {
                timing: Timing {
                    start_delay_max: Duration::from_millis(300),
                    initial_retransmit: Duration::from_secs(1),
                    max_retransmit: Duration::from_secs(4),
                    ..Timing::default()
                },
                seed: 1,
                ..DhcpConfig::default()
            },
            timeout: Duration::from_secs(30),
        }),
        ..DatapathConfig::default()
    };
    let dp = Datapath::start(vec![lane], config).unwrap();
    let report = match dp.wait_leases() {
        Ok(r) => r,
        Err(e) => {
            say(&format!("lease-error {e}"));
            panic!("{e}");
        }
    };
    say(&format!(
        "bound count={} hosts={}",
        report.bound(),
        report.hosts()
    ));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let pools = pools();
    let (ok, failed) = {
        let _guard = runtime.enter();
        runtime.block_on(async {
            let mut listener = dp.listen(PORT).unwrap();
            tokio::spawn(async move {
                while let Ok((mut s, _)) = listener.accept().await {
                    tokio::spawn(async move {
                        let mut buf = vec![0u8; BYTES];
                        if s.read_exact(&mut buf).await.is_err() {
                            return;
                        }
                        let _ = s.write_all(&buf).await;
                        let _ = s.shutdown().await;
                        let mut rest = Vec::new();
                        let _ = s.read_to_end(&mut rest).await;
                    });
                }
            });
            let connector = dp.connector();
            let (mut ok, mut failed) = (0u32, 0u32);
            for i in 0..HOSTS {
                let source = IpAddr::V4(Ipv4Addr::new(10, 201, 0, 100 + u8::try_from(i).unwrap()));
                let to = SocketAddr::new(pools.server.ip(i % SERVERS).unwrap(), PORT);
                let seed = u8::try_from(i).unwrap();
                let run = async {
                    let mut s = connector
                        .connect(source, to)
                        .await
                        .map_err(|e| e.to_string())?;
                    s.write_all(&pattern(seed))
                        .await
                        .map_err(|e| e.to_string())?;
                    let mut back = vec![0u8; BYTES];
                    s.read_exact(&mut back).await.map_err(|e| e.to_string())?;
                    if back == pattern(seed) {
                        Ok(())
                    } else {
                        Err("the echo differs".to_owned())
                    }
                };
                match tokio::time::timeout(Duration::from_secs(20), run).await {
                    Ok(Ok(())) => ok += 1,
                    Ok(Err(e)) => {
                        eprintln!("{MARK}session {i} from {source} failed: {e}");
                        failed += 1;
                    }
                    Err(_) => {
                        eprintln!("{MARK}session {i} from {source} timed out");
                        failed += 1;
                    }
                }
            }
            (ok, failed)
        })
    };
    let stats = dp.stats().remove(0);
    let reports = dp.stop();
    let c = stats.client;
    say(&format!(
        "sessions ok={ok} failed={failed} dhcp_handled={} dhcp_unsupported={} arp_replied={} \
         resolved={} failed_resolutions={} panicked={}",
        c.dhcp_handled,
        c.dhcp_unsupported,
        c.arp_replied,
        c.neighbor_resolved,
        c.neighbor_failed,
        u8::from(reports[0].panicked)
    ));
}

#[test]
fn a_datapath_leases_its_client_addresses_and_runs_sessions_through_the_router() {
    if !is_root() {
        if run("sudo", &["-n", "true"]).is_err() {
            return skip("not root and no passwordless sudo");
        }
        let exe = std::env::current_exe().unwrap();
        let require = format!("FPS_REQUIRE_AFXDP={}", u8::from(required()));
        let status = Command::new("sudo")
            .args(["-n", "env", &require])
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
    let prefix = format!("dpd{}", std::process::id() % 100_000);
    let h = match Harness::up(&prefix) {
        Ok(h) => h,
        Err(e) => return skip(&format!("the namespace harness did not come up: {e}")),
    };
    let (client, router, server) = (h.ns("client"), h.ns("router"), h.ns("server"));
    // The hosts of the pools own both arms. Move the server arm into the
    // client namespace, and remove what the harness configured.
    run(
        "ip",
        &[
            "-n", &server, "link", "set", "eth0", "netns", &client, "name", "eth1",
        ],
    )
    .unwrap();
    for dev in ["eth0", "eth1"] {
        run("ip", &["-n", &client, "route", "flush", "dev", dev]).unwrap();
        run("ip", &["-n", &client, "addr", "flush", "dev", dev]).unwrap();
        run("ip", &["-n", &client, "link", "set", dev, "up"]).unwrap();
    }

    let mut dhcp = Helper::start(
        &router,
        &[
            ("FPS_DPDHCP_ROLE", "server"),
            ("FPS_DPDHCP_IFACE", "cli0"),
            ("FPS_DPDHCP_SERVER_IP", "10.201.0.1"),
        ],
    );
    dhcp.wait_line("listening", Duration::from_secs(20));
    let mut own = Helper::start(&client, &[("FPS_DPDHCP_ROLE", "client")]);
    let first = own.wait_line("", Duration::from_secs(20));
    if first.starts_with("unsupported") {
        return skip(&format!("the helper could not open AF_XDP: {first}"));
    }
    let bound = own.wait_line("bound", Duration::from_secs(40));
    assert_eq!(field(&bound, "count"), HOSTS.to_string(), "{bound}");
    assert_eq!(field(&bound, "hosts"), HOSTS.to_string(), "{bound}");
    let done = own.wait_line("sessions", Duration::from_secs(120));
    assert_eq!(field(&done, "ok"), HOSTS.to_string(), "{done}");
    assert_eq!(field(&done, "failed"), "0", "{done}");
    assert_eq!(field(&done, "dhcp_unsupported"), "0", "{done}");
    assert_eq!(field(&done, "failed_resolutions"), "0", "{done}");
    assert_eq!(field(&done, "panicked"), "0", "{done}");
    let (ok, lines) = own.finish();
    assert!(ok, "the datapath helper failed: {lines:?}");

    // The router knows the MAC address of every leased host.
    let neigh = run("ip", &["-n", &router, "-4", "neigh", "show", "dev", "cli0"]).unwrap();
    for i in 0..HOSTS {
        let want = format!("10.201.0.{} ", 100 + i);
        assert!(
            neigh.lines().any(|l| l.starts_with(&want)),
            "the router has no neighbor entry for {want}: {neigh}"
        );
    }
    let (ok, lines) = dhcp.finish();
    assert!(ok, "the DHCP server helper failed: {lines:?}");
}

#[test]
#[ignore = "started by the harness test inside a namespace"]
fn helper_process() {
    let Ok(role) = std::env::var("FPS_DPDHCP_ROLE") else {
        return;
    };
    match role.as_str() {
        "server" => helper_server(
            &env_var("FPS_DPDHCP_IFACE"),
            &env_var("FPS_DPDHCP_SERVER_IP"),
        ),
        "client" => helper_client(),
        other => panic!("unknown role {other}"),
    }
}

fn say(msg: &str) {
    println!("{MARK}{msg}");
    let _ = std::io::stdout().flush();
}

fn env_var(key: &str) -> String {
    std::env::var(key).unwrap_or_default()
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
