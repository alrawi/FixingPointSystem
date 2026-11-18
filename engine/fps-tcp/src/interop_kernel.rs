// SPDX-License-Identifier: GPL-3.0-only
//! The own stack against the kernel stack over a veth pair with netem
//! (TRN-26).
//!
//! The own stack runs over an AF_XDP queue on one end of the namespace link of
//! [`crate::netns`], and the kernel stack is a socket peer in the namespace at
//! the other end. The link is the one of TRN-22: a client namespace, a router
//! namespace with the `tc` qdiscs on both of its interfaces (so the kernel sees
//! a drop in the middle of a network and not in its own qdisc), and a server
//! namespace. In [`Pairing::OwnClient`] the own stack sits on `vc` and opens
//! the connection to a kernel server, and in [`Pairing::OwnServer`] it sits on
//! `vs` and accepts the connection of a kernel client.
//!
//! The own stack has no ARP requests of its own, so the router interface that
//! faces it gets the MAC address that the own stack uses for the next hop (the
//! MAC address of the host at the far end in its tables), and the own stack
//! answers the ARP requests of the router for its own host. The kernel and the
//! own stack both complete the TCP checksum on send: the transmit offload of
//! every interface is off, because a frame that reaches an AF_XDP socket has
//! no later hop that would complete a partial checksum.
//!
//! The own stack lives in a helper process, this binary again, started with
//! `ip netns exec` and the flag of [`crate::netns::PEER_FLAG`] and one of the
//! roles `own-client` and `own-server` ([`own_peer_main`]), so the AF_XDP
//! queue opens in the right namespace and the harness needs no `setns`. The
//! helper prints the options that its tap read on the wire, which the parent
//! parses into a [`WireSummary`].

use std::io;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fps_backend_afxdp::memory::frame_pair;
use fps_backend_afxdp::{open_port, BindMode, FrameIo, PortConfig, QueueConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::clock::MonotonicClock;
use crate::impair::ImpairConfig;
use crate::interop::{PairAddresses, Pairing};
use crate::netns::{
    flag, lines_of, parse_result, run_command, say, spawn_peer, NetnsError, NetnsLink,
};
use crate::own::{OwnConfig, OwnStack, PoolError, Pools};
use crate::wiretap::{SharedTap, TappedFrames, WireSummary};
use crate::workload::{drain, drain_checked, send_bulk, send_pattern, LossConfig, LossRun};

/// The pools of the own stack on the namespace link: the client host at
/// 10.77.1.1 and the server host at 10.77.2.1, the addresses of
/// [`crate::netns`].
///
/// # Errors
///
/// Returns an error when the pools cannot be built.
pub fn netns_pools() -> Result<Pools, PoolError> {
    Pools::sized("10.77.1.0/24", 1, "10.77.2.0/24", 1, None)
}

/// The options of the own stack that a run sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnOptions {
    /// Window scaling.
    pub window_scale: bool,
    /// Selective acknowledgment.
    pub sack: bool,
    /// Timestamps.
    pub timestamps: bool,
}

impl Default for OwnOptions {
    fn default() -> Self {
        Self {
            window_scale: true,
            sack: true,
            timestamps: true,
        }
    }
}

/// The switches of TRN-34 of the own stack, all off by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LossSwitches {
    /// Sender pacing.
    pub pacing: bool,
    /// HyStart for CUBIC.
    pub hystart: bool,
    /// RACK loss detection.
    pub rack: bool,
    /// Tail loss probes.
    pub tlp: bool,
}

impl LossSwitches {
    /// Every switch on.
    pub const ALL: Self = Self {
        pacing: true,
        hystart: true,
        rack: true,
        tlp: true,
    };
}

/// The settings of one kernel interop run.
#[derive(Clone, Debug)]
pub struct KernelInteropConfig {
    /// Which end the own stack plays.
    pub pairing: Pairing,
    /// The options of the own stack.
    pub own: OwnOptions,
    /// The switches of TRN-34 of the own stack.
    pub loss: LossSwitches,
    /// Whether the kernel keeps its defaults. `false` turns timestamps, SACK,
    /// and window scaling off in the namespace of the kernel peer.
    pub kernel_options: bool,
    /// The transfer.
    pub transfer: LossConfig,
    /// The settings of one direction of the link.
    pub link: ImpairConfig,
    /// The MTU of every interface.
    pub mtu: usize,
}

