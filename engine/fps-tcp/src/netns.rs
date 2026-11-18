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
use crate::workload::{LossConfig, LossRun};

/// The address of the client host.
pub const CLIENT_ADDR: Ipv4Addr = Ipv4Addr::new(10, 77, 1, 1);
/// The address of the server host.
pub const SERVER_ADDR: Ipv4Addr = Ipv4Addr::new(10, 77, 2, 1);
/// The hidden first argument that turns the binary into a peer process.
pub const PEER_FLAG: &str = "--netns-peer";

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

fn run_command(cmd: &mut Command) -> Result<String, NetnsError> {
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

fn ip(args: &[&str]) -> Result<String, NetnsError> {
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

    /// Returns the name of the client namespace.
    pub fn client_ns(&self) -> &str {
        &self.client
    }

    /// Returns the name of the server namespace.
    pub fn server_ns(&self) -> &str {
        &self.server
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
fn lines_of(child: &mut Child) -> Result<mpsc::Receiver<String>, NetnsError> {
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

fn spawn_peer(exe: &Path, ns: &str, args: &[String]) -> Result<Child, NetnsError> {
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

/// Reads the result line of a client peer.
fn parse_result(line: &str, wall: Duration) -> Result<LossRun, NetnsError> {
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

fn flag(args: &[String], name: &str) -> Option<String> {
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
        "server" => runtime.block_on(serve(port, chunk, buffer, congestion.as_deref())),
        "client" => runtime.block_on(send(
            port,
            chunk,
            buffer,
            congestion.as_deref(),
            number("--bytes", 0),
            Duration::from_millis(number("--timeout-ms", 60_000)),
        )),
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

fn say(line: &str) {
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
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        total += n as u64;
    }
    stream.write_all(&total.to_be_bytes()).await?;
    stream.shutdown().await
}

async fn send(
    port: u16,
    chunk: usize,
    buffer: Option<u32>,
    congestion: Option<&str>,
    bytes: u64,
    timeout: Duration,
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
        let data = vec![0xA5u8; chunk.max(1)];
        let mut left = bytes;
        while left > 0 {
            let take = usize::try_from(left).unwrap_or(usize::MAX).min(data.len());
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
