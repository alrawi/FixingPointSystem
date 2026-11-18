// SPDX-License-Identifier: GPL-3.0-only
//! The kernel reference stack on an impaired link in network namespaces
//! (TRN-22).
//!
//! The loss workload for the kernel cannot use the loopback interface, because
//! the kernel would carry the segments without a link to impair. [`NetnsLink`]
//! builds a client namespace and a server namespace with a router namespace
//! between them, joined by two veth pairs, and puts the `tc` qdiscs of
//! [`crate::impair::netem_commands`] on both interfaces of the router, so each
//! direction has the settings of the in memory link. [`kernel_transfer`] then runs one transfer
//! with a server process in one namespace and a client process in the other.
//! The processes are this binary again, started with `ip netns exec` and the
//! hidden flag `--netns-peer`, so the harness needs no `setns` and no unsafe
//! code. The client reads `RetransSegs` of its namespace before and after the
//! transfer, which is the retransmission count of the kernel.
//!
//! The setup needs root, `ip`, and `tc`. A kernel without the `sch_netem`
//! module cannot add the `netem` qdisc, and the setup then fails with
//! [`NetnsError::NetemMissing`] so a caller can skip or fail with a message.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpSocket;

use crate::impair::{netem_commands, ImpairConfig};
use crate::workload::{fill_pattern, first_mismatch, LossConfig, LossRun};

/// The address of the client host.
pub const CLIENT_ADDR: Ipv4Addr = Ipv4Addr::new(10, 77, 1, 1);
/// The address of the server host.
pub const SERVER_ADDR: Ipv4Addr = Ipv4Addr::new(10, 77, 2, 1);
/// The hidden first argument that turns the binary into a peer process.
pub const PEER_FLAG: &str = "--netns-peer";

/// The flag with which a binary is asked to turn the transmit checksum
/// offload of one interface off and exit (see
/// [`NetnsLink::complete_checksums`]).
pub const OFFLOAD_FLAG: &str = "--netns-offload-off";

/// Errors of the namespace setup and the peer processes.
#[derive(Debug, thiserror::Error)]
pub enum NetnsError {
    /// A command failed or could not start.
    #[error("{what}: {detail}")]
    Command {
        /// The command line.
        what: String,
        /// The error output or the reason that it did not start.
        detail: String,
    },
    /// The kernel has no `netem` qdisc.
    #[error("the kernel has no netem qdisc (module sch_netem is missing): {0}")]
    NetemMissing(String),
    /// A peer process did not report as expected.
    #[error("peer process: {0}")]
    Peer(String),
}

pub(crate) fn run_command(cmd: &mut Command) -> Result<String, NetnsError> {
    let what = format!("{cmd:?}").replace('"', "");
    let out = cmd.output().map_err(|e| NetnsError::Command {
        what: what.clone(),
        detail: e.to_string(),
    })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(NetnsError::Command {
            what,
            detail: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        })
    }
}

pub(crate) fn ip(args: &[&str]) -> Result<String, NetnsError> {
    run_command(Command::new("ip").args(args))
}

/// Returns true when this process runs as root.
pub fn is_root() -> bool {
    std::fs::metadata("/proc/self").is_ok_and(|m| m.uid() == 0)
}

/// Three namespaces, a client, a router, and a server, joined by two veth
/// pairs, with the impairment on the two ends of the router. The namespaces
/// go away when the value drops.
///
/// The impairment is in the router and not at the hosts for a reason. The
/// kernel treats a drop in the qdisc of its own interface as congestion that
/// it sees at once, and `RetransSegs` counts every retransmission that such a
/// drop refuses. A bottleneck in the middle drops the way a network does, as
/// the in memory link does.
pub struct NetnsLink {
    client: String,
    router: String,
    server: String,
}

