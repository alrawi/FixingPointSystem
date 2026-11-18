// SPDX-License-Identifier: GPL-3.0-only
//! The own stack reaches a kernel peer behind a namespace router with no
//! neighbor entry configured, over IPv4 and over IPv6 (TRN-35).
//!
//! Three namespaces make a chain:
//!
//! ```text
//!   own (AF_XDP on oa) --veth-- router (ra, rb, forwarding) --veth-- kernel peer (kb)
//!        10.0.0.2                  10.0.0.1 | 10.0.1.1                   10.0.1.2
//!        fd00:a::2                 fd00:a::1 | fd00:b::1                 fd00:b::2
//! ```
//!
//! The datapath runs the own stack over an AF_XDP queue on `oa` with the
//! next hop resolution on. The host pool of that arm names the router address
//! as its gateway, and nothing else: the kernel peer is in no pool, no
//! neighbor entry exists anywhere, and the router namespace starts with an
//! empty neighbor table. The own stack finds the router with an ARP request
//! (IPv4) or a neighbor solicitation (IPv6), and the router finds the own
//! host the other way, through the answers of the splitter. Four scenarios
//! run, each a byte checked echo of 256 KiB (`FPS_ROUTER_KIB` changes the
//! size): the own stack as the client and as the server, over both families.
//! The kernel is the other side, a plain socket in the peer namespace.
//!
//! The tests need root, `ip`, `sysctl`, and AF_XDP. As a normal user the
//! binary runs itself again through `sudo -n` and skips with a message when
//! that is not possible. `CI=true` or `FPS_REQUIRE_AFXDP=1` turns every skip
//! into a failure. The binary is its own helper: the peers and the own stack
//! are this binary again, started with `ip netns exec` and the test
//! `helper_router`, which reads its role from `FPS_ROUTER_ROLE`.
//! `FPS_ROUTER_RESULTS` names a directory that receives `router-<scenario>.txt`.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use fps_backend_afxdp::memory::frame_pair;
use fps_backend_afxdp::{open_port, BindMode, FrameIo, OpenedPort, PortConfig, QueueConfig};
use fps_datapath::{Datapath, DatapathConfig, Lane, NeighborConfig};
use fps_hosts::{HostPool, PoolSpec};
use fps_tcp_own::{HostSlice, Pools};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MARK: &str = "fps-router ";
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
const PORT: u16 = 9300;

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn bytes() -> usize {
    usize::try_from(env_u64("FPS_ROUTER_KIB", 256) * 1024).unwrap_or(256 * 1024)
}

/// What the own stack does and over which family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    OwnClient4,
    OwnServer4,
    OwnClient6,
    OwnServer6,
}

impl Role {
    fn name(self) -> &'static str {
        match self {
            Self::OwnClient4 => "own-client-v4",
            Self::OwnServer4 => "own-server-v4",
            Self::OwnClient6 => "own-client-v6",
            Self::OwnServer6 => "own-server-v6",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        [
            Self::OwnClient4,
            Self::OwnServer4,
            Self::OwnClient6,
            Self::OwnServer6,
        ]
        .into_iter()
        .find(|r| r.name() == name)
    }

    fn v6(self) -> bool {
        matches!(self, Self::OwnClient6 | Self::OwnServer6)
    }

    fn own_is_client(self) -> bool {
        matches!(self, Self::OwnClient4 | Self::OwnClient6)
    }

    /// The address of the kernel peer.
    fn peer(self) -> IpAddr {
        let text = if self.v6() { "fd00:b::2" } else { "10.0.1.2" };
        text.parse().unwrap()
    }
}

fn pool(name: &str, v6: bool, prefix: &str, gateway: &str) -> HostPool {
    HostPool::parse(&PoolSpec {
        name,
        ipv4: (!v6).then_some(prefix),
        ipv6: v6.then_some(prefix),
        count: 1,
        gateway: Some(gateway),
        ..PoolSpec::default()
    })
    .unwrap()
}

