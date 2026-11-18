// SPDX-License-Identifier: GPL-3.0-only
//! The own stack against the kernel stack over a veth pair with netem
//! (TRN-26).
//!
//! The own stack runs over an AF_XDP queue on one end of the namespace link of
//! TRN-22, and the kernel stack is a socket peer in the namespace at the other
//! end, with the netem qdiscs in the router namespace between them. Both
//! directions run: the own stack opens and the kernel accepts, and the other
//! way round. The loss workload moves a position dependent pattern that the
//! receiver checks byte by byte, at 1 percent loss with 20 ms in each
//! direction (and over a clean link, which needs no qdisc). The tap in the own
//! stack's queue reads every frame with the smoltcp parser, and the checks
//! compare the options of the handshake and their use with what the RFCs ask,
//! with the kernel at its defaults and with timestamps, SACK, and window
//! scaling off in its namespace, and with the own options off.
//!
//! The scenarios need root, `ip`, `tc`, AF_XDP, and for the lossy ones the
//! `sch_netem` module. As a normal user the binary runs itself again through
//! `sudo -n` and skips with a message when that is not possible. `CI=true`
//! turns every skip into a failure, and so do `FPS_REQUIRE_AFXDP=1` (root,
//! sudo, `ip`, AF_XDP) and `FPS_REQUIRE_NETEM=1` (the qdisc). Complete TCP
//! checksums need the transmit offload off on every interface: the harness
//! uses `ethtool` where it is installed and otherwise this binary, which
//! sets it with the ethtool ioctl (the hosted runner image has no `ethtool`).
//!
//! The target has its own `main` (`harness = false`), because the peers are
//! this binary again, started with `ip netns exec` and the flag `--netns-peer`.
//!
//! `FPS_INTEROP_RESULTS` names a directory that receives `interop-kernel.txt`
//! with one line per scenario.

use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::Duration;

use fps_tcp::impair::ImpairConfig;
use fps_tcp::interop::Pairing;
use fps_tcp::interop_kernel::{
    own_kernel_transfer, KernelInterop, KernelInteropConfig, LossSwitches, OwnOptions,
};
use fps_tcp::netns::{self, NetnsError, OFFLOAD_FLAG, PEER_FLAG};
use fps_tcp::wiretap::{violations, OwnSide};
use fps_tcp::workload::LossConfig;

const KIB: u64 = 1024;
const ONE_PERCENT: u32 = 10_000;
const MTU: usize = 1500;

fn flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| v == "1")
}

fn ci() -> bool {
    std::env::var("CI").is_ok_and(|v| v == "true")
}

/// A scenario that cannot run for lack of root, sudo, `ip`, or AF_XDP.
fn skip(what: &str, why: &str) -> Result<(), String> {
    if ci() || flag("FPS_REQUIRE_AFXDP") {
        Err(format!("{what} cannot run here and CI requires it: {why}"))
    } else {
        eprintln!("skipping {what}: {why}");
        Ok(())
    }
}

/// A scenario that the kernel cannot run for lack of a qdisc.
fn skip_qdisc(what: &str, why: &str) -> Result<(), String> {
    if ci() || flag("FPS_REQUIRE_NETEM") {
        Err(format!("{what} cannot run here and CI requires it: {why}"))
    } else {
        eprintln!("skipping {what}: {why}");
        Ok(())
    }
}

fn exe() -> PathBuf {
    std::env::current_exe().expect("the path of this binary")
}

fn missing_qdisc(error: &NetnsError) -> bool {
    match error {
        NetnsError::NetemMissing(_) => true,
        NetnsError::Command { detail, .. } => detail.contains("kind is unknown"),
        NetnsError::Peer(_) => false,
    }
}

struct Scenario {
    name: String,
    pairing: Pairing,
    link: ImpairConfig,
    kib: u64,
    own: OwnOptions,
    kernel_options: bool,
    /// The switches of TRN-34 of the own stack.
    loss: LossSwitches,
}

impl Scenario {
    /// Whether the link drops frames, by loss or by a full queue.
    fn drops(&self) -> bool {
        self.link.loss_ppm > 0 || self.link.rate_bps > 0
    }
}

const ALL: OwnOptions = OwnOptions {
    window_scale: true,
    sack: true,
    timestamps: true,
};
const NONE: OwnOptions = OwnOptions {
    window_scale: false,
    sack: false,
    timestamps: false,
};