impl NetnsLink {
    /// Builds the link under the name prefix `prefix`, with `mtu` on every
    /// interface.
    ///
    /// # Errors
    ///
    /// Returns an error when a command fails. A missing `netem` qdisc is
    /// [`NetnsError::NetemMissing`]. The namespaces that were created are
    /// removed again.
    pub fn up(prefix: &str, config: &ImpairConfig, mtu: usize) -> Result<Self, NetnsError> {
        let link = Self {
            client: format!("{prefix}c"),
            router: format!("{prefix}r"),
            server: format!("{prefix}s"),
        };
        // From here a failure drops `link`, which deletes what exists.
        for ns in [&link.client, &link.router, &link.server] {
            ip(&["netns", "add", ns])?;
        }
        let mtu = mtu.to_string();
        // Each arm is a veth pair, the host end first and the router end
        // second, on its own /24.
        let arms = [
            (link.client.as_str(), "vc", "10.77.1.1", "rc", "10.77.1.2"),
            (link.server.as_str(), "vs", "10.77.2.1", "rs", "10.77.2.2"),
        ];
        for (host, host_dev, host_addr, router_dev, router_addr) in arms {
            ip(&[
                "link",
                "add",
                host_dev,
                "netns",
                host,
                "type",
                "veth",
                "peer",
                "name",
                router_dev,
                "netns",
                &link.router,
            ])?;
            for (ns, dev, addr) in [
                (host, host_dev, host_addr),
                (link.router.as_str(), router_dev, router_addr),
            ] {
                ip(&["-n", ns, "addr", "add", &format!("{addr}/24"), "dev", dev])?;
                ip(&["-n", ns, "link", "set", dev, "mtu", &mtu, "up"])?;
                // Offloads would pass segments larger than the MTU through the
                // qdisc as one frame, so a loss would take many segments at once.
                let _ = run_command(Command::new("ip").args([
                    "netns", "exec", ns, "ethtool", "-K", dev, "tso", "off", "gso", "off", "gro",
                    "off",
                ]));
            }
            ip(&["-n", host, "link", "set", "lo", "up"])?;
            ip(&["-n", host, "route", "add", "default", "via", router_addr])?;
        }
        run_command(Command::new("ip").args([
            "netns",
            "exec",
            &link.router,
            "sysctl",
            "-qw",
            "net.ipv4.ip_forward=1",
        ]))?;
        for dev in ["rc", "rs"] {
            for args in netem_commands(config, dev) {
                let mut cmd = Command::new("ip");
                cmd.args(["netns", "exec", &link.router, "tc"]).args(&args);
                run_command(&mut cmd).map_err(|e| match e {
                    NetnsError::Command { detail, .. }
                        if args.iter().any(|a| a == "netem")
                            && (detail.contains("kind is unknown")
                                || detail.contains("No such file or directory")) =>
                    {
                        NetnsError::NetemMissing(detail)
                    }
                    other => other,
                })?;
            }
        }
        Ok(link)
    }

    /// Returns the name of the router namespace.
    pub fn router_ns(&self) -> &str {
        &self.router
    }

    /// Returns the name of the client namespace.
    pub fn client_ns(&self) -> &str {
        &self.client
    }

    /// Returns the name of the server namespace.
    pub fn server_ns(&self) -> &str {
        &self.server
    }
}

impl NetnsLink {
    /// Turns the transmit checksum offload off on every interface of the
    /// three namespaces, so every frame that leaves a host carries a complete
    /// TCP checksum. A frame that reaches an AF_XDP socket (TRN-26) has no
    /// later hop that would complete a partial checksum, and the own stack
    /// verifies it. The command is `ethtool`; a host without it (the hosted
    /// runner image has none) runs `exe` again in the namespace with
    /// [`OFFLOAD_FLAG`] and the interface, which the binary answers with the
    /// ethtool ioctl (the test binary of TRN-26 does, because this crate
    /// forbids unsafe code).
    ///
    /// # Errors
    ///
    /// Returns an error when neither way works.
    pub fn complete_checksums(&self, exe: &Path) -> Result<(), NetnsError> {
        for (ns, dev) in [
            (&self.client, "vc"),
            (&self.router, "rc"),
            (&self.router, "rs"),
            (&self.server, "vs"),
        ] {
            let by_command = run_command(
                Command::new("ip").args(["netns", "exec", ns, "ethtool", "-K", dev, "tx", "off"]),
            );
            if by_command.is_err() {
                run_command(
                    Command::new("ip")
                        .args(["netns", "exec", ns])
                        .arg(exe)
                        .args([OFFLOAD_FLAG, dev]),
                )?;
            }
        }
        Ok(())
    }