/// The pools of a role. The arm on `oa` has the pool of the own namespace,
/// whose gateway is the router. The other arm is idle and has a pool of its
/// own, so the lane is complete.
fn pools_for(role: Role) -> Pools {
    let on_link = if role.v6() {
        pool("own", true, "fd00:a::/64", "fd00:a::1")
    } else {
        pool("own", false, "10.0.0.0/24", "10.0.0.1")
    };
    let idle = if role.v6() {
        pool("idle", true, "fd00:c::/64", "fd00:c::1")
    } else {
        pool("idle", false, "10.0.2.0/24", "10.0.2.1")
    };
    let (client, server) = if role.own_is_client() {
        (on_link, idle)
    } else {
        (idle, on_link)
    };
    Pools {
        client: HostSlice::whole(client),
        server: HostSlice::whole(server),
        vlan: None,
    }
}

// ---------------------------------------------------------------------------
// The orchestrator: namespaces, processes, and the checks
// ---------------------------------------------------------------------------

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
        "the router test cannot run here and CI requires it: {why}"
    );
    eprintln!("skipping the router test: {why}");
}

/// Whether the kernel can create an AF_XDP socket at all.
fn kernel_has_af_xdp() -> bool {
    const AF_XDP: libc::c_int = 44;
    // SAFETY: `socket` takes no pointers, and the descriptor that it returns
    // is closed at once.
    let fd = unsafe { libc::socket(AF_XDP, libc::SOCK_RAW, 0) };
    if fd >= 0 {
        // SAFETY: `fd` is the descriptor from above, which nothing else owns.
        unsafe { libc::close(fd) };
    }
    fd >= 0 || io::Error::last_os_error().raw_os_error() != Some(libc::EAFNOSUPPORT)
}

/// The three namespaces, which `Drop` removes again.
struct Topology {
    own: String,
    router: String,
    peer: String,
}

impl Topology {
    fn up(tag: &str, v6: bool) -> Result<Self, String> {
        let t = Self {
            own: format!("t35o{tag}"),
            router: format!("t35r{tag}"),
            peer: format!("t35k{tag}"),
        };
        // Built after the value exists, so that `Drop` cleans up a failure
        // half way.
        for ns in [&t.own, &t.router, &t.peer] {
            run("ip", &["netns", "add", ns])?;
        }
        let (o, r, k) = (t.own.as_str(), t.router.as_str(), t.peer.as_str());
        run(
            "ip",
            &[
                "-n", o, "link", "add", "oa", "type", "veth", "peer", "name", "ra", "netns", r,
            ],
        )?;
        run(
            "ip",
            &[
                "-n", r, "link", "add", "rb", "type", "veth", "peer", "name", "kb", "netns", k,
            ],
        )?;
        for (ns, dev) in [(o, "oa"), (r, "ra"), (r, "rb"), (k, "kb")] {
            run("ip", &["-n", ns, "link", "set", dev, "up"])?;
        }
        // No neighbor entry is configured anywhere: the router and the peer
        // know only their own addresses and their routes. Only the family of
        // the scenario is set up, so a kernel without IPv6 still runs IPv4.
        let (ra, rb, kb, via, forwarding) = if v6 {
            (
                "fd00:a::1/64",
                "fd00:b::1/64",
                "fd00:b::2/64",
                "fd00:b::1",
                "net.ipv6.conf.all.forwarding=1",
            )
        } else {
            (
                "10.0.0.1/24",
                "10.0.1.1/24",
                "10.0.1.2/24",
                "10.0.1.1",
                "net.ipv4.ip_forward=1",
            )
        };
        for (ns, dev, address) in [(r, "ra", ra), (r, "rb", rb), (k, "kb", kb)] {
            let mut add = vec!["-n", ns, "addr", "add", address, "dev", dev];
            if v6 {
                // No duplicate address detection, so a socket can bind at once.
                add.push("nodad");
            }
            run("ip", &add)?;
        }
        let mut route = vec!["-n", k];
        if v6 {
            route.push("-6");
        }
        route.extend(["route", "add", "default", "via", via]);
        run("ip", &route)?;
        run(
            "ip",
            &["netns", "exec", r, "sysctl", "-q", "-w", forwarding],
        )?;
        Ok(t)
    }