/// What one kernel interop run measured.
#[derive(Clone, Debug)]
pub struct KernelInterop {
    /// The outcome of the transfer.
    pub run: LossRun,
    /// The options that the tap read at the own stack's interface.
    pub wire: WireSummary,
    /// The segments that the own stack sent again.
    pub own_retransmits: u64,
}

fn peer_error(what: impl Into<String>) -> NetnsError {
    NetnsError::Peer(what.into())
}

/// Reads lines of a peer until `want` returns a value or the time is up.
fn wait_for<T>(
    lines: &std::sync::mpsc::Receiver<String>,
    limit: Duration,
    mut want: impl FnMut(&str) -> Option<T>,
) -> Result<T, NetnsError> {
    let deadline = Instant::now() + limit;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match lines.recv_timeout(left) {
            Ok(line) => {
                if let Some(found) = want(&line) {
                    return Ok(found);
                }
            }
            Err(_) => return Err(peer_error("the peer did not report in time")),
        }
    }
}

fn own_args(role: &str, iface: &str, config: &KernelInteropConfig) -> Vec<String> {
    let t = &config.transfer;
    let on = |b: bool| u8::from(b).to_string();
    vec![
        role.to_string(),
        "--iface".into(),
        iface.into(),
        "--port".into(),
        t.port.to_string(),
        "--chunk".into(),
        t.chunk.to_string(),
        "--bytes".into(),
        t.bytes.to_string(),
        "--timeout-ms".into(),
        t.timeout.as_millis().to_string(),
        "--verify".into(),
        on(t.verify),
        "--own-wscale".into(),
        on(config.own.window_scale),
        "--own-sack".into(),
        on(config.own.sack),
        "--own-timestamps".into(),
        on(config.own.timestamps),
        "--own-pacing".into(),
        on(config.loss.pacing),
        "--own-hystart".into(),
        on(config.loss.hystart),
        "--own-rack".into(),
        on(config.loss.rack),
        "--own-tlp".into(),
        on(config.loss.tlp),
        "--mtu".into(),
        config.mtu.to_string(),
    ]
}

fn kernel_args(role: &str, config: &KernelInteropConfig) -> Vec<String> {
    let t = &config.transfer;
    let mut args = vec![
        role.to_string(),
        "--port".into(),
        t.port.to_string(),
        "--chunk".into(),
        t.chunk.to_string(),
        "--verify".into(),
        u8::from(t.verify).to_string(),
    ];
    if role == "client" {
        args.extend([
            "--bytes".into(),
            t.bytes.to_string(),
            "--timeout-ms".into(),
            t.timeout.as_millis().to_string(),
        ]);
    }
    args
}

fn wire_of(
    lines: &std::sync::mpsc::Receiver<String>,
    limit: Duration,
) -> Result<WireSummary, NetnsError> {
    wait_for(lines, limit, |l| {
        l.strip_prefix("wire ").and_then(WireSummary::from_line)
    })
}