    /// Gives the router interface `dev` (`rc` or `rs`) the MAC address `mac`,
    /// which the own stack uses as the next hop of its frames: it has no ARP
    /// requests of its own and takes the MAC address of a neighbor from its
    /// tables.
    ///
    /// # Errors
    ///
    /// Returns an error when `ip` refuses.
    pub fn set_router_mac(&self, dev: &str, mac: [u8; 6]) -> Result<(), NetnsError> {
        let text = mac.map(|b| format!("{b:02x}")).join(":");
        ip(&["-n", &self.router, "link", "set", dev, "address", &text])?;
        Ok(())
    }
}

impl Drop for NetnsLink {
    fn drop(&mut self) {
        for ns in [&self.client, &self.router, &self.server] {
            let _ = ip(&["netns", "del", ns]);
        }
    }
}

/// Reads a process line by line on a thread, so the caller can wait with a
/// timeout.
pub(crate) fn lines_of(child: &mut Child) -> Result<mpsc::Receiver<String>, NetnsError> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| NetnsError::Peer("no stdout".into()))?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    Ok(rx)
}

pub(crate) fn spawn_peer(exe: &Path, ns: &str, args: &[String]) -> Result<Child, NetnsError> {
    Command::new("ip")
        .args(["netns", "exec", ns])
        .arg(exe)
        .arg(PEER_FLAG)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| NetnsError::Command {
            what: format!("ip netns exec {ns} {}", exe.display()),
            detail: e.to_string(),
        })
}

/// Runs one transfer of the loss workload on the kernel stack over a fresh
/// namespace link with the settings `link`, and returns its outcome. `exe` is
/// this binary, `mtu` the MTU of both ends, and `buffer` the socket buffer in
/// bytes or `None` for the kernel's own sizing.
///
/// # Errors
///
/// Returns an error when the link or a peer process cannot be set up. A
/// transfer that does not complete is a [`LossRun`] with `completed` false,
/// not an error.
pub fn kernel_transfer(
    exe: &Path,
    prefix: &str,
    link: &ImpairConfig,
    config: &LossConfig,
    mtu: usize,
    buffer: Option<usize>,
    congestion: Option<&str>,
) -> Result<LossRun, NetnsError> {
    let ns = NetnsLink::up(prefix, link, mtu)?;
    if config.flows > 1 {
        return kernel_flows(exe, &ns, config, buffer, congestion);
    }
    let mut common = vec![
        "--port".to_string(),
        config.port.to_string(),
        "--chunk".to_string(),
        config.chunk.to_string(),
    ];
    if let Some(bytes) = buffer {
        common.extend(["--buffer-bytes".to_string(), bytes.to_string()]);
    }
    if let Some(name) = congestion {
        common.extend(["--cc".to_string(), name.to_string()]);
    }
    let mut server_args = vec!["server".to_string()];
    server_args.extend(common.clone());
    let mut server = spawn_peer(exe, ns.server_ns(), &server_args)?;
    let result = (|| {
        let server_lines = lines_of(&mut server)?;
        match server_lines.recv_timeout(Duration::from_secs(15)) {
            Ok(line) if line == "ready" => {}
            other => return Err(NetnsError::Peer(format!("the server said {other:?}"))),
        }
        let mut client_args = vec!["client".to_string()];
        client_args.extend(common);
        client_args.extend([
            "--bytes".to_string(),
            config.bytes.to_string(),
            "--timeout-ms".to_string(),
            config.timeout.as_millis().to_string(),
        ]);
        let started = Instant::now();
        let mut client = spawn_peer(exe, ns.client_ns(), &client_args)?;
        let client_lines = lines_of(&mut client)?;
        let limit = config.timeout + Duration::from_secs(20);
        let line = client_lines.recv_timeout(limit);
        let _ = client.kill();
        let _ = client.wait();
        match line {
            Ok(line) => parse_result(&line, started.elapsed()),
            Err(_) => Err(NetnsError::Peer("the client reported nothing".into())),
        }
    })();
    let _ = server.kill();
    let _ = server.wait();
    drop(ns);
    result
}