    /// Starts this binary as `role` in the namespace `ns`, with its output
    /// piped.
    fn spawn(&self, ns: &str, role: &str, scenario: Role) -> Child {
        let exe = std::env::current_exe().unwrap();
        Command::new("ip")
            .args(["netns", "exec", ns])
            .arg(exe)
            .args(["--ignored", "--exact", "helper_router", "--nocapture"])
            .env("FPS_ROUTER_ROLE", role)
            .env("FPS_ROUTER_SCENARIO", scenario.name())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap()
    }
}

impl Drop for Topology {
    fn drop(&mut self) {
        for ns in [&self.own, &self.router, &self.peer] {
            let _ = run("ip", &["netns", "del", ns]);
        }
    }
}

/// The lines that a helper reported, without the marker.
fn lines(child: &mut Child) -> Vec<String> {
    let stdout = child.stdout.take().unwrap();
    BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| l.strip_prefix(MARK).map(str::to_owned))
        .collect()
}

/// Reads lines until one starts with `first`, and returns what was read.
fn wait_for_line(reader: &mut BufReader<std::process::ChildStdout>, first: &str) -> Vec<String> {
    let mut seen = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).unwrap();
        assert!(n > 0, "the helper ended before {first:?}: {seen:?}");
        if let Some(text) = line.trim_end().strip_prefix(MARK) {
            seen.push(text.to_owned());
            if text.starts_with(first) {
                return seen;
            }
        }
    }
}

fn field<'a>(line: &'a str, key: &str) -> &'a str {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
        .unwrap_or_else(|| panic!("no field {key} in {line:?}"))
}

fn number(line: &str, key: &str) -> u64 {
    field(line, key)
        .parse()
        .unwrap_or_else(|_| panic!("field {key} of {line:?} is not a number"))
}

fn find<'a>(lines: &'a [String], start: &str) -> &'a str {
    lines
        .iter()
        .find(|l| l.starts_with(start))
        .unwrap_or_else(|| panic!("no {start} line in {lines:?}"))
}