/// A link without delay or loss, which needs no qdisc.
fn clean() -> ImpairConfig {
    ImpairConfig::default()
}

/// 1 percent loss in each direction with 20 ms in each direction (netem).
fn one_percent() -> ImpairConfig {
    ImpairConfig {
        loss_ppm: ONE_PERCENT,
        delay_us: 20_000,
        ..ImpairConfig::default()
    }
}

/// A bottleneck of 20 Mbit/s with a queue of 32 KiB, which drops by tail
/// drop (a `tbf` qdisc, no `netem`).
fn bottleneck() -> ImpairConfig {
    ImpairConfig {
        rate_bps: 20_000_000,
        queue_bytes: 32 * 1024,
        ..ImpairConfig::default()
    }
}

fn scenarios() -> Vec<Scenario> {
    let mut all = Vec::new();
    for (pairing, from, to) in [
        (Pairing::OwnClient, "own client", "kernel server"),
        (Pairing::OwnServer, "kernel client", "own server"),
    ] {
        let mut add = |what: &str, link, kib, own, kernel_options| {
            all.push(Scenario {
                name: format!("{from} to {to}, {what}"),
                pairing,
                link,
                kib,
                own,
                kernel_options,
                loss: LossSwitches::default(),
            });
        };
        add("clean link", clean(), 1024, ALL, true);
        add("bottleneck with tail drop", bottleneck(), 2048, ALL, true);
        add("1% loss and 20 ms", one_percent(), 2048, ALL, true);
        add(
            "kernel options off, bottleneck",
            bottleneck(),
            1024,
            ALL,
            false,
        );
        add(
            "own options off, bottleneck",
            bottleneck(),
            1024,
            NONE,
            true,
        );
        // TRN-34: the same link with pacing, HyStart, RACK, and tail loss
        // probes on, for the comparison of the retransmissions of the own
        // stack with those of the kernel.
        all.push(Scenario {
            name: format!("{from} to {to}, {SWITCHED_ROW}"),
            pairing,
            link: bottleneck(),
            kib: 2048,
            own: ALL,
            kernel_options: true,
            loss: LossSwitches::ALL,
        });
        all.push(Scenario {
            name: format!("{from} to {to}, {SWITCHED_LOSS_ROW}"),
            pairing,
            link: one_percent(),
            kib: 2048,
            own: ALL,
            kernel_options: true,
            loss: LossSwitches::ALL,
        });
    }
    all
}

/// The name of the TRN-34 row on the bottleneck.
const SWITCHED_ROW: &str = "bottleneck with tail drop, loss switches on";
/// The name of the TRN-34 row at 1 percent loss.
const SWITCHED_LOSS_ROW: &str = "1% loss and 20 ms, loss switches on";

fn config_of(scenario: &Scenario) -> KernelInteropConfig {
    KernelInteropConfig {
        pairing: scenario.pairing,
        own: scenario.own,
        loss: scenario.loss,
        kernel_options: scenario.kernel_options,
        transfer: LossConfig {
            bytes: scenario.kib * KIB,
            timeout: Duration::from_secs(90),
            port: 9500,
            verify: true,
            ..LossConfig::default()
        },
        link: scenario.link,
        mtu: MTU,
    }
}

/// What the kernel puts on the wire, on top of the checks that hold for any
/// peer.
fn kernel_expectations(scenario: &Scenario, result: &KernelInterop) -> Vec<String> {
    let mut out = Vec::new();
    let wire = &result.wire;
    let (Some(syn), Some(syn_ack)) = (wire.syn, wire.syn_ack) else {
        return vec!["no handshake".into()];
    };
    let kernel = match scenario.pairing {
        Pairing::OwnClient => syn_ack,
        Pairing::OwnServer => syn,
    };
    if kernel.mss != Some(1460) {
        out.push(format!("the kernel MSS is {:?}, not 1460", kernel.mss));
    }
    if scenario.kernel_options {
        // The kernel keeps its defaults: it offers all three as an opener, and
        // as an acceptor it answers what the own stack offered.
        let offered = |on: bool| scenario.pairing == Pairing::OwnServer || on;
        if kernel.sack_permitted != offered(scenario.own.sack) {
            out.push(format!(
                "kernel SACK permitted is {}",
                kernel.sack_permitted
            ));
        }
        if kernel.timestamps != offered(scenario.own.timestamps) {
            out.push(format!("kernel timestamps are {}", kernel.timestamps));
        }
        if kernel.window_scale.is_some() != offered(scenario.own.window_scale) {
            out.push(format!("kernel window scale is {:?}", kernel.window_scale));
        }
    } else if kernel.sack_permitted || kernel.timestamps || kernel.window_scale.is_some() {
        out.push(format!(
            "the kernel offers options that are off: {kernel:?}"
        ));
    }
    out
}