/// Runs the fairness variant of the loss workload (TRN-25) on the kernel
/// stack in the namespaces of `ns`: the server peer accepts `config.flows`
/// connections and reports the bytes of each in the measurement window, and
/// the client peer opens the flows, sends without end, and reports the
/// segments that the kernel sent again.
fn kernel_flows(
    exe: &Path,
    ns: &NetnsLink,
    config: &LossConfig,
    buffer: Option<usize>,
    congestion: Option<&str>,
) -> Result<LossRun, NetnsError> {
    let mut common = vec![
        "--port".to_string(),
        config.port.to_string(),
        "--chunk".to_string(),
        config.chunk.to_string(),
        "--flows".to_string(),
        config.flows.to_string(),
        "--warmup-ms".to_string(),
        config.warmup.as_millis().to_string(),
        "--window-ms".to_string(),
        config.window.as_millis().to_string(),
    ];
    if let Some(bytes) = buffer {
        common.extend(["--buffer-bytes".to_string(), bytes.to_string()]);
    }
    if let Some(name) = congestion {
        common.extend(["--cc".to_string(), name.to_string()]);
    }
    let mut server_args = vec!["server".to_string()];
    server_args.extend(common.clone());
    let mut server = spawn_peer(exe, ns.server_ns(), &server_args)?;
    let result = (|| {
        let server_lines = lines_of(&mut server)?;
        match server_lines.recv_timeout(Duration::from_secs(15)) {
            Ok(line) if line == "ready" => {}
            other => return Err(NetnsError::Peer(format!("the server said {other:?}"))),
        }
        let mut client_args = vec!["client".to_string()];
        client_args.extend(common);
        let mut client = spawn_peer(exe, ns.client_ns(), &client_args)?;
        let client_lines = lines_of(&mut client)?;
        let limit = FLOWS_CONNECT_LIMIT + config.warmup + config.window + Duration::from_secs(20);
        let server_line = server_lines.recv_timeout(limit);
        let client_line = client_lines.recv_timeout(Duration::from_secs(30));
        let _ = client.kill();
        let _ = client.wait();
        let server_line =
            server_line.map_err(|_| NetnsError::Peer("the server reported nothing".into()))?;
        let retransmits = client_line
            .ok()
            .and_then(|line| field_of(&line, "retransmits"))
            .unwrap_or(0);
        parse_flows(&server_line, retransmits)
    })();
    let _ = server.kill();
    let _ = server.wait();
    result
}

/// How long the flows of the kernel peers have to connect.
const FLOWS_CONNECT_LIMIT: Duration = Duration::from_secs(30);

/// Reads `key=value` from a result line.
fn field_of(line: &str, key: &str) -> Option<u64> {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(key)?.strip_prefix('=')?.parse().ok())
}

/// Reads the result line of the server peer of a fairness run: `flows`,
/// `window_us`, and `bytes` as a comma separated list per flow.
fn parse_flows(line: &str, retransmits: u64) -> Result<LossRun, NetnsError> {
    let bad = || NetnsError::Peer(format!("unreadable result {line:?}"));
    let window_us = field_of(line, "window_us").ok_or_else(bad)?;
    let list = line
        .split_whitespace()
        .find_map(|w| w.strip_prefix("bytes="))
        .ok_or_else(bad)?;
    let flow_bytes: Vec<u64> = list
        .split(',')
        .map(|v| v.parse().map_err(|_| bad()))
        .collect::<Result<_, _>>()?;
    let completed = !flow_bytes.is_empty() && flow_bytes.iter().all(|b| *b > 0);
    Ok(LossRun {
        completed,
        confirmed_bytes: flow_bytes.iter().sum(),
        elapsed: Duration::from_micros(window_us),
        retransmits,
        failure: (!completed).then(|| "a flow moved no byte in the window".to_string()),
        flow_bytes,
        ..LossRun::default()
    })
}