fn scenario(test: &str, role: Role) {
    let root = run("id", &["-u"]).is_ok_and(|s| s.trim() == "0");
    if !root {
        if run("sudo", &["-n", "true"]).is_err() {
            skip("not root and no passwordless sudo");
            return;
        }
        let exe = std::env::current_exe().unwrap();
        let mut cmd = Command::new("sudo");
        cmd.args(["-n", "env"]);
        for key in [
            "CI",
            "FPS_REQUIRE_AFXDP",
            "FPS_ROUTER_KIB",
            "FPS_ROUTER_RESULTS",
        ] {
            if let Ok(v) = std::env::var(key) {
                cmd.arg(format!("{key}={v}"));
            }
        }
        let status = cmd
            .arg(exe)
            .args(["--exact", test, "--nocapture"])
            .status()
            .unwrap();
        assert!(status.success(), "the test failed under sudo");
        return;
    }
    if let Err(e) = run("ip", &["-V"]) {
        skip(&e);
        return;
    }
    if !kernel_has_af_xdp() {
        skip("the kernel has no AF_XDP sockets");
        return;
    }
    // One scenario at a time: they share the CPUs, and the timing of a
    // handshake is not what they test.
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tag = format!(
        "{}{}",
        role.name().replace('-', ""),
        std::process::id() % 100_000
    );
    let topology = match Topology::up(&tag, role.v6()) {
        Ok(t) => t,
        Err(e) => {
            skip(&format!("the namespaces did not come up: {e}"));
            return;
        }
    };
    // The side that listens starts first, so the other one can connect.
    let (own, peer, own_lines, peer_lines);
    if role.own_is_client() {
        let mut server = topology.spawn(&topology.peer, "peer-server", role);
        let mut reader = BufReader::new(server.stdout.take().unwrap());
        let mut seen = wait_for_line(&mut reader, "ready");
        let mut client = topology.spawn(&topology.own, "own-client", role);
        own_lines = lines(&mut client);
        seen.extend(
            reader
                .lines()
                .map_while(Result::ok)
                .filter_map(|l| l.strip_prefix(MARK).map(str::to_owned)),
        );
        peer_lines = seen;
        own = client.wait().unwrap();
        peer = server.wait().unwrap();
    } else {
        let mut server = topology.spawn(&topology.own, "own-server", role);
        let mut reader = BufReader::new(server.stdout.take().unwrap());
        let mut seen = wait_for_line(&mut reader, "ready");
        let mut client = topology.spawn(&topology.peer, "peer-client", role);
        peer_lines = lines(&mut client);
        seen.extend(
            reader
                .lines()
                .map_while(Result::ok)
                .filter_map(|l| l.strip_prefix(MARK).map(str::to_owned)),
        );
        own_lines = seen;
        peer = client.wait().unwrap();
        own = server.wait().unwrap();
    }
    for l in own_lines.iter().chain(&peer_lines) {
        eprintln!("{MARK}{l}");
    }
    assert!(own.success(), "the own stack failed: {own_lines:?}");
    assert!(peer.success(), "the kernel peer failed: {peer_lines:?}");

    let n = bytes() as u64;
    let done = find(&peer_lines, "peer done");
    assert_eq!(number(done, "bytes"), n, "{done}");
    let result = find(&own_lines, "own done");
    assert_eq!(number(result, "bytes"), n, "{result}");
    // The own stack found its next hop itself: it asked, the router answered,
    // the frames that waited went out, and nothing failed.
    assert!(number(result, "requests") >= 1, "{result}");
    assert_eq!(number(result, "resolved"), 1, "{result}");
    assert_eq!(number(result, "failed"), 0, "{result}");
    assert_eq!(number(result, "entries"), 1, "the gateway only: {result}");
    assert!(number(result, "answers") >= 1, "{result}");
    assert!(number(result, "released") >= 1, "{result}");
    if !role.own_is_client() {
        // The router found the own host with ARP or neighbor discovery, which
        // the splitter answered. (When the own stack is the client, the router
        // already learned its address from the request, as the kernel does.)
        assert!(number(result, "replied") >= 1, "{result}");
    }
    assert_eq!(number(result, "panicked"), 0, "{result}");
    if let Ok(dir) = std::env::var("FPS_ROUTER_RESULTS") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut text = own_lines.join("\n");
        text.push('\n');
        std::fs::write(dir.join(format!("router-{}.txt", role.name())), text).unwrap();
    }
}

#[test]
fn the_own_client_reaches_a_kernel_server_behind_a_router_over_ipv4() {
    scenario(
        "the_own_client_reaches_a_kernel_server_behind_a_router_over_ipv4",
        Role::OwnClient4,
    );
}

#[test]
fn the_own_server_is_reached_by_a_kernel_client_behind_a_router_over_ipv4() {
    scenario(
        "the_own_server_is_reached_by_a_kernel_client_behind_a_router_over_ipv4",
        Role::OwnServer4,
    );
}

#[test]
fn the_own_client_reaches_a_kernel_server_behind_a_router_over_ipv6() {
    scenario(
        "the_own_client_reaches_a_kernel_server_behind_a_router_over_ipv6",
        Role::OwnClient6,
    );
}

#[test]
fn the_own_server_is_reached_by_a_kernel_client_behind_a_router_over_ipv6() {
    scenario(
        "the_own_server_is_reached_by_a_kernel_client_behind_a_router_over_ipv6",
        Role::OwnServer6,
    );
}

// ---------------------------------------------------------------------------
// The helper: this binary again, inside a namespace
// ---------------------------------------------------------------------------

fn say(msg: &str) {
    println!("{MARK}{msg}");
    let _ = io::stdout().flush();
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| u8::try_from((i * 7 + i / 251) % 256).unwrap_or(0))
        .collect()
}