/// The row text, the goodput in Mbit/s, and the segments that the sender sent
/// again.
type Row = (String, f64, u64);

fn run_scenario(index: usize, scenario: &Scenario) -> Result<Option<Row>, String> {
    let prefix = format!("fpsix{}x{index}", std::process::id() % 100_000);
    let config = config_of(scenario);
    let result = match own_kernel_transfer(&exe(), &prefix, &config) {
        Ok(result) => result,
        Err(error) if missing_qdisc(&error) => {
            skip_qdisc(&scenario.name, &error.to_string())?;
            return Ok(None);
        }
        Err(error) => return Err(format!("{}: {error}", scenario.name)),
    };
    let (run, wire) = (&result.run, &result.wire);
    let line = wire.to_line();
    if !run.completed {
        return Err(format!(
            "{}: the transfer did not complete: {:?}\nwire: {line}",
            scenario.name, run.failure
        ));
    }
    let bytes = scenario.kib * KIB;
    if run.confirmed_bytes != bytes {
        return Err(format!(
            "{}: confirmed {} of {bytes}",
            scenario.name, run.confirmed_bytes
        ));
    }
    let side = OwnSide {
        opener: scenario.pairing == Pairing::OwnClient,
        mtu: MTU,
        window_scale: scenario.own.window_scale,
        sack: scenario.own.sack,
        timestamps: scenario.own.timestamps,
        rcv_buffer: 1024 * 1024,
    };
    let mut found = violations(wire, &side, scenario.drops(), None);
    found.extend(kernel_expectations(scenario, &result));
    if wire.bad_checksums > 0 {
        found.push("wrong checksums: the transmit offload is still on".into());
    }
    let sent_again = match scenario.pairing {
        Pairing::OwnClient => result.own_retransmits,
        Pairing::OwnServer => run.retransmits,
    };
    if scenario.drops() && sent_again == 0 {
        found.push("the link drops frames and the sender never sent one again".into());
    }
    if !found.is_empty() {
        return Err(format!("{}: {found:?}\nwire: {line}", scenario.name));
    }
    let (syn, syn_ack) = (
        wire.syn.unwrap_or_default(),
        wire.syn_ack.unwrap_or_default(),
    );
    let text = format!(
        "{} | {:.2} Mbit/s | sender retransmissions {sent_again} | SYN mss={:?} ws={:?} sack={} ts={} | SYN-ACK mss={:?} ws={:?} sack={} ts={} | agreed ws={} sack={} ts={} | sack segments {} | wire: {line}",
        scenario.name,
        run.goodput_mbit_per_s(),
        syn.mss,
        syn.window_scale,
        syn.sack_permitted,
        syn.timestamps,
        syn_ack.mss,
        syn_ack.window_scale,
        syn_ack.sack_permitted,
        syn_ack.timestamps,
        wire.window_scale_agreed(),
        wire.sack_agreed(),
        wire.timestamps_agreed(),
        wire.acceptor.sack_segments,
    );
    println!("ok {text}");
    Ok(Some((text, run.goodput_mbit_per_s(), sent_again)))
}

/// TRN-34: on the short tail-drop queue, the own sender with the switches on
/// sends at most twice as many segments again as the kernel sender does (with
/// a floor for the probing of the queue that a single run cannot avoid) and
/// reaches at least 90 percent of its goodput.
fn compare_with_the_kernel(measures: &[(String, f64, u64)]) -> Result<(), String> {
    let find = |name: &str| measures.iter().find(|(n, _, _)| n == name);
    let own = find(&format!("own client to kernel server, {SWITCHED_ROW}"));
    let kernel = find("kernel client to own server, bottleneck with tail drop");
    let (Some((_, own_goodput, own_sent)), Some((_, kernel_goodput, kernel_sent))) = (own, kernel)
    else {
        return Ok(());
    };
    println!(
        "the own sender with the switches on: {own_goodput:.2} Mbit/s and {own_sent} segments sent again; the kernel sender: {kernel_goodput:.2} Mbit/s and {kernel_sent}"
    );
    let bound = 2 * kernel_sent + RETRANSMIT_FLOOR;
    if *own_sent > bound {
        return Err(format!(
            "the own sender sent {own_sent} segments again on the short queue, the bound is {bound} (the kernel sent {kernel_sent})"
        ));
    }
    if *own_goodput < 0.9 * kernel_goodput {
        return Err(format!(
            "the own sender reached {own_goodput:.2} Mbit/s, below 90% of the {kernel_goodput:.2} Mbit/s of the kernel sender"
        ));
    }
    Ok(())
}