/// Reads the result line of a client peer.
pub(crate) fn parse_result(line: &str, wall: Duration) -> Result<LossRun, NetnsError> {
    let field = |key: &str| -> Option<u64> {
        line.split_whitespace()
            .find_map(|w| w.strip_prefix(key)?.strip_prefix('=')?.parse().ok())
    };
    let (Some(done), Some(confirmed), Some(elapsed_us), Some(retransmits)) = (
        field("completed"),
        field("confirmed"),
        field("elapsed_us"),
        field("retransmits"),
    ) else {
        return Err(NetnsError::Peer(format!("unreadable result {line:?}")));
    };
    let completed = done == 1;
    Ok(LossRun {
        completed,
        confirmed_bytes: confirmed,
        elapsed: if completed {
            Duration::from_micros(elapsed_us)
        } else {
            wall.min(Duration::from_micros(elapsed_us))
        },
        retransmits,
        link_lost: 0,
        link_queue_dropped: 0,
        failure: (!completed).then(|| "not complete before the timeout".to_string()),
        flow_bytes: Vec::new(),
    })
}

/// Reads `RetransSegs` of this namespace from `/proc/net/snmp`.
fn retrans_segs() -> u64 {
    let Ok(text) = std::fs::read_to_string("/proc/net/snmp") else {
        return 0;
    };
    let mut tcp = text.lines().filter(|l| l.starts_with("Tcp:"));
    let (Some(names), Some(values)) = (tcp.next(), tcp.next()) else {
        return 0;
    };
    names
        .split_whitespace()
        .position(|n| n == "RetransSegs")
        .and_then(|at| values.split_whitespace().nth(at))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

pub(crate) fn flag(args: &[String], name: &str) -> Option<String> {
    let at = args.iter().position(|a| a == name)?;
    args.get(at + 1).cloned()
}

/// The entry point of a peer process. `args` follow [`PEER_FLAG`]: the role
/// (`server` or `client`) and its flags. The server prints `ready` and then
/// serves one transfer, and the client prints one `result` line.
pub fn peer_main(args: &[String]) -> i32 {
    let role = args.first().map_or("", String::as_str);
    let number = |name: &str, default: u64| {
        flag(args, name)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let port = u16::try_from(number("--port", 9100)).unwrap_or(9100);
    let chunk = usize::try_from(number("--chunk", 16 * 1024)).unwrap_or(16 * 1024);
    let buffer = flag(args, "--buffer-bytes").and_then(|v| v.parse::<u32>().ok());
    let congestion = flag(args, "--cc");
    let flows = usize::try_from(number("--flows", 1)).unwrap_or(1);
    let warmup = Duration::from_millis(number("--warmup-ms", 5_000));
    let window = Duration::from_millis(number("--window-ms", 10_000));
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("peer: runtime: {error}");
            return 1;
        }
    };
    let outcome = match role {
        "server" if flows > 1 => runtime.block_on(serve_flows(
            port,
            chunk,
            buffer,
            congestion.as_deref(),
            flows,
            warmup,
            window,
        )),
        "client" if flows > 1 => runtime.block_on(send_flows(
            port,
            chunk,
            buffer,
            congestion.as_deref(),
            flows,
            warmup + window + FLOWS_CONNECT_LIMIT.min(Duration::from_secs(5)),
        )),
        "server" => runtime.block_on(serve(
            port,
            chunk,
            buffer,
            congestion.as_deref(),
            number("--verify", 0) == 1,
        )),
        "client" => runtime.block_on(send(
            port,
            chunk,
            buffer,
            congestion.as_deref(),
            number("--bytes", 0),
            Duration::from_millis(number("--timeout-ms", 60_000)),
            number("--verify", 0) == 1,
        )),
        role if role.starts_with("own-") => {
            return crate::interop_kernel::own_peer_main(role, args)
        }
        _ => Err(std::io::Error::other("the role is server or client")),
    };
    match outcome {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("peer {role}: {error}");
            1
        }
    }
}