/// Turns the transmit checksum offload of `dev` off with the ethtool ioctl
/// (`ETHTOOL_STXCSUM`), because the veth hands a frame with a partial checksum
/// on to an AF_XDP end and the hosted runner has no `ethtool` command.
fn tx_checksum_off(dev: &str) -> io::Result<()> {
    #[repr(C)]
    struct EthtoolValue {
        cmd: u32,
        data: u32,
    }
    const SIOCETHTOOL: libc::Ioctl = 0x8946;
    const ETHTOOL_STXCSUM: u32 = 0x17;
    // SAFETY: `socket` takes no pointers.
    let sock = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if sock < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut value = EthtoolValue {
        cmd: ETHTOOL_STXCSUM,
        data: 0,
    };
    // SAFETY: `ifreq` is plain data for which all zero bytes are valid.
    let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
    for (slot, byte) in request.ifr_name.iter_mut().zip(dev.bytes().take(15)) {
        *slot = byte as libc::c_char;
    }
    request.ifr_ifru.ifru_data = std::ptr::addr_of_mut!(value).cast();
    // SAFETY: `request` points at `value`, and both outlive the call, which
    // reads `cmd` and `data` of the structure that the request names.
    let result = unsafe { libc::ioctl(sock, SIOCETHTOOL, &mut request) };
    let error = io::Error::last_os_error();
    // SAFETY: `sock` is the descriptor from above, which nothing else owns.
    unsafe { libc::close(sock) };
    if result < 0 {
        Err(error)
    } else {
        Ok(())
    }
}

fn open(interface: &str) -> OpenedPort {
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
            Ok(p) => return p,
            Err(e) if e.raw_os_error() == Some(16) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("opening {interface}: {e:?}"),
        }
    }
}

