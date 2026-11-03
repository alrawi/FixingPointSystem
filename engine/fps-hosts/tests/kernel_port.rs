// SPDX-License-Identifier: GPL-3.0-only
//! Integration tests of kernel port setup in the QA-01 namespace harness.
//!
//! The tests need root, iproute2, and `sysctl`, so they are ignored by
//! default. `make e2e-hst` runs them as root with `--ignored`. Each test brings
//! up its own harness under its own name prefix, hands the bare client arm
//! interface to port setup, opens TCP sessions from every pool address of the
//! port namespace to a server in the server namespace, and checks that
//! teardown leaves the client namespace and this namespace as they were.
//!
//! The client and server run as child processes of this test binary inside
//! the namespaces, started with `ip netns exec` and the hidden test
//! `helper_process`, so the test needs no `setns` and no unsafe code.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fps_hosts::{
    cleanup, setup, HostPool, InterfaceAddr, IpCommand, PoolSpec, PortError, PortSpec,
};
use socket2::{Domain, Protocol, Socket, Type};

const SERVER_PORT: u16 = 9000;
/// The marker of the lines that the helper processes report on.
const MARK: &str = "fps-hst ";

fn harness_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/harness/netns.sh")
}

fn run(cmd: &str, args: &[&str]) -> String {
    let out = Command::new(cmd).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "{cmd} {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn ip(args: &[&str]) -> String {
    run("ip", args)
}

fn require_root() {
    let uid = run("id", &["-u"]);
    assert_eq!(
        uid.trim(),
        "0",
        "the kernel port tests need root, run make e2e-hst as root"
    );
}

/// One QA-01 harness, which `Drop` takes down again.
struct Harness {
    prefix: &'static str,
    state_dir: PathBuf,
}

impl Harness {
    /// Brings the harness up, or returns the error output of `netns.sh up`.
    fn up(prefix: &'static str, env: &[(&str, &str)]) -> Result<Self, String> {
        let state_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("netns-{prefix}"));
        let mut cmd = Command::new(harness_script());
        cmd.arg("up")
            .env("FPS_NETNS", prefix)
            .env("FPS_STATE_DIR", &state_dir);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().unwrap();
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        Ok(Self { prefix, state_dir })
    }

    fn ns(&self, arm: &str) -> String {
        format!("{}-{arm}", self.prefix)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = Command::new(harness_script())
            .arg("down")
            .env("FPS_NETNS", self.prefix)
            .env("FPS_STATE_DIR", &self.state_dir)
            .status();
    }
}

/// Removes the port when a test fails between setup and teardown, before the
/// harness goes down. Locals drop in reverse order, so a guard declared after
/// the harness runs first.
struct Cleanup(PortSpec);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = cleanup(&self.0, &mut IpCommand::new());
    }
}

/// Links, addresses, and routes of one namespace, or of this namespace for
/// `None`, as text that teardown must restore.
fn snapshot(ns: Option<&str>) -> String {
    let mut out = String::new();
    for args in [
        &["-o", "link", "show"][..],
        &["-o", "addr", "show"],
        &["-4", "route", "show", "table", "all"],
        &["-6", "route", "show", "table", "all"],
    ] {
        let mut full: Vec<&str> = Vec::new();
        if let Some(ns) = ns {
            full.extend(["-n", ns]);
        }
        full.extend_from_slice(args);
        let text = Command::new("ip").args(&full).output().unwrap();
        out.push_str(&String::from_utf8_lossy(&text.stdout));
    }
    out
}

/// Waits up to three seconds for the snapshot of `ns` to equal `want`. A veth
/// that moves back reports no carrier for a moment after it comes up.
fn settled_snapshot(ns: Option<&str>, want: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let got = snapshot(ns);
        if got == want || Instant::now() > deadline {
            return got;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn namespaces() -> HashSet<String> {
    ip(&["netns", "list"])
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_owned)
        .collect()
}

/// Starts this test binary as a helper in namespace `ns`.
fn helper(ns: &str, role: &str, env: &[(&str, String)]) -> Child {
    let exe = std::env::current_exe().unwrap();
    let mut cmd = Command::new("ip");
    cmd.args(["netns", "exec", ns])
        .arg(exe)
        .args(["--exact", "helper_process", "--ignored", "--nocapture"])
        .args(["--test-threads", "1"])
        .env("FPS_HST_ROLE", role)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.spawn().unwrap()
}

/// Reads helper lines until one holds `MARK` followed by `word`.
fn wait_line(lines: &mut impl Iterator<Item = std::io::Result<String>>, word: &str) -> String {
    for line in lines {
        let line = line.unwrap();
        // libtest prints the test name on the same line before the output.
        if let Some(at) = line.find(MARK) {
            let rest = &line[at + MARK.len()..];
            if rest.starts_with(word) {
                return rest.to_owned();
            }
        }
    }
    panic!("helper exited before reporting {word}");
}

fn field(line: &str, key: &str) -> u64 {
    line.split_whitespace()
        .find_map(|kv| kv.strip_prefix(&format!("{key}=")))
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("line '{line}' has no field {key}"))
}