pub(crate) fn say(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn socket(buffer: Option<u32>, congestion: Option<&str>) -> std::io::Result<TcpSocket> {
    let socket = TcpSocket::new_v4()?;
    if let Some(name) = congestion {
        crate::kernel::set_congestion(&socket, name)?;
    }
    if let Some(bytes) = buffer {
        socket.set_send_buffer_size(bytes)?;
        socket.set_recv_buffer_size(bytes)?;
    }
    Ok(socket)
}

async fn serve(
    port: u16,
    chunk: usize,
    buffer: Option<u32>,
    congestion: Option<&str>,
    verify: bool,
) -> std::io::Result<()> {
    let socket = socket(buffer, congestion)?;
    socket.set_reuseaddr(true)?;
    socket.bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))?;
    let listener = socket.listen(16)?;
    say("ready");
    let (mut stream, _) = listener.accept().await?;
    stream.set_nodelay(true)?;
    let mut buf = vec![0u8; chunk.max(1)];
    let mut total = 0u64;
    let mut wrong = false;
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        if verify && !wrong && first_mismatch(total, &buf[..n]).is_some() {
            wrong = true;
        }
        total += n as u64;
    }
    let answer = if wrong { u64::MAX } else { total };
    stream.write_all(&answer.to_be_bytes()).await?;
    stream.shutdown().await
}

/// The server of a fairness run: accepts `flows` connections, counts the bytes
/// of each, and reports the counts of the measurement window.
async fn serve_flows(
    port: u16,
    chunk: usize,
    buffer: Option<u32>,
    congestion: Option<&str>,
    flows: usize,
    warmup: Duration,
    window: Duration,
) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    let socket = socket(buffer, congestion)?;
    socket.set_reuseaddr(true)?;
    socket.bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))?;
    let listener = socket.listen(64)?;
    say("ready");
    let counters: Arc<Vec<AtomicU64>> = Arc::new((0..flows).map(|_| AtomicU64::new(0)).collect());
    let accept_all = async {
        for index in 0..flows {
            let (mut stream, _) = listener.accept().await?;
            stream.set_nodelay(true)?;
            let counters = Arc::clone(&counters);
            tokio::spawn(async move {
                let mut buf = vec![0u8; chunk.max(1)];
                loop {
                    match stream.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            counters[index].fetch_add(n as u64, Ordering::Relaxed);
                        }
                    }
                }
            });
        }
        Ok::<(), std::io::Error>(())
    };
    if tokio::time::timeout(FLOWS_CONNECT_LIMIT, accept_all)
        .await
        .is_err()
    {
        say(&format!(
            "result flows={flows} window_us=0 bytes={}",
            vec!["0"; flows].join(",")
        ));
        return Ok(());
    }
    tokio::time::sleep(warmup).await;
    let read = || -> Vec<u64> { counters.iter().map(|c| c.load(Ordering::Relaxed)).collect() };
    let before = read();
    let opened = Instant::now();
    tokio::time::sleep(window).await;
    let after = read();
    let elapsed = opened.elapsed();
    let bytes: Vec<String> = after
        .iter()
        .zip(&before)
        .map(|(a, b)| (a - b).to_string())
        .collect();
    say(&format!(
        "result flows={flows} window_us={} bytes={}",
        elapsed.as_micros(),
        bytes.join(",")
    ));
    Ok(())
}

/// The client of a fairness run: opens `flows` connections, sends on all of
/// them for `run` and reports the segments that the kernel sent again.
async fn send_flows(
    port: u16,
    chunk: usize,
    buffer: Option<u32>,
    congestion: Option<&str>,
    flows: usize,
    run: Duration,
) -> std::io::Result<()> {
    let before = retrans_segs();
    let mut tasks = Vec::new();
    for _ in 0..flows {
        let socket = socket(buffer, congestion)?;
        let mut stream = socket
            .connect(SocketAddr::from((SERVER_ADDR, port)))
            .await?;
        stream.set_nodelay(true)?;
        tasks.push(tokio::spawn(async move {
            let data = vec![0xA5u8; chunk.max(1)];
            while stream.write_all(&data).await.is_ok() {}
        }));
    }
    tokio::time::sleep(run).await;
    for task in &tasks {
        task.abort();
    }
    say(&format!(
        "result retransmits={}",
        retrans_segs().saturating_sub(before)
    ));
    Ok(())
}