/// A blocking socket peer for the kernel side. The peer server echoes what it
/// reads, and the peer client sends the pattern and checks the echo.
fn peer(role: &str, scenario: Role) {
    tx_checksum_off("kb").expect("the transmit offload of kb turns off");
    let n = bytes();
    let timeout = Some(Duration::from_secs(30));
    match role {
        "peer-server" => {
            let listener = TcpListener::bind(SocketAddr::new(scenario.peer(), PORT)).unwrap();
            say("ready");
            let (mut stream, from) = listener.accept().unwrap();
            stream.set_read_timeout(timeout).unwrap();
            stream.set_write_timeout(timeout).unwrap();
            let mut data = vec![0u8; n];
            stream.read_exact(&mut data).unwrap();
            assert_eq!(data, pattern(n), "the bytes arrived intact");
            stream.write_all(&data).unwrap();
            stream.shutdown(std::net::Shutdown::Write).unwrap();
            say(&format!("peer done bytes={n} from={from}"));
        }
        _ => {
            // The own server is the host of the pool on the link of `oa`.
            let target = pools_for(scenario).server.ip(0).unwrap();
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut stream = loop {
                match TcpStream::connect_timeout(
                    &SocketAddr::new(target, PORT),
                    Duration::from_secs(5),
                ) {
                    Ok(s) => break s,
                    Err(e) if Instant::now() < deadline => {
                        eprintln!("{MARK}connect to {target} failed: {e}, trying again");
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    Err(e) => panic!("connect to {target}: {e}"),
                }
            };
            stream.set_read_timeout(timeout).unwrap();
            stream.set_write_timeout(timeout).unwrap();
            stream.write_all(&pattern(n)).unwrap();
            let mut back = vec![0u8; n];
            stream.read_exact(&mut back).unwrap();
            assert_eq!(back, pattern(n), "the echo is intact");
            say(&format!("peer done bytes={n}"));
        }
    }
}

fn own(role: &str, scenario: Role) {
    tx_checksum_off("oa").expect("the transmit offload of oa turns off");
    let mut port = open("oa");
    let (spare, _spare_end) = frame_pair(8, 1600);
    let queue: Box<dyn FrameIo> = Box::new(port.queues.remove(0));
    let (client, server): (Box<dyn FrameIo>, Box<dyn FrameIo>) = if scenario.own_is_client() {
        (queue, Box::new(spare))
    } else {
        (Box::new(spare), queue)
    };
    let pools = pools_for(scenario);
    let lane = Lane {
        client,
        server,
        pools: pools.clone(),
    };
    let config = DatapathConfig {
        neighbors: Some(NeighborConfig::default()),
        ..DatapathConfig::default()
    };
    let dp = Datapath::start(vec![lane], config).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let n = bytes();
    let outcome = {
        let _guard = runtime.enter();
        runtime.block_on(async {
            let echoed = if role == "own-client" {
                let source = pools.client.ip(0).unwrap();
                let destination = SocketAddr::new(scenario.peer(), PORT);
                let mut session = dp
                    .connector()
                    .connect(source, destination)
                    .await
                    .map_err(|e| format!("connect: {e}"))?;
                session
                    .write_all(&pattern(n))
                    .await
                    .map_err(|e| format!("write: {e}"))?;
                let mut back = vec![0u8; n];
                session
                    .read_exact(&mut back)
                    .await
                    .map_err(|e| format!("read: {e}"))?;
                back == pattern(n)
            } else {
                let mut listener = dp.listen(PORT).map_err(|e| format!("listen: {e}"))?;
                say("ready");
                let (mut session, from) =
                    tokio::time::timeout(Duration::from_secs(30), listener.accept())
                        .await
                        .map_err(|_| "no connection within 30 s".to_owned())?
                        .map_err(|e| format!("accept: {e}"))?;
                eprintln!("{MARK}accepted {from}");
                let mut data = vec![0u8; n];
                session
                    .read_exact(&mut data)
                    .await
                    .map_err(|e| format!("read: {e}"))?;
                let intact = data == pattern(n);
                session
                    .write_all(&data)
                    .await
                    .map_err(|e| format!("write: {e}"))?;
                session
                    .shutdown()
                    .await
                    .map_err(|e| format!("shutdown: {e}"))?;
                // The echo is only in the send buffer. Wait for the peer to
                // read it all and close, or stopping the datapath would cut
                // the transfer short.
                let mut rest = Vec::new();
                tokio::time::timeout(Duration::from_secs(30), session.read_to_end(&mut rest))
                    .await
                    .map_err(|_| "the peer did not close within 30 s".to_owned())?
                    .map_err(|e| format!("read to the end: {e}"))?;
                intact
            };
            Ok::<bool, String>(echoed)
        })
    };
    let lane = dp.stats().remove(0);
    let reports = dp.stop();
    // The arm on `oa` is the one that resolved.
    let (arm, other) = if scenario.own_is_client() {
        (lane.client, lane.server)
    } else {
        (lane.server, lane.client)
    };
    let replied = if scenario.v6() {
        arm.nd_replied
    } else {
        arm.arp_replied
    };
    let ok = matches!(outcome, Ok(true));
    say(&format!(
        "own done ok={} bytes={} requests={} resolved={} failed={} held={} released={} \
         dropped={} answers={} entries={} replied={} idle_requests={} panicked={}",
        u8::from(ok),
        if ok { n } else { 0 },
        arm.neighbor_requests,
        arm.neighbor_resolved,
        arm.neighbor_failed,
        arm.neighbor_held,
        arm.neighbor_released,
        arm.neighbor_dropped,
        arm.neighbor_answers,
        arm.neighbor_entries,
        replied,
        other.neighbor_requests,
        u8::from(reports[0].panicked),
    ));
    if let Err(ref e) = outcome {
        eprintln!("{MARK}error {e}");
    }
    assert!(ok, "the echo through the router failed: {outcome:?}");
}

#[test]
#[ignore = "started by the router tests inside a namespace"]
fn helper_router() {
    let (Ok(role), Ok(name)) = (
        std::env::var("FPS_ROUTER_ROLE"),
        std::env::var("FPS_ROUTER_SCENARIO"),
    ) else {
        return;
    };
    let scenario = Role::parse(&name).expect("a known scenario");
    if role.starts_with("peer") {
        peer(&role, scenario);
    } else {
        own(&role, scenario);
    }
}