/// Runs one transfer between the own stack and the kernel over a fresh
/// namespace link, and returns what happened. `exe` is a binary that calls
/// [`crate::netns::peer_main`] when it starts with the peer flag.
///
/// # Errors
///
/// Returns an error when the link or a peer process cannot be set up. A
/// transfer that does not complete is a [`LossRun`] with `completed` false,
/// not an error.
pub fn own_kernel_transfer(
    exe: &Path,
    prefix: &str,
    config: &KernelInteropConfig,
) -> Result<KernelInterop, NetnsError> {
    let pools = netns_pools().map_err(|e| peer_error(e.to_string()))?;
    let addresses = PairAddresses::of(&pools).map_err(|e| peer_error(e.to_string()))?;
    if addresses.client_ip != crate::netns::CLIENT_ADDR
        || addresses.server_ip != crate::netns::SERVER_ADDR
    {
        return Err(peer_error(format!(
            "the pools of the own stack are at {} and {}, not at the namespace addresses",
            addresses.client_ip, addresses.server_ip
        )));
    }
    let ns = NetnsLink::up(prefix, &config.link, config.mtu)?;
    if let Err(error) = ns.complete_checksums(exe) {
        eprintln!("interop: could not turn the transmit checksum offload off: {error}");
    }
    match config.pairing {
        Pairing::OwnClient => ns.set_router_mac("rc", addresses.server_mac)?,
        Pairing::OwnServer => ns.set_router_mac("rs", addresses.client_mac)?,
    }
    if !config.kernel_options {
        let kernel_ns = match config.pairing {
            Pairing::OwnClient => ns.server_ns(),
            Pairing::OwnServer => ns.client_ns(),
        };
        run_command(Command::new("ip").args([
            "netns",
            "exec",
            kernel_ns,
            "sysctl",
            "-qw",
            "net.ipv4.tcp_timestamps=0",
            "net.ipv4.tcp_sack=0",
            "net.ipv4.tcp_window_scaling=0",
        ]))?;
    }
    let wait = config.transfer.timeout + Duration::from_secs(30);
    match config.pairing {
        Pairing::OwnClient => {
            let mut server = spawn_peer(exe, ns.server_ns(), &kernel_args("server", config))?;
            let result = (|| {
                let server_lines = lines_of(&mut server)?;
                wait_for(&server_lines, Duration::from_secs(15), |l| {
                    (l == "ready").then_some(())
                })?;
                let started = Instant::now();
                let mut client =
                    spawn_peer(exe, ns.client_ns(), &own_args("own-client", "vc", config))?;
                let client_lines = lines_of(&mut client)?;
                let outcome = wait_for(&client_lines, wait, |l| {
                    l.starts_with("result ").then(|| l.to_string())
                });
                let wire = wire_of(&client_lines, Duration::from_secs(2));
                let _ = client.kill();
                let _ = client.wait();
                let line = outcome?;
                let run = parse_result(&line, started.elapsed())?;
                Ok(KernelInterop {
                    own_retransmits: run.retransmits,
                    run,
                    wire: wire?,
                })
            })();
            let _ = server.kill();
            let _ = server.wait();
            drop(ns);
            result
        }
        Pairing::OwnServer => {
            let mut own = spawn_peer(exe, ns.server_ns(), &own_args("own-server", "vs", config))?;
            let result = (|| {
                let own_lines = lines_of(&mut own)?;
                wait_for(&own_lines, Duration::from_secs(30), |l| {
                    (l == "ready").then_some(())
                })?;
                let started = Instant::now();
                let mut client = spawn_peer(exe, ns.client_ns(), &kernel_args("client", config))?;
                let client_lines = lines_of(&mut client)?;
                let outcome = wait_for(&client_lines, wait, |l| {
                    l.starts_with("result ").then(|| l.to_string())
                });
                let _ = client.kill();
                let _ = client.wait();
                let line = outcome?;
                let run = parse_result(&line, started.elapsed())?;
                // The own server reports after a short grace for the last
                // acknowledgments.
                let wire = wire_of(&own_lines, Duration::from_secs(10))?;
                let own_retransmits = wait_for(&own_lines, Duration::from_secs(2), |l| {
                    l.strip_prefix("own_retransmits=")?.parse().ok()
                })
                .unwrap_or(0);
                Ok(KernelInterop {
                    run,
                    wire,
                    own_retransmits,
                })
            })();
            let _ = own.kill();
            let _ = own.wait();
            drop(ns);
            result
        }
    }
}

/// The frames that one pump pass moves are few, so the helper polls often.
const IDLE_SLEEP: Duration = Duration::from_micros(30);

fn open_queue(iface: &str) -> io::Result<Box<dyn FrameIo>> {
    let config = PortConfig {
        interface: iface.to_owned(),
        queues: vec![0],
        queue: QueueConfig {
            mode: BindMode::Auto,
            ..QueueConfig::default()
        },
    };
    // The kernel releases the queue of a closed socket in a work queue, so a
    // bind right after a close can meet EBUSY for a few milliseconds.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match open_port(&config) {
            Ok(mut port) => {
                return match port.queues.pop() {
                    Some(queue) => Ok(Box::new(queue)),
                    None => Err(io::Error::other("the port has no queue")),
                };
            }
            Err(e) if e.raw_os_error() == Some(16) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(io::Error::other(format!("opening {iface}: {e}"))),
        }
    }
}

/// The entry point of the helper process of the own stack. `role` is
/// `own-client` or `own-server`, and `args` are the flags that
/// [`own_args`] writes. Returns the exit code.
pub fn own_peer_main(role: &str, args: &[String]) -> i32 {
    match run_own(role, args) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("peer {role}: {error}");
            1
        }
    }
}