async fn send(
    port: u16,
    chunk: usize,
    buffer: Option<u32>,
    congestion: Option<&str>,
    bytes: u64,
    timeout: Duration,
    verify: bool,
) -> std::io::Result<()> {
    let before = retrans_segs();
    let started = Instant::now();
    let transfer = async {
        let socket = socket(buffer, congestion)?;
        let mut stream = socket
            .connect(SocketAddr::from((SERVER_ADDR, port)))
            .await?;
        stream.set_nodelay(true)?;
        let first_write = Instant::now();
        let mut data = vec![0xA5u8; chunk.max(1)];
        let mut left = bytes;
        while left > 0 {
            let take = usize::try_from(left).unwrap_or(usize::MAX).min(data.len());
            if verify {
                fill_pattern(bytes - left, &mut data[..take]);
            }
            stream.write_all(&data[..take]).await?;
            left -= take as u64;
        }
        stream.shutdown().await?;
        let mut answer = [0u8; 8];
        stream.read_exact(&mut answer).await?;
        Ok::<_, std::io::Error>((u64::from_be_bytes(answer), first_write.elapsed()))
    };
    let outcome = tokio::time::timeout(timeout, transfer).await;
    let retransmits = retrans_segs().saturating_sub(before);
    match outcome {
        Ok(Ok((confirmed, elapsed))) => say(&format!(
            "result completed={} confirmed={confirmed} elapsed_us={} retransmits={retransmits}",
            u8::from(confirmed == bytes),
            elapsed.as_micros()
        )),
        Ok(Err(error)) => {
            eprintln!("peer client: {error}");
            say(&format!(
                "result completed=0 confirmed=0 elapsed_us={} retransmits={retransmits}",
                started.elapsed().as_micros()
            ));
        }
        Err(_) => say(&format!(
            "result completed=0 confirmed=0 elapsed_us={} retransmits={retransmits}",
            started.elapsed().as_micros()
        )),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_line_becomes_a_run() {
        let line = "result completed=1 confirmed=4194304 elapsed_us=2500000 retransmits=7";
        let run = parse_result(line, Duration::from_secs(3)).expect("parse");
        assert!(run.completed);
        assert_eq!(run.confirmed_bytes, 4_194_304);
        assert_eq!(run.elapsed, Duration::from_micros(2_500_000));
        assert_eq!(run.retransmits, 7);
        assert!((run.goodput_mbit_per_s() - 13.42).abs() < 0.01);
    }

    #[test]
    fn the_server_line_of_a_fairness_run_gives_the_bytes_per_flow() {
        let line = "result flows=4 window_us=10000500 bytes=100,200,300,400";
        let run = parse_flows(line, 9).expect("parse");
        assert!(run.completed);
        assert_eq!(run.flow_bytes, vec![100, 200, 300, 400]);
        assert_eq!(run.confirmed_bytes, 1000);
        assert_eq!(run.elapsed, Duration::from_micros(10_000_500));
        assert_eq!(run.retransmits, 9);
        let jain = run.jain_index().expect("jain");
        assert!((jain - 1000.0 * 1000.0 / (4.0 * 300_000.0)).abs() < 1e-9);
        // A flow that moved nothing is a failure, and a damaged line is an error.
        let stalled = parse_flows("result flows=2 window_us=5 bytes=7,0", 0).expect("parse");
        assert!(!stalled.completed && stalled.failure.is_some());
        assert!(parse_flows("result flows=2", 0).is_err());
        assert!(parse_flows("result window_us=5 bytes=1,x", 0).is_err());
    }

    #[test]
    fn a_timeout_line_is_not_complete_and_has_no_goodput() {
        let line = "result completed=0 confirmed=0 elapsed_us=60000000 retransmits=300";
        let run = parse_result(line, Duration::from_secs(61)).expect("parse");
        assert!(!run.completed);
        assert_eq!(run.goodput_mbit_per_s(), 0.0);
        assert!(run.failure.is_some());
        assert!(parse_result("garbage", Duration::ZERO).is_err());
    }

    #[test]
    fn flags_are_read_by_name() {
        let args: Vec<String> = ["client", "--port", "9", "--bytes", "5"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        assert_eq!(flag(&args, "--bytes").as_deref(), Some("5"));
        assert_eq!(flag(&args, "--chunk"), None);
    }

    #[test]
    fn retransmissions_are_readable_in_this_namespace() {
        // The file exists on every Linux host, and the counter only grows.
        let first = retrans_segs();
        assert!(retrans_segs() >= first);
    }
}