/// The pool settings that both helpers read from the environment.
fn pool_env(prefix: &str, count: u32, gateway: &str) -> Vec<(&'static str, String)> {
    vec![
        ("FPS_HST_POOL", prefix.to_owned()),
        ("FPS_HST_COUNT", count.to_string()),
        ("FPS_HST_GATEWAY", gateway.to_owned()),
    ]
}

fn pool_from_env() -> HostPool {
    let prefix = std::env::var("FPS_HST_POOL").unwrap();
    let gateway = std::env::var("FPS_HST_GATEWAY").unwrap();
    let count: u32 = std::env::var("FPS_HST_COUNT").unwrap().parse().unwrap();
    HostPool::parse(&PoolSpec {
        name: "clients",
        ipv4: Some(&prefix),
        count,
        gateway: Some(&gateway),
        ..PoolSpec::default()
    })
    .unwrap()
}

/// Runs the server and the client helpers and checks that every pool address
/// opened one session that the server saw from that address.
fn sessions(port_ns: &str, server_ns: &str, server: Ipv4Addr, env: &[(&'static str, String)]) {
    let mut env = env.to_vec();
    env.push(("FPS_HST_SERVER", format!("{server}:{SERVER_PORT}")));
    let mut srv = helper(server_ns, "server", &env);
    let mut srv_lines = BufReader::new(srv.stdout.take().unwrap()).lines();
    wait_line(&mut srv_lines, "listening");
    let started = Instant::now();
    let mut cli = helper(port_ns, "client", &env);
    let mut cli_lines = BufReader::new(cli.stdout.take().unwrap()).lines();
    let client = wait_line(&mut cli_lines, "client");
    let server = wait_line(&mut srv_lines, "server");
    let elapsed = started.elapsed();
    assert!(
        cli.wait().unwrap().success(),
        "client helper failed: {client}"
    );
    assert!(
        srv.wait().unwrap().success(),
        "server helper failed: {server}"
    );
    let count = u64::from(pool_from_env_count(&env));
    println!("{count} sessions in {elapsed:?}: {client} | {server}");
    assert_eq!(field(&client, "ok"), count, "{client}");
    assert_eq!(field(&client, "failed"), 0, "{client}");
    assert_eq!(field(&server, "accepted"), count, "{server}");
    assert_eq!(field(&server, "distinct"), count, "{server}");
    assert_eq!(field(&server, "mismatched"), 0, "{server}");
}

fn pool_from_env_count(env: &[(&'static str, String)]) -> u32 {
    env.iter()
        .find(|(k, _)| *k == "FPS_HST_COUNT")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap()
}

fn sysctl_in(ns: &str, key: &str) -> String {
    let out = ip(&["netns", "exec", ns, "sysctl", "-n", key]);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A routed port: the router sends the pool prefix to the port address, so
/// 20,000 hosts need one neighbor entry on the router.
#[test]
#[ignore = "needs root and the QA-01 harness, run make e2e-hst"]
fn routed_port_binds_and_connects_from_20000_pool_addresses() {
    require_root();
    let h = Harness::up("hstr", &[]).unwrap();
    let client = h.ns("client");
    // The harness configures the client arm as a host. Port setup takes only
    // a bare interface, so the test removes that configuration first.
    ip(&["-n", &client, "route", "flush", "dev", "eth0"]);
    ip(&["-n", &client, "addr", "flush", "dev", "eth0"]);
    let before_client = snapshot(Some(&client));
    let before_self = snapshot(None);
    let before_ns = namespaces();

    let pool = HostPool::parse(&PoolSpec {
        name: "clients",
        ipv4: Some("10.10.0.0/16"),
        count: 20_000,
        gateway: Some("10.201.0.1"),
        ..PoolSpec::default()
    })
    .unwrap();
    let mut spec = PortSpec::new("hstr", "eth0", &pool, pool.range()).unwrap();
    spec.origin_netns = Some(client.clone());
    spec.address = Some("10.201.0.2/24".parse::<InterfaceAddr>().unwrap());
    let mut runner = IpCommand::new();

    // The server arm carries the default route of its namespace, so setup
    // must refuse it and leave the namespace alone.
    let server_ns = h.ns("server");
    let before_server = snapshot(Some(&server_ns));
    let mut mgmt = spec.clone();
    mgmt.origin_netns = Some(server_ns.clone());
    let err = setup(&mgmt, &mut runner).unwrap_err();
    assert!(matches!(err, PortError::Management { .. }), "{err}");
    assert_eq!(snapshot(Some(&server_ns)), before_server);
    assert!(!namespaces().contains(&spec.namespace()));

    let started = Instant::now();
    let guard = Cleanup(spec.clone());
    let port = setup(&spec, &mut runner).unwrap();
    println!(
        "setup took {:?}, skipped sysctls {:?}",
        started.elapsed(),
        port.skipped_sysctls()
    );
    let port_ns = port.namespace();
    assert_eq!(
        sysctl_in(&port_ns, "net.ipv4.ip_local_port_range"),
        "1024 65535"
    );
    assert_eq!(sysctl_in(&port_ns, "net.ipv4.tcp_tw_reuse"), "1");
    assert_eq!(
        sysctl_in(&port_ns, "net.ipv4.tcp_rmem"),
        "4096 131072 16777216"
    );
    ip(&[
        "-n",
        &h.ns("router"),
        "route",
        "add",
        "10.10.0.0/16",
        "via",
        "10.201.0.2",
    ]);

    sessions(
        &port_ns,
        &server_ns,
        Ipv4Addr::new(10, 202, 0, 2),
        &pool_env("10.10.0.0/16", 20_000, "10.201.0.1"),
    );

    let started = Instant::now();
    port.teardown(&mut runner).unwrap();
    println!("teardown took {:?}", started.elapsed());
    assert_eq!(namespaces(), before_ns, "teardown left a namespace");
    assert_eq!(
        settled_snapshot(Some(&client), &before_client),
        before_client,
        "teardown changed the client namespace"
    );
    assert_eq!(
        snapshot(None),
        before_self,
        "port setup changed this namespace"
    );
    // A second cleanup finds nothing to do.
    cleanup(&spec, &mut runner).unwrap();
    drop(guard);
    drop(h);
    assert!(
        !namespaces().iter().any(|n| n.starts_with("hstr-")),
        "the harness left a namespace"
    );
}

/// An on-link port: the router resolves every host address, and the gateway
/// lies inside the pool prefix.
#[test]
#[ignore = "needs root and the QA-01 harness, run make e2e-hst"]
fn on_link_port_answers_for_pool_addresses() {
    on_link("hsto", None);
}

/// The on-link port on an 802.1Q subinterface. A kernel without the 8021q
/// module skips the test unless `FPS_REQUIRE_VLAN` is 1, as in the QA-01
/// harness self test.
#[test]
#[ignore = "needs root and the QA-01 harness, run make e2e-hst"]
fn vlan_port_answers_for_on_link_pool_addresses() {
    on_link("hstv", Some(100));
}

fn on_link(prefix: &'static str, vlan: Option<u16>) {
    require_root();
    let vlan_text = vlan.map(|v| v.to_string()).unwrap_or_default();
    let env = [
        ("FPS_CLIENT_NET", "10.30.0.0/22"),
        ("FPS_CLIENT_VLAN", vlan_text.as_str()),
    ];
    let h = match Harness::up(prefix, &env) {
        Ok(h) => h,
        Err(e)
            if e.contains("8021q") && std::env::var("FPS_REQUIRE_VLAN").as_deref() != Ok("1") =>
        {
            println!("skipped because the kernel cannot create VLANs: {e}");
            return;
        }
        Err(e) => panic!("harness up failed: {e}"),
    };
    let client = h.ns("client");
    // The harness configures the client arm as a host. Port setup takes only
    // a bare interface, so the test removes that configuration first.
    if vlan.is_some() {
        ip(&[
            "-n",
            &client,
            "link",
            "del",
            "dev",
            &format!("eth0.{vlan_text}"),
        ]);
    } else {
        ip(&["-n", &client, "route", "flush", "dev", "eth0"]);
        ip(&["-n", &client, "addr", "flush", "dev", "eth0"]);
    }
    let before_client = snapshot(Some(&client));
    let before_ns = namespaces();

    let pool = HostPool::parse(&PoolSpec {
        name: "clients",
        ipv4: Some("10.30.0.0/22"),
        count: 500,
        gateway: Some("10.30.0.1"),
        ..PoolSpec::default()
    })
    .unwrap();
    let mut spec = PortSpec::new(prefix, "eth0", &pool, pool.range()).unwrap();
    spec.origin_netns = Some(client.clone());
    spec.vlan = vlan;
    let mut runner = IpCommand::new();
    let guard = Cleanup(spec.clone());
    let port = setup(&spec, &mut runner).unwrap();
    let port_ns = port.namespace();
    let gw = ip(&["-n", &port_ns, "route", "get", "10.30.0.1"]);
    assert!(
        !gw.starts_with("local"),
        "the gateway must not be local: {gw}"
    );
    assert!(gw.contains(&format!("dev {}", spec.data_device())), "{gw}");

    sessions(
        &port_ns,
        &h.ns("server"),
        Ipv4Addr::new(10, 202, 0, 2),
        &pool_env("10.30.0.0/22", 500, "10.30.0.1"),
    );

    port.teardown(&mut runner).unwrap();
    assert_eq!(namespaces(), before_ns, "teardown left a namespace");
    assert_eq!(
        settled_snapshot(Some(&client), &before_client),
        before_client,
        "teardown changed the client namespace"
    );
    drop(guard);
}

/// The client and server sides, which run as child processes inside the
/// namespaces. Without `FPS_HST_ROLE` the test does nothing.
#[test]
#[ignore = "helper process of the kernel port tests"]
fn helper_process() {
    match std::env::var("FPS_HST_ROLE").as_deref() {
        Ok("server") => serve(),
        Ok("client") => connect_all(),
        _ => {}
    }
}

fn server_addr() -> SocketAddr {
    std::env::var("FPS_HST_SERVER").unwrap().parse().unwrap()
}

/// Accepts one session per pool host. Each client sends its host index, and
/// the server checks that the peer address is the address of that index.
fn serve() {
    let pool = pool_from_env();
    let listener = TcpListener::bind(server_addr()).unwrap();
    listener.set_nonblocking(true).unwrap();
    println!("{MARK}listening");
    let deadline = Instant::now() + Duration::from_secs(120);
    let (mut accepted, mut mismatched) = (0u32, 0u32);
    let mut seen = HashSet::new();
    while accepted < pool.count() && Instant::now() < deadline {
        let (mut stream, peer) = match listener.accept() {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            Err(e) => panic!("accept: {e}"),
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut idx = [0u8; 4];
        accepted += 1;
        if stream.read_exact(&mut idx).is_err() {
            mismatched += 1;
            continue;
        }
        let index = u32::from_be_bytes(idx);
        let want = pool.ipv4_addr(index).ok().flatten().map(IpAddr::V4);
        if want != Some(peer.ip()) {
            mismatched += 1;
        }
        seen.insert(peer.ip());
        let _ = stream.write_all(&[1]);
    }
    println!(
        "{MARK}server accepted={accepted} distinct={} mismatched={mismatched}",
        seen.len()
    );
}

/// Binds one socket to every pool address in turn and connects it to the
/// server, from eight threads.
fn connect_all() {
    let pool = Arc::new(pool_from_env());
    let server = server_addr();
    let ok = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(AtomicUsize::new(0));
    let first_error = Arc::new(Mutex::new(None::<String>));
    let threads = 8u32;
    let workers: Vec<_> = (0..threads)
        .map(|t| {
            let (pool, ok, failed, first_error) = (
                pool.clone(),
                ok.clone(),
                failed.clone(),
                first_error.clone(),
            );
            std::thread::spawn(move || {
                for index in (t..pool.count()).step_by(threads as usize) {
                    match one_session(&pool, index, server) {
                        Ok(()) => ok.fetch_add(1, Ordering::Relaxed),
                        Err(e) => {
                            first_error.lock().unwrap().get_or_insert(e);
                            failed.fetch_add(1, Ordering::Relaxed)
                        }
                    };
                }
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    let err = first_error.lock().unwrap().clone().unwrap_or_default();
    println!(
        "{MARK}client ok={} failed={} first_error={err:?}",
        ok.load(Ordering::Relaxed),
        failed.load(Ordering::Relaxed)
    );
}

fn one_session(pool: &HostPool, index: u32, server: SocketAddr) -> Result<(), String> {
    let addr = pool.ipv4_addr(index).map_err(|e| e.to_string())?.unwrap();
    let ctx = |what: &str, e: std::io::Error| format!("host {index} {addr}: {what}: {e}");
    let sock = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))
        .map_err(|e| ctx("socket", e))?;
    sock.bind(&SocketAddr::new(IpAddr::V4(addr), 0).into())
        .map_err(|e| ctx("bind", e))?;
    sock.connect_timeout(&server.into(), Duration::from_secs(5))
        .map_err(|e| ctx("connect", e))?;
    let local = sock
        .local_addr()
        .ok()
        .and_then(|a| a.as_socket())
        .map(|a| a.ip());
    if local != Some(IpAddr::V4(addr)) {
        return Err(format!("host {index}: bound to {local:?}"));
    }
    let mut stream: std::net::TcpStream = sock.into();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| ctx("timeout", e))?;
    stream
        .write_all(&index.to_be_bytes())
        .map_err(|e| ctx("write", e))?;
    let mut ack = [0u8; 1];
    stream.read_exact(&mut ack).map_err(|e| ctx("read", e))?;
    Ok(())
}