fn run_own(role: &str, args: &[String]) -> io::Result<()> {
    let text = |name: &str| flag(args, name).unwrap_or_default();
    let number = |name: &str, default: u64| text(name).parse().unwrap_or(default);
    let on = |name: &str| text(name) != "0";
    let set = |name: &str| text(name) == "1";
    let iface = text("--iface");
    let port = u16::try_from(number("--port", 9100)).unwrap_or(9100);
    let chunk = usize::try_from(number("--chunk", 16 * 1024)).unwrap_or(16 * 1024);
    let bytes = number("--bytes", 0);
    let timeout = Duration::from_millis(number("--timeout-ms", 60_000));
    let verify = number("--verify", 0) == 1;
    let config = OwnConfig {
        window_scale: on("--own-wscale"),
        sack: on("--own-sack"),
        timestamps: on("--own-timestamps"),
        pacing: set("--own-pacing"),
        hystart: set("--own-hystart"),
        rack: set("--own-rack"),
        tlp: set("--own-tlp"),
        mtu: usize::try_from(number("--mtu", 1500)).unwrap_or(1500),
        ..OwnConfig::default()
    };
    let frame_len = config.mtu + 18;
    let tap = SharedTap::new();
    let queue: Box<dyn FrameIo> = Box::new(TappedFrames::new(open_queue(&iface)?, tap.clone()));
    // The arm that the pairing does not use faces nothing.
    let (idle, _gone) = frame_pair(1, frame_len);
    let idle: Box<dyn FrameIo> = Box::new(idle);
    let client_side = role == "own-client";
    let (client, server) = if client_side {
        (queue, idle)
    } else {
        (idle, queue)
    };
    let pools = netns_pools().map_err(io::Error::other)?;
    let stack = OwnStack::with_links_and_pools(
        client,
        server,
        config,
        pools,
        Arc::new(MonotonicClock::new()),
    )
    .map_err(io::Error::other)?;
    let stop = Arc::new(AtomicBool::new(false));
    let pump = {
        let (stack, stop) = (stack.clone(), Arc::clone(&stop));
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if !stack.pump() {
                    std::thread::sleep(IDLE_SLEEP);
                }
            }
        })
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    let outcome = if client_side {
        runtime.block_on(own_send(&stack, port, chunk, bytes, timeout, verify, &tap))
    } else {
        runtime.block_on(own_serve(&stack, port, chunk, verify, timeout, &tap))
    };
    stop.store(true, Ordering::Relaxed);
    let _ = pump.join();
    outcome
}

async fn own_send(
    stack: &OwnStack,
    port: u16,
    chunk: usize,
    bytes: u64,
    timeout: Duration,
    verify: bool,
    tap: &SharedTap,
) -> io::Result<()> {
    let started = Instant::now();
    let transfer = async {
        let mut session = stack.connect(port).await?;
        let first_write = Instant::now();
        if verify {
            send_pattern(&mut session, bytes, chunk).await?;
        } else {
            send_bulk(&mut session, bytes, chunk).await?;
        }
        let mut answer = [0u8; 8];
        session.read_exact(&mut answer).await?;
        Ok::<_, io::Error>((u64::from_be_bytes(answer), first_write.elapsed()))
    };
    let outcome = tokio::time::timeout(timeout, transfer).await;
    let retransmits = stack.counters().client.retransmits;
    match outcome {
        Ok(Ok((confirmed, elapsed))) => say(&format!(
            "result completed={} confirmed={confirmed} elapsed_us={} retransmits={retransmits}",
            u8::from(confirmed == bytes),
            elapsed.as_micros()
        )),
        Ok(Err(error)) => {
            eprintln!("peer own-client: {error}");
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
    // The parent reads the result first and the tap after it.
    say(&format!("wire {}", tap.summary().to_line()));
    Ok(())
}

async fn own_serve(
    stack: &OwnStack,
    port: u16,
    chunk: usize,
    verify: bool,
    timeout: Duration,
    tap: &SharedTap,
) -> io::Result<()> {
    stack.listen(port)?;
    say("ready");
    let serve = async {
        let mut session = stack.accept(port).await?;
        let received = if verify {
            drain_checked(&mut session, chunk).await?
        } else {
            drain(&mut session, chunk).await?
        };
        session.write_all(&received.to_be_bytes()).await?;
        session.shutdown().await?;
        // The answer must reach the peer before this process ends, so the
        // session stays open until the peer has acknowledged it or a grace
        // time is over.
        tokio::time::sleep(Duration::from_millis(500)).await;
        Ok::<_, io::Error>(received)
    };
    let outcome = tokio::time::timeout(timeout + Duration::from_secs(20), serve).await;
    say(&format!("wire {}", tap.summary().to_line()));
    say(&format!(
        "own_retransmits={}",
        stack.counters().server.retransmits
    ));
    match outcome {
        Ok(Ok(count)) => say(&format!("served count={count}")),
        Ok(Err(error)) => eprintln!("peer own-server: {error}"),
        Err(_) => eprintln!("peer own-server: no connection before the timeout"),
    }
    Ok(())
}