/// The retransmissions that one run of the probing of a 32 KiB queue may cost
/// beyond twice those of the kernel, so that a loaded host does not decide.
const RETRANSMIT_FLOOR: u64 = 8;

fn leftover_namespaces() -> bool {
    Command::new("ip")
        .args(["netns", "list"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("fpsix"))
}

fn has_af_xdp() -> bool {
    std::fs::read_to_string("/proc/net/protocols").is_ok_and(|t| t.contains("XDP"))
}

fn run_all() -> Result<(), String> {
    if Command::new("ip").arg("-V").output().is_err() {
        return skip("the kernel interop scenarios", "ip is not installed");
    }
    if !has_af_xdp() {
        return skip(
            "the kernel interop scenarios",
            "the kernel has no AF_XDP sockets",
        );
    }
    let mut rows = Vec::new();
    let mut measures: Vec<(String, f64, u64)> = Vec::new();
    for (index, scenario) in scenarios().iter().enumerate() {
        if let Some((text, goodput, retransmits)) = run_scenario(index, scenario)? {
            rows.push(text);
            measures.push((scenario.name.clone(), goodput, retransmits));
        }
    }
    compare_with_the_kernel(&measures)?;
    if leftover_namespaces() {
        return Err("the namespaces were not removed".into());
    }
    if let Ok(dir) = std::env::var("FPS_INTEROP_RESULTS") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut text = rows.join("\n");
        text.push('\n');
        std::fs::write(dir.join("interop-kernel.txt"), text).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn run_again_as_root() -> Result<(), String> {
    let sudo = Command::new("sudo").args(["-n", "true"]).status();
    if !sudo.is_ok_and(|s| s.success()) {
        return skip(
            "the kernel interop scenarios",
            "not root and no passwordless sudo",
        );
    }
    // sudo drops the environment, so the child learns the requirements here.
    let mut command = Command::new("sudo");
    command.args(["-n", "env"]);
    for key in [
        "CI",
        "FPS_REQUIRE_AFXDP",
        "FPS_REQUIRE_NETEM",
        "FPS_INTEROP_RESULTS",
    ] {
        if let Ok(value) = std::env::var(key) {
            command.arg(format!("{key}={value}"));
        }
    }
    let status = command.arg(exe()).status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("the scenarios failed under sudo".into())
    }
}

/// Turns the transmit checksum offload of `dev` off with the ethtool ioctl
/// (`ETHTOOL_STXCSUM`), for a host without the `ethtool` command. The binary
/// runs in the namespace of the interface, started by
/// `NetnsLink::complete_checksums`.
fn tx_checksum_off(dev: &str) -> std::io::Result<()> {
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
        return Err(std::io::Error::last_os_error());
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
    let error = std::io::Error::last_os_error();
    // SAFETY: `sock` is the descriptor from above, which nothing else owns.
    unsafe { libc::close(sock) };
    if result < 0 {
        Err(error)
    } else {
        Ok(())
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some(OFFLOAD_FLAG) {
        return match args.get(1).map(|dev| tx_checksum_off(dev)) {
            Some(Ok(())) => ExitCode::SUCCESS,
            Some(Err(error)) => {
                eprintln!("offload off: {error}");
                ExitCode::FAILURE
            }
            None => ExitCode::FAILURE,
        };
    }
    if args.first().map(String::as_str) == Some(PEER_FLAG) {
        let code = netns::peer_main(&args[1..]);
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    let outcome = if netns::is_root() {
        run_all()
    } else {
        run_again_as_root()
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("interop_kernel test failed: {message}");
            ExitCode::FAILURE
        }
    }
}
