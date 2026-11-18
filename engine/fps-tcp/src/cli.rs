// SPDX-License-Identifier: GPL-3.0-only
//! The command line of the bench binary `examples/tcp_bench.rs`.
//!
//! The command takes plain arguments only, so a relay can run it on a host
//! without a shell. [`run`] parses them, runs the workloads, prints the report,
//! and writes the `fps-bench/v1` document when `--out` names a file.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::clock::MonotonicClock;
use crate::impair::{percent_text, percent_to_ppm, ImpairConfig};
use crate::kernel::{KernelConfig, KernelStack};
use crate::lossy::LossyOwn;
use crate::netns::{self, NetnsError};
use crate::own::{OwnConfig, OwnStack, Pools};
use crate::report::BenchReport;
use crate::smol::{SmolConfig, SmolStack, TimeWait};
use crate::stack::TcpStack;
use crate::sysinfo::{self, HostInfo};
use crate::workload::{
    loss_transfer, session_rate, socket_scaling, throughput, BenchError, Direction, LossConfig,
    LossLevel, LossReport, LossRun, RateConfig, RateReport, RateRun, ScaleConfig, ThroughputConfig,
    ThroughputReport, ThroughputRun,
};

/// The text that `--help` prints.
pub const USAGE: &str = "\
usage: tcp_bench [--stack kernel|own|smoltcp] [--workload rate|throughput|scale|loss|all] [--profile small|full]
                 [--label TEXT] [--out PATH|-]
  rate:        [--duration-secs N] [--concurrency N] [--threads N]
               [--request-bytes N] [--response-bytes N]
  throughput:  [--transfer-sessions N] [--transfer-mib N] [--chunk-kib N]
               [--direction upload|download] [--threads N]
  scale:       [--scale-steps N,N,...] [--shards N] [--scale-buffer-bytes N]
               [--step-seconds N] [--probe-sessions N] [--batch N]
  loss:        [--loss-profile fat] [--loss-levels PCT,PCT,...] [--loss-transfers N] [--loss-kib N]
               [--loss-timeout-secs N] [--link-delay-ms N] [--link-reorder PCT]
               [--link-rate-mbit N] [--link-queue-kib N] [--link-seed N]
  kernel:      [--nodelay on|off] [--buffer-kib N] [--backlog N]
  own:         [--own-mtu N] [--own-window-kib N] [--own-time-wait-ms N] [--own-rto-ms N]
               [--own-pool N] [--own-server-hosts N] [--own-vlan ID]
               [--own-wscale on|off] [--own-timestamps on|off] [--own-sack on|off]
               [--own-delayed-ack on|off]
  smoltcp:     [--smol-mtu N] [--smol-checksums on|off] [--smol-buffer-kib N]
               [--smol-time-wait skip|keep] [--smol-idle-poll-ms N]
The stack kernel is the TCP of the operating system on the loopback interface, and it
is the reference that other stacks are compared with. The stack own is the minimal TCP
stack of TRN-18 over an in memory Ethernet link, see the module fps_tcp::own. With
--own-pool N its client side is a pool of N hosts (N addresses, N MAC addresses), with
--own-server-hosts the server side is a pool too, and --own-vlan tags the link. The stack
smoltcp is the smoltcp crate behind the same seam (TRN-20), over an in memory IP link,
see the module fps_tcp::smol. Without --buffer-kib the kernel
sizes the socket buffers itself.
The workload loss (TRN-22) is not part of all. It moves one transfer per run over a link
with random loss (each --loss-levels value, default 0,0.1,1), a delay of --link-delay-ms
(default 20) in each direction, optional reordering (a share of frames skips the delay),
and an optional rate limit with a bounded queue, and it reports goodput, retransmissions,
and completion per transfer. The own and smoltcp stacks run over the in memory link with
those settings. The kernel stack runs in a client and a server network namespace with a
router namespace between them, with tc qdiscs (tbf for the rate, netem for the rest) on both
router interfaces, which needs root: without it
tcp_bench runs itself again through sudo -n, and it exits with 3 when that is not possible
(1 when CI=true or FPS_REQUIRE_NETEM=1), and also with 3 when the kernel has no sch_netem
module. The link flags apply to the loss workload only.
--loss-profile fat (TRN-24) sets the long fat path of its acceptance: 0.1 percent loss, 50 ms of
delay in each direction (a round trip of 100 ms), 50 Mbit/s with a queue of 1 MiB, so that the
bandwidth delay product of 625 KB is far above 64 KB, and 8 MiB per transfer with a timeout of
180 s. Flags after it override its values. Without it the loss workload keeps its defaults.
The profile small finishes within a minute and fits a hosted CI runner. The profile
full measures up to 1,000,000 sockets and is meant for a host with 16 GiB of memory.
Every flag after the profile overrides the profile. Sockets count both hosts of a pair.
The option --out - prints the fps-bench/v1 JSON document after the report.
";

/// Which workloads a run executes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Selection {
    Rate,
    Throughput,
    Scale,
    Loss,
    All,
}

impl Selection {
    fn rate(self) -> bool {
        matches!(self, Selection::Rate | Selection::All)
    }
    fn throughput(self) -> bool {
        matches!(self, Selection::Throughput | Selection::All)
    }
    fn scale(self) -> bool {
        matches!(self, Selection::Scale | Selection::All)
    }
    fn loss(self) -> bool {
        self == Selection::Loss
    }
}

/// The settings of one run after the profile and the flags were applied.
#[derive(Clone, Debug)]
struct Options {
    stack: String,
    selection: Selection,
    profile: String,
    label: String,
    out: Option<PathBuf>,
    rate: RateConfig,
    rate_threads: usize,
    transfer: ThroughputConfig,
    transfer_threads: usize,
    scale: ScaleConfig,
    shards: usize,
    scale_buffer: Option<usize>,
    kernel: KernelConfig,
    own: OwnConfig,
    /// The client hosts, the server hosts, and the VLAN of the own stack.
    own_pools: (u32, u32, Option<u16>),
    smol: SmolConfig,
    loss: LossConfig,
    /// The loss levels of the loss workload, in parts per million.
    loss_levels: Vec<u32>,
    loss_transfers: usize,
    /// The named preset of the loss workload, when one was given.
    loss_profile: Option<String>,
    /// The link of the loss workload. Its `loss_ppm` is set per level.
    link: ImpairConfig,
}

/// A mistake in the command line.
#[derive(Debug, thiserror::Error)]
enum UsageError {
    #[error("unknown flag {0}")]
    UnknownFlag(String),
    #[error("flag {0} needs a value")]
    MissingValue(String),
    #[error("flag {flag} has the invalid value {value}")]
    BadValue { flag: String, value: String },
    #[error("unknown stack {0}, the stacks are: kernel, own, smoltcp")]
    UnknownStack(String),
    #[error("the link settings are invalid: {0}")]
    BadLink(String),
}

fn profile_defaults(profile: &str) -> Option<Options> {
    let kernel = KernelConfig::default();
    let mut options = Options {
        stack: "kernel".into(),
        selection: Selection::All,
        profile: profile.into(),
        label: "local".into(),
        out: None,
        rate: RateConfig::default(),
        rate_threads: 1,
        transfer: ThroughputConfig::default(),
        transfer_threads: 1,
        scale: ScaleConfig::default(),
        shards: 1,
        scale_buffer: None,
        kernel,
        own: OwnConfig::default(),
        own_pools: (1, 1, None),
        smol: SmolConfig::default(),
        loss: LossConfig::default(),
        loss_levels: vec![0, 1_000, 10_000],
        loss_transfers: 3,
        loss_profile: None,
        link: ImpairConfig {
            delay_us: 20_000,
            ..ImpairConfig::default()
        },
    };
    match profile {
        "small" => {
            options.rate.duration = Duration::from_secs(2);
            options.rate.concurrency = 32;
            options.transfer.sessions = 4;
            options.transfer.bytes_per_session = 64 * 1024 * 1024;
            options.scale.steps = vec![1_000, 4_000, 16_000];
            options.scale.step_budget = Duration::from_secs(30);
            options.scale.probe_sessions = 100;
        }
        "full" => {
            options.rate.duration = Duration::from_secs(10);
            options.rate.concurrency = 64;
            options.transfer.sessions = 8;
            options.transfer.bytes_per_session = 512 * 1024 * 1024;
            options.scale.steps =
                vec![1_000, 10_000, 100_000, 250_000, 500_000, 750_000, 1_000_000];
            options.scale.step_budget = Duration::from_secs(300);
            options.scale.probe_sessions = 500;
            options.shards = 64;
        }
        _ => return None,
    }
    Some(options)
}

/// Splits `--flag value` and `--flag=value` into pairs.
fn split_flags(args: &[String]) -> Result<Vec<(String, String)>, UsageError> {
    let mut pairs = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if !arg.starts_with("--") {
            return Err(UsageError::UnknownFlag(arg.clone()));
        }
        if let Some((flag, value)) = arg.split_once('=') {
            pairs.push((flag.to_string(), value.to_string()));
        } else {
            let value = iter
                .next()
                .ok_or_else(|| UsageError::MissingValue(arg.clone()))?;
            pairs.push((arg.clone(), value.clone()));
        }
    }
    Ok(pairs)
}

fn number<T: std::str::FromStr>(flag: &str, value: &str) -> Result<T, UsageError> {
    value.parse().map_err(|_| UsageError::BadValue {
        flag: flag.into(),
        value: value.into(),
    })
}

fn on_off(flag: &str, value: &str) -> Result<bool, UsageError> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(bad(flag, value)),
    }
}

fn bad(flag: &str, value: &str) -> UsageError {
    UsageError::BadValue {
        flag: flag.into(),
        value: value.into(),
    }
}

fn parse(args: &[String]) -> Result<Options, UsageError> {
    let pairs = split_flags(args)?;
    let profile = pairs
        .iter()
        .find(|(flag, _)| flag == "--profile")
        .map_or("small", |(_, value)| value.as_str());
    let mut o = profile_defaults(profile).ok_or_else(|| bad("--profile", profile))?;
    for (flag, value) in &pairs {
        let (f, v) = (flag.as_str(), value.as_str());
        match f {
            "--profile" => {}
            "--stack" => o.stack = v.into(),
            "--workload" => {
                o.selection = match v {
                    "rate" => Selection::Rate,
                    "throughput" => Selection::Throughput,
                    "scale" => Selection::Scale,
                    "loss" => Selection::Loss,
                    "all" => Selection::All,
                    _ => return Err(bad(f, v)),
                }
            }
            "--label" => o.label = v.into(),
            "--out" => o.out = Some(PathBuf::from(v)),
            "--duration-secs" => {
                let secs: f64 = number(f, v)?;
                if !(secs.is_finite() && secs > 0.0) {
                    return Err(bad(f, v));
                }
                o.rate.duration = Duration::from_secs_f64(secs);
            }
            "--concurrency" => o.rate.concurrency = number(f, v)?,
            "--threads" => {
                let threads: usize = number(f, v)?;
                o.rate_threads = threads.max(1);
                o.transfer_threads = threads.max(1);
            }
            "--request-bytes" => o.rate.request_bytes = number(f, v)?,
            "--response-bytes" => o.rate.response_bytes = number(f, v)?,
            "--transfer-sessions" => o.transfer.sessions = number(f, v)?,
            "--transfer-mib" => {
                let mib: u64 = number(f, v)?;
                o.transfer.bytes_per_session = mib.saturating_mul(1024 * 1024);
            }
            "--chunk-kib" => {
                let kib: usize = number(f, v)?;
                o.transfer.chunk = kib.saturating_mul(1024).max(1);
            }
            "--direction" => {
                o.transfer.direction = match v {
                    "upload" => Direction::Upload,
                    "download" => Direction::Download,
                    _ => return Err(bad(f, v)),
                }
            }
            "--scale-steps" => {
                let steps: Result<Vec<usize>, _> = v.split(',').map(|s| number(f, s)).collect();
                o.scale.steps = steps?;
            }
            "--shards" => o.shards = number::<usize>(f, v)?.max(1),
            "--scale-buffer-bytes" => o.scale_buffer = Some(number(f, v)?),
            "--step-seconds" => o.scale.step_budget = Duration::from_secs(number(f, v)?),
            "--probe-sessions" => o.scale.probe_sessions = number(f, v)?,
            "--batch" => o.scale.batch = number(f, v)?,
            "--nodelay" => {
                o.kernel.nodelay = match v {
                    "on" => true,
                    "off" => false,
                    _ => return Err(bad(f, v)),
                }
            }
            "--buffer-kib" => {
                let kib: usize = number(f, v)?;
                o.kernel.buffer_bytes = Some(kib.saturating_mul(1024));
            }
            "--backlog" => {
                o.kernel.backlog = number(f, v)?;
                o.own.backlog = o.kernel.backlog as usize;
            }
            "--own-mtu" => o.own.mtu = number::<usize>(f, v)?.clamp(128, 65_535),
            "--own-window-kib" => {
                let kib: usize = number(f, v)?;
                o.own.rcv_buffer = kib.saturating_mul(1024).clamp(1, 1 << 30);
            }
            "--own-time-wait-ms" => {
                let ms: i64 = number(f, v)?;
                o.own.time_wait_us = ms.saturating_mul(1000);
            }
            "--own-rto-ms" => {
                let ms: i64 = number(f, v)?;
                o.own.rto_us = ms.saturating_mul(1000).max(1000);
            }
            "--own-pool" => o.own_pools.0 = number::<u32>(f, v)?.max(1),
            "--own-server-hosts" => o.own_pools.1 = number::<u32>(f, v)?.max(1),
            "--own-vlan" => o.own_pools.2 = Some(number(f, v)?),
            "--loss-profile" => {
                if v != "fat" {
                    return Err(bad(f, v));
                }
                o.loss_levels = vec![1_000];
                o.link.delay_us = 50_000;
                o.link.rate_bps = 50_000_000;
                o.link.queue_bytes = 1024 * 1024;
                o.loss.bytes = 8 * 1024 * 1024;
                o.loss.timeout = Duration::from_secs(180);
                o.loss_profile = Some(v.to_string());
            }
            "--own-wscale" => o.own.window_scale = on_off(f, v)?,
            "--own-timestamps" => o.own.timestamps = on_off(f, v)?,
            "--own-sack" => o.own.sack = on_off(f, v)?,
            "--own-delayed-ack" => o.own.delayed_ack = on_off(f, v)?,
            "--loss-levels" => {
                let levels: Result<Vec<u32>, _> = v
                    .split(',')
                    .map(|s| percent_to_ppm(s).map_err(|_| bad(f, v)))
                    .collect();
                o.loss_levels = levels?;
                if o.loss_levels.is_empty() {
                    return Err(bad(f, v));
                }
            }
            "--loss-transfers" => o.loss_transfers = number::<usize>(f, v)?.max(1),
            "--loss-kib" => {
                let kib: u64 = number(f, v)?;
                o.loss.bytes = kib.saturating_mul(1024).max(1);
            }
            "--loss-timeout-secs" => {
                o.loss.timeout = Duration::from_secs(number::<u64>(f, v)?.max(1));
            }
            "--link-delay-ms" => {
                let ms: f64 = number(f, v)?;
                if !(ms.is_finite() && ms >= 0.0) {
                    return Err(bad(f, v));
                }
                o.link.delay_us = (ms * 1000.0).round() as i64;
            }
            "--link-reorder" => o.link.reorder_ppm = percent_to_ppm(v).map_err(|_| bad(f, v))?,
            "--link-rate-mbit" => {
                let mbit: f64 = number(f, v)?;
                if !(mbit.is_finite() && mbit >= 0.0) {
                    return Err(bad(f, v));
                }
                o.link.rate_bps = (mbit * 1e6).round() as u64;
            }
            "--link-queue-kib" => {
                let kib: usize = number(f, v)?;
                o.link.queue_bytes = kib.saturating_mul(1024);
            }
            "--link-seed" => o.link.seed = number(f, v)?,
            "--smol-mtu" => o.smol.link.mtu = number(f, v)?,
            "--smol-checksums" => {
                o.smol.link.checksums = match v {
                    "on" => true,
                    "off" => false,
                    _ => return Err(bad(f, v)),
                }
            }
            "--smol-buffer-kib" => {
                let kib: usize = number(f, v)?;
                o.smol.rx_buffer = kib.saturating_mul(1024);
                o.smol.tx_buffer = kib.saturating_mul(1024);
            }
            "--smol-time-wait" => {
                o.smol.time_wait = match v {
                    "skip" => TimeWait::Skip,
                    "keep" => TimeWait::Keep,
                    _ => return Err(bad(f, v)),
                }
            }
            "--smol-idle-poll-ms" => {
                let millis: i64 = number(f, v)?;
                o.smol.idle_poll_micros = millis.saturating_mul(1000);
            }
            _ => return Err(UsageError::UnknownFlag(flag.clone())),
        }
    }
    if !["kernel", "own", "smoltcp"].contains(&o.stack.as_str()) {
        return Err(UsageError::UnknownStack(o.stack));
    }
    if let Err(reason) = o.link.validate() {
        return Err(UsageError::BadLink(reason));
    }
    Ok(o)
}

/// Runs `body` on a current thread runtime with timers and the I/O reactor.
fn on_runtime<F: std::future::Future>(body: F) -> Result<F::Output, BenchError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|source| BenchError::Io {
            what: "build runtime".into(),
            source,
        })?;
    Ok(runtime.block_on(body))
}

/// Runs `work` once per thread, all started together, and collects the results.
fn on_threads<T, F>(threads: usize, work: F) -> Result<Vec<T>, BenchError>
where
    T: Send,
    F: Fn(usize, &Barrier) -> Result<T, BenchError> + Sync,
{
    let barrier = Barrier::new(threads);
    thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|index| {
                let (barrier, work) = (&barrier, &work);
                scope.spawn(move || work(index, barrier))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| BenchError::Task("a bench thread panicked".into()))?
            })
            .collect()
    })
}

fn buffer_text(bytes: Option<usize>) -> String {
    bytes.map_or_else(|| "kernel default".to_string(), |b| b.to_string())
}

fn describe(o: &Options) -> Vec<(String, String)> {
    let mut config = vec![("profile".to_string(), o.profile.clone())];
    if o.stack == "own" {
        config.push(("own.mtu".into(), o.own.mtu.to_string()));
        config.push(("own.rcv_buffer".into(), o.own.rcv_buffer.to_string()));
        config.push(("own.snd_buffer".into(), o.own.snd_buffer.to_string()));
        config.push(("own.rto_us".into(), o.own.rto_us.to_string()));
        config.push(("own.time_wait_us".into(), o.own.time_wait_us.to_string()));
        config.push(("own.backlog".into(), o.own.backlog.to_string()));
        config.push(("own.link_depth".into(), o.own.link_depth.to_string()));
        config.push(("own.window_scale".into(), o.own.window_scale.to_string()));
        config.push(("own.timestamps".into(), o.own.timestamps.to_string()));
        config.push(("own.sack".into(), o.own.sack.to_string()));
        config.push(("own.delayed_ack".into(), o.own.delayed_ack.to_string()));
        config.push(("own.client_hosts".into(), o.own_pools.0.to_string()));
        config.push(("own.server_hosts".into(), o.own_pools.1.to_string()));
        config.push((
            "own.vlan".into(),
            o.own_pools.2.map_or("none".to_string(), |v| v.to_string()),
        ));
    } else if o.stack == "smoltcp" {
        config.push(("smol.mtu".into(), o.smol.link.mtu.to_string()));
        config.push(("smol.checksums".into(), o.smol.link.checksums.to_string()));
        config.push(("smol.time_wait".into(), format!("{:?}", o.smol.time_wait)));
        config.push(("smol.nagle".into(), o.smol.nagle.to_string()));
        config.push((
            "smol.idle_poll_ms".into(),
            (o.smol.idle_poll_micros / 1000).to_string(),
        ));
        config.push(("smol.backlog".into(), o.smol.backlog.to_string()));
    } else {
        config.push(("nodelay".into(), o.kernel.nodelay.to_string()));
        config.push(("backlog".into(), o.kernel.backlog.to_string()));
    }
    if o.selection.loss() {
        let levels: Vec<String> = o.loss_levels.iter().map(|p| percent_text(*p)).collect();
        config.push(("loss.levels_percent".into(), levels.join(",")));
        if let Some(profile) = &o.loss_profile {
            config.push(("loss.profile".into(), profile.clone()));
        }
        config.push(("loss.transfers".into(), o.loss_transfers.to_string()));
        config.push(("loss.bytes".into(), o.loss.bytes.to_string()));
        config.push(("loss.chunk".into(), o.loss.chunk.to_string()));
        config.push((
            "loss.timeout_secs".into(),
            o.loss.timeout.as_secs().to_string(),
        ));
        config.push(("link.delay_us".into(), o.link.delay_us.to_string()));
        config.push((
            "link.reorder_percent".into(),
            percent_text(o.link.reorder_ppm),
        ));
        config.push(("link.rate_bps".into(), o.link.rate_bps.to_string()));
        config.push(("link.queue_bytes".into(), o.link.queue_bytes.to_string()));
        config.push(("link.seed".into(), o.link.seed.to_string()));
        config.push(("link.mtu".into(), link_mtu(o).to_string()));
        if o.stack == "kernel" {
            config.push((
                "kernel.buffer_bytes".into(),
                buffer_text(o.kernel.buffer_bytes),
            ));
        }
    }
    if o.selection.rate() {
        config.push((
            "rate.duration_secs".into(),
            o.rate.duration.as_secs_f64().to_string(),
        ));
        config.push(("rate.concurrency".into(), o.rate.concurrency.to_string()));
        config.push(("rate.threads".into(), o.rate_threads.to_string()));
        config.push((
            "rate.request_bytes".into(),
            o.rate.request_bytes.to_string(),
        ));
        config.push((
            "rate.response_bytes".into(),
            o.rate.response_bytes.to_string(),
        ));
        if o.stack == "smoltcp" {
            config.push(("rate.buffer_bytes".into(), o.smol.rx_buffer.to_string()));
        } else if o.stack != "own" {
            config.push((
                "rate.buffer_bytes".into(),
                buffer_text(o.kernel.buffer_bytes),
            ));
        }
    }
    if o.selection.throughput() {
        config.push((
            "throughput.sessions".into(),
            o.transfer.sessions.to_string(),
        ));
        config.push((
            "throughput.bytes_per_session".into(),
            o.transfer.bytes_per_session.to_string(),
        ));
        config.push(("throughput.chunk".into(), o.transfer.chunk.to_string()));
        config.push((
            "throughput.direction".into(),
            format!("{:?}", o.transfer.direction),
        ));
        config.push(("throughput.threads".into(), o.transfer_threads.to_string()));
        if o.stack == "smoltcp" {
            config.push((
                "throughput.buffer_bytes".into(),
                o.smol.rx_buffer.to_string(),
            ));
        } else if o.stack != "own" {
            config.push((
                "throughput.buffer_bytes".into(),
                buffer_text(o.kernel.buffer_bytes),
            ));
        }
    }
    if o.selection.scale() {
        let steps: Vec<String> = o.scale.steps.iter().map(usize::to_string).collect();
        config.push(("scale.steps".into(), steps.join(",")));
        config.push(("scale.shards".into(), o.shards.to_string()));
        config.push((
            "open_files_soft_limit".into(),
            sysinfo::open_files_limit().map_or_else(|| "unlimited".to_string(), |n| n.to_string()),
        ));
        if o.stack == "smoltcp" {
            config.push((
                "scale.buffer_bytes".into(),
                o.scale_buffer.unwrap_or(SMOL_SCALE_BUFFER).to_string(),
            ));
        } else if o.stack != "own" {
            config.push((
                "scale.buffer_bytes".into(),
                buffer_text(o.scale_buffer.or(o.kernel.buffer_bytes)),
            ));
        }
        config.push((
            "scale.step_budget_secs".into(),
            o.scale.step_budget.as_secs().to_string(),
        ));
        config.push((
            "scale.probe_sessions".into(),
            o.scale.probe_sessions.to_string(),
        ));
    }
    config
}

/// Builds shard `index` of a stack, for the scaling workload when `scale` is
/// true.
type MakeStack<'a, S> = &'a (dyn Fn(usize, bool) -> S + Sync);

/// The socket buffer in bytes that a smoltcp socket gets in the scaling
/// workload, unless `--scale-buffer-bytes` says otherwise. The held sockets
/// are idle, so a small buffer shows the cost of the socket itself.
const SMOL_SCALE_BUFFER: usize = 512;

fn make_smol(options: &Options, index: usize, scale: bool) -> SmolStack {
    let mut config = options.smol.clone();
    config.seed = config.seed.wrapping_add(index as u64 * 2);
    if scale {
        let buffer = options.scale_buffer.unwrap_or(SMOL_SCALE_BUFFER);
        config.rx_buffer = buffer;
        config.tx_buffer = buffer;
    }
    SmolStack::new(config, Arc::new(MonotonicClock::new()))
}

fn make_kernel(options: &Options, scale: bool) -> KernelStack {
    let mut kernel = options.kernel.clone();
    if scale && options.scale_buffer.is_some() {
        kernel.buffer_bytes = options.scale_buffer;
    }
    KernelStack::new(kernel)
}

/// The IP MTU of the loss link of the chosen stack.
fn link_mtu(o: &Options) -> usize {
    match o.stack.as_str() {
        "own" => o.own.mtu,
        "smoltcp" => o.smol.link.mtu,
        _ => 1500,
    }
}

/// Why the loss workload did not produce a report.
enum LossError {
    /// The run cannot happen here (no netem), which is a skip unless the
    /// environment requires it.
    Skip(String),
    Bench(BenchError),
}

fn finish_report(options: &Options, key: &str, name: String, notes: Vec<String>) -> BenchReport {
    BenchReport {
        stack_key: key.into(),
        stack_name: name,
        stack_notes: notes,
        label: options.label.clone(),
        host: HostInfo::detect(),
        config: describe(options),
        rate: None,
        throughput: None,
        scale: None,
        loss: None,
        finished_unix_secs: 0,
    }
}

/// Runs the transfers of every loss level. `transfer` runs transfer `run` of
/// a level on a link with the loss `link`.
fn loss_levels(
    options: &Options,
    mut transfer: impl FnMut(&ImpairConfig, usize) -> Result<LossRun, LossError>,
) -> Result<Vec<LossLevel>, LossError> {
    let mut levels = Vec::new();
    for (level_index, ppm) in options.loss_levels.iter().enumerate() {
        let mut runs = Vec::new();
        for run in 0..options.loss_transfers {
            let mut link = options.link;
            link.loss_ppm = *ppm;
            // Every transfer has its own random sequence.
            link.seed = link
                .seed
                .wrapping_add((level_index * options.loss_transfers + run) as u64 * 7919);
            runs.push(transfer(&link, run)?);
        }
        levels.push(LossLevel {
            loss_ppm: *ppm,
            runs,
        });
    }
    Ok(levels)
}

fn invalid_link(reason: String) -> LossError {
    LossError::Bench(BenchError::Stalled(format!(
        "the link is invalid: {reason}"
    )))
}

/// Measures the loss workload on the stack that `options` names.
fn loss_report(options: &Options) -> Result<BenchReport, LossError> {
    let mut report;
    let levels = match options.stack.as_str() {
        "own" => {
            let make = |link: &ImpairConfig| {
                LossyOwn::new(options.own.clone(), *link, Arc::new(MonotonicClock::new()))
                    .map_err(invalid_link)
            };
            let probe = make(&options.link)?;
            report = finish_report(options, "own", probe.name(), probe.notes());
            loss_levels(options, |link, _| {
                let stack = make(link)?;
                on_runtime(loss_transfer(&stack, &options.loss)).map_err(LossError::Bench)
            })?
        }
        "smoltcp" => {
            let make = |link: &ImpairConfig, run: usize| {
                let mut config = options.smol.clone();
                config.impair = Some(*link);
                // A frame that falls due must reach its host without a session
                // acting, so the idle interval is short.
                config.idle_poll_micros = config.idle_poll_micros.min(1000);
                config.seed = config.seed.wrapping_add(run as u64 * 2);
                SmolStack::new(config, Arc::new(MonotonicClock::new()))
            };
            let probe = make(&options.link, 0);
            report = finish_report(options, "smoltcp", probe.name(), probe.notes());
            loss_levels(options, |link, run| {
                let stack = make(link, run);
                on_runtime(loss_transfer(&stack, &options.loss)).map_err(LossError::Bench)
            })?
        }
        _ => {
            let exe = std::env::current_exe().map_err(|e| {
                LossError::Bench(BenchError::Io {
                    what: "locate the tcp_bench binary".into(),
                    source: e,
                })
            })?;
            report = finish_report(
                options,
                "kernel",
                "kernel TCP over a router in network namespaces".to_string(),
                kernel_notes(options),
            );
            let mut count = 0u32;
            loss_levels(options, |link, _| {
                count += 1;
                let prefix = format!("fpstcp{}x{count}", std::process::id() % 100_000);
                let buffer = options.kernel.buffer_bytes;
                netns::kernel_transfer(&exe, &prefix, link, &options.loss, link_mtu(options), buffer)
                    .map_err(|error| match error {
                        NetnsError::NetemMissing(detail) => LossError::Skip(format!(
                            "the kernel has no netem qdisc (module sch_netem), so the kernel stack cannot be measured on a lossy link here: {detail}"
                        )),
                        other => LossError::Bench(BenchError::Stalled(other.to_string())),
                    })
            })?
        }
    };
    report.loss = Some(LossReport {
        link: ImpairConfig {
            loss_ppm: 0,
            ..options.link
        }
        .describe(),
        bytes_per_transfer: options.loss.bytes,
        levels,
    });
    report.finished_unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Ok(report)
}

fn kernel_notes(options: &Options) -> Vec<String> {
    let congestion = std::fs::read_to_string("/proc/sys/net/ipv4/tcp_congestion_control")
        .map_or_else(|_| "unknown".to_string(), |t| t.trim().to_string());
    vec![
        format!(
            "the client and the server are processes of this binary in two network namespaces, with a router namespace between them and a veth pair on each side (mtu {}); each interface of the router has a tbf qdisc for the rate (when set) and a netem qdisc below it for delay, loss, and reordering, so both directions have the link settings and a drop happens in the middle, as in a network; offloads on the veth ends are turned off",
            link_mtu(options)
        ),
        format!(
            "the kernel sizes the socket buffers itself unless --buffer-kib is set, and its congestion control is {congestion}, so its window is far above the 64 KiB window of the own stack and of the smoltcp default"
        ),
        "retransmissions are the growth of RetransSegs in the client namespace during the transfer, which counts segments that the kernel sent again; the link drop counters are not read for the kernel".to_string(),
        "every transfer has a fresh pair of namespaces, so the metrics cache of the kernel does not carry the slow start threshold of one transfer into the next".to_string(),
        "the tbf token bucket holds 5 ms of the rate, which the in memory link does not have, so a kernel burst can pass the bottleneck that the in memory link serializes".to_string(),
    ]
}

fn measure<S: TcpStack>(
    options: &Options,
    key: &str,
    make: MakeStack<'_, S>,
) -> Result<BenchReport, BenchError> {
    let mut report = BenchReport {
        stack_key: key.into(),
        stack_name: make(0, false).name(),
        stack_notes: make(0, false).notes(),
        label: options.label.clone(),
        host: HostInfo::detect(),
        config: describe(options),
        rate: None,
        throughput: None,
        scale: None,
        loss: None,
        finished_unix_secs: 0,
    };
    if options.selection.rate() {
        let runs: Vec<RateRun> = on_threads(options.rate_threads, |index, barrier| {
            let stack = make(index, false);
            barrier.wait();
            on_runtime(session_rate(&stack, &options.rate))?
        })?;
        report.rate = Some(RateReport::from_runs(runs));
    }
    if options.selection.throughput() {
        let runs: Vec<ThroughputRun> = on_threads(options.transfer_threads, |index, barrier| {
            let stack = make(index, false);
            barrier.wait();
            on_runtime(throughput(&stack, &options.transfer))?
        })?;
        report.throughput = Some(ThroughputReport::from_runs(&runs));
    }
    if options.selection.scale() {
        let stacks: Vec<S> = (0..options.shards).map(|i| make(i, true)).collect();
        report.scale = Some(on_runtime(socket_scaling(&stacks, &options.scale))??);
    }
    report.finished_unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Ok(report)
}

/// Runs the bench command with `args`, which exclude the program name, and
/// returns the process exit code.
pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        let _ = out.write_all(USAGE.as_bytes());
        return 0;
    }
    if args.first().map(String::as_str) == Some(netns::PEER_FLAG) {
        return netns::peer_main(args.get(1..).unwrap_or_default());
    }
    let options = match parse(args) {
        Ok(options) => options,
        Err(error) => {
            let _ = writeln!(err, "tcp_bench: {error}\n{USAGE}");
            return 2;
        }
    };
    if options.selection.loss() {
        if options.stack == "kernel" && !netns::is_root() {
            return rerun_as_root(args, err);
        }
        return match loss_report(&options) {
            Ok(report) => emit(&options, &report, out, err),
            Err(LossError::Skip(why)) => {
                let _ = writeln!(err, "tcp_bench: {why}");
                skip_code()
            }
            Err(LossError::Bench(error)) => {
                let _ = writeln!(err, "tcp_bench: {error}");
                1
            }
        };
    }
    let measured = if options.stack == "own" {
        let (clients, servers, vlan) = options.own_pools;
        let pools = match Pools::counts(clients, servers, vlan) {
            Ok(pools) => pools,
            Err(error) => {
                let _ = writeln!(err, "tcp_bench: the own host pools are invalid: {error}");
                return 2;
            }
        };
        if let Err(error) = pools.stations() {
            let _ = writeln!(err, "tcp_bench: the own host pools are invalid: {error}");
            return 2;
        }
        measure(&options, "own", &|_, _| {
            // The pools were built above, so this does not fail.
            OwnStack::with_pools(
                options.own.clone(),
                pools.clone(),
                Arc::new(MonotonicClock::new()),
            )
            .unwrap_or_else(|_| OwnStack::new(options.own.clone()))
        })
    } else if options.stack == "smoltcp" {
        measure(&options, "smoltcp", &|index, scale| {
            make_smol(&options, index, scale)
        })
    } else {
        measure(&options, "kernel", &|_, scale| make_kernel(&options, scale))
    };
    let report = match measured {
        Ok(report) => report,
        Err(error) => {
            let _ = writeln!(err, "tcp_bench: {error}");
            if let Some(source) = std::error::Error::source(&error) {
                let _ = writeln!(err, "  caused by: {source}");
            }
            return 1;
        }
    };
    emit(&options, &report, out, err)
}

/// Whether a run that cannot happen here must fail: `CI=true` or
/// `FPS_REQUIRE_NETEM=1`.
fn skip_is_failure() -> bool {
    std::env::var("CI").is_ok_and(|v| v == "true")
        || std::env::var("FPS_REQUIRE_NETEM").is_ok_and(|v| v == "1")
}

/// The exit code of a run that could not happen: 3, or 1 where a skip fails.
fn skip_code() -> i32 {
    if skip_is_failure() {
        1
    } else {
        3
    }
}

/// Runs this command again as root through `sudo -n`, because the kernel loss
/// workload needs network namespaces. The child prints the report.
fn rerun_as_root(args: &[String], err: &mut dyn Write) -> i32 {
    let runnable = std::env::var_os("FPS_TCP_BENCH_SUDO").is_none()
        && std::process::Command::new("sudo")
            .args(["-n", "true"])
            .status()
            .is_ok_and(|s| s.success());
    let exe = std::env::current_exe();
    let Some(exe) = exe.ok().filter(|_| runnable) else {
        let _ = writeln!(
            err,
            "tcp_bench: the kernel loss workload needs root for network namespaces and tc, and sudo -n is not available, so it did not run"
        );
        return skip_code();
    };
    match std::process::Command::new("sudo")
        .args(["-n", "env", "FPS_TCP_BENCH_SUDO=1"])
        .arg(exe)
        .args(args)
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            let _ = writeln!(err, "tcp_bench: sudo: {error}");
            1
        }
    }
}

/// Prints the report and writes the document that `--out` names.
fn emit(options: &Options, report: &BenchReport, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let _ = out.write_all(report.to_text().as_bytes());
    if let Some(path) = &options.out {
        if path.as_os_str() == "-" {
            let _ = out.write_all(report.to_json().as_bytes());
        } else if let Err(error) = std::fs::write(path, report.to_json()) {
            let _ = writeln!(err, "tcp_bench: write {}: {error}", path.display());
            return 1;
        } else {
            let _ = writeln!(out, "wrote {}", path.display());
        }
    }
    let failed = report.rate.as_ref().is_some_and(|r| r.errors > 0);
    i32::from(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_split_on_space_and_equals() {
        let pairs = split_flags(&args(&["--a", "1", "--b=2"])).expect("split");
        assert_eq!(
            pairs,
            vec![
                ("--a".to_string(), "1".to_string()),
                ("--b".to_string(), "2".to_string())
            ]
        );
    }

    #[test]
    fn defaults_follow_the_small_profile() {
        let o = parse(&args(&[])).expect("parse");
        assert_eq!(o.profile, "small");
        assert_eq!(o.selection, Selection::All);
        assert_eq!(o.scale.steps, vec![1_000, 4_000, 16_000]);
    }

    #[test]
    fn full_profile_reaches_one_million_sockets_and_flags_override_it() {
        let o = parse(&args(&["--profile", "full", "--shards", "8"])).expect("parse");
        assert_eq!(o.scale.steps.last(), Some(&1_000_000));
        assert_eq!(o.shards, 8);
    }

    #[test]
    fn flag_order_does_not_change_the_profile() {
        let o = parse(&args(&["--shards", "8", "--profile", "full"])).expect("parse");
        assert_eq!(o.shards, 8);
        assert_eq!(o.rate.duration, Duration::from_secs(10));
    }

    #[test]
    fn bad_input_is_reported() {
        assert!(matches!(
            parse(&args(&["--workload", "nope"])),
            Err(UsageError::BadValue { .. })
        ));
        assert!(matches!(
            parse(&args(&["--bogus", "1"])),
            Err(UsageError::UnknownFlag(_))
        ));
        assert!(matches!(
            parse(&args(&["--threads"])),
            Err(UsageError::MissingValue(_))
        ));
        assert!(matches!(
            parse(&args(&["--stack", "other"])),
            Err(UsageError::UnknownStack(_))
        ));
        assert!(matches!(
            parse(&args(&["--scale-steps", "10,x"])),
            Err(UsageError::BadValue { .. })
        ));
        assert!(matches!(
            parse(&args(&["--profile", "huge"])),
            Err(UsageError::BadValue { .. })
        ));
    }

    #[test]
    fn stack_flags_reach_the_kernel_config() {
        let o = parse(&args(&[
            "--nodelay=off",
            "--buffer-kib",
            "128",
            "--backlog",
            "512",
            "--scale-buffer-bytes",
            "2048",
        ]))
        .expect("parse");
        assert!(!o.kernel.nodelay);
        assert_eq!(o.kernel.buffer_bytes, Some(128 * 1024));
        assert_eq!(o.kernel.backlog, 512);
        assert_eq!(o.scale_buffer, Some(2048));
        assert_eq!(parse(&args(&[])).expect("parse").kernel.buffer_bytes, None);
    }

    #[test]
    fn smoltcp_flags_reach_its_config() {
        let o = parse(&args(&[
            "--stack",
            "smoltcp",
            "--smol-mtu",
            "9000",
            "--smol-checksums=off",
            "--smol-buffer-kib",
            "128",
            "--smol-time-wait",
            "keep",
            "--smol-idle-poll-ms",
            "5",
        ]))
        .expect("parse");
        assert_eq!(o.stack, "smoltcp");
        assert_eq!(o.smol.idle_poll_micros, 5000);
        assert_eq!(o.smol.link.mtu, 9000);
        assert!(!o.smol.link.checksums);
        assert_eq!(o.smol.rx_buffer, 128 * 1024);
        assert_eq!(o.smol.time_wait, TimeWait::Keep);
    }

    #[test]
    fn a_tiny_smoltcp_run_reports_all_measures() {
        let argv = args(&[
            "--stack",
            "smoltcp",
            "--duration-secs",
            "0.3",
            "--concurrency",
            "4",
            "--transfer-sessions",
            "2",
            "--transfer-mib",
            "2",
            "--scale-steps",
            "200,400",
            "--probe-sessions",
            "10",
            "--out",
            "-",
        ]);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&argv, &mut out, &mut err);
        let text = String::from_utf8_lossy(&out).to_string();
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        assert!(text.contains("smoltcp"), "{text}");
        assert!(text.contains("sessions per second"), "{text}");
        assert!(text.contains("socket scaling"), "{text}");
        assert!(text.contains("smoltcp.scale.400.idle_poll_us"), "{text}");
        assert!(text.contains("smoltcp.rate.sessions_per_s"), "{text}");
    }

    #[test]
    fn flags_of_removed_stacks_are_unknown() {
        for flag in ["--mtu", "--checksums", "--time-wait", "--idle-poll-ms"] {
            assert!(matches!(
                parse(&args(&[flag, "1"])),
                Err(UsageError::UnknownFlag(_))
            ));
        }
    }

    #[test]
    fn a_tiny_run_prints_all_three_measures_and_writes_the_json() {
        let dir = std::env::temp_dir().join(format!("fps-tcp-cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("result.json");
        let argv = args(&[
            "--duration-secs",
            "0.3",
            "--concurrency",
            "4",
            "--transfer-sessions",
            "2",
            "--transfer-mib",
            "2",
            "--scale-steps",
            "200,400",
            "--probe-sessions",
            "10",
            "--label",
            "unit test",
            "--out",
            path.to_str().expect("utf8 path"),
        ]);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&argv, &mut out, &mut err);
        let text = String::from_utf8_lossy(&out).to_string();
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        assert!(text.contains("sessions per second"), "{text}");
        assert!(text.contains("throughput:"), "{text}");
        assert!(text.contains("socket scaling"), "{text}");
        let json = std::fs::read_to_string(&path).expect("json");
        assert!(json.contains("kernel.scale.400.idle_poll_us"));
        assert!(json.contains("kernel.rate.sessions_per_s"));
        assert!(json.contains("loopback interface"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn own_pool_flags_reach_the_options_and_the_report() {
        let o = parse(&args(&[
            "--stack",
            "own",
            "--own-pool",
            "20000",
            "--own-server-hosts",
            "3",
            "--own-vlan",
            "100",
        ]))
        .expect("parse");
        assert_eq!(o.own_pools, (20_000, 3, Some(100)));
        let text = format!("{:?}", describe(&o));
        assert!(text.contains("own.client_hosts"), "{text}");
        assert!(Pools::counts(20_000, 3, Some(100))
            .and_then(|p| p.stations())
            .is_ok());
        assert!(Pools::counts(1, 1, Some(0))
            .and_then(|p| p.stations())
            .is_err());
    }

    #[test]
    fn the_fat_loss_profile_sets_the_long_fat_path_and_later_flags_override_it() {
        let o = parse(&args(&["--workload", "loss", "--loss-profile", "fat"])).expect("parse");
        assert_eq!(o.loss_levels, vec![1_000]);
        assert_eq!(o.link.delay_us, 50_000);
        assert_eq!(o.link.rate_bps, 50_000_000);
        assert_eq!(o.link.queue_bytes, 1024 * 1024);
        assert_eq!(o.loss.bytes, 8 * 1024 * 1024);
        // A bandwidth delay product far above 64 KB: 50 Mbit/s and a round
        // trip of 100 ms.
        let bdp = o.link.rate_bps / 8 * (2 * o.link.delay_us as u64) / 1_000_000;
        assert!(bdp > 64 * 1024, "{bdp}");
        let o = parse(&args(&[
            "--loss-profile",
            "fat",
            "--link-delay-ms",
            "10",
            "--loss-levels",
            "0,1",
        ]))
        .expect("parse");
        assert_eq!(o.link.delay_us, 10_000);
        assert_eq!(o.loss_levels, vec![0, 10_000]);
        assert!(parse(&args(&["--loss-profile", "thin"])).is_err());
        // The default loss run is unchanged.
        let o = parse(&args(&["--workload", "loss"])).expect("parse");
        assert_eq!(o.loss_levels, vec![0, 1_000, 10_000]);
        assert_eq!(o.link.delay_us, 20_000);
        assert_eq!(o.link.rate_bps, 0);
        assert!(o.loss_profile.is_none());
    }

    #[test]
    fn the_own_option_flags_switch_each_option() {
        let o = parse(&args(&[
            "--stack",
            "own",
            "--own-wscale",
            "off",
            "--own-timestamps",
            "off",
            "--own-sack",
            "off",
            "--own-delayed-ack",
            "off",
        ]))
        .expect("parse");
        assert!(!o.own.window_scale && !o.own.timestamps && !o.own.sack && !o.own.delayed_ack);
        assert!(parse(&args(&["--own-sack", "maybe"])).is_err());
        let defaults = parse(&args(&["--stack", "own"])).expect("parse");
        assert!(defaults.own.window_scale && defaults.own.timestamps && defaults.own.sack);
    }

    #[test]
    fn own_stack_flags_reach_its_config_and_a_tiny_run_reports_all_measures() {
        let o = parse(&args(&[
            "--stack",
            "own",
            "--own-mtu",
            "9000",
            "--own-window-kib",
            "32",
            "--own-time-wait-ms",
            "250",
            "--own-rto-ms",
            "50",
        ]))
        .expect("parse");
        assert_eq!(o.stack, "own");
        assert_eq!(o.own.mtu, 9000);
        assert_eq!(o.own.rcv_buffer, 32 * 1024);
        assert_eq!(o.own.time_wait_us, 250_000);
        assert_eq!(o.own.rto_us, 50_000);
        let argv = args(&[
            "--stack",
            "own",
            "--duration-secs",
            "0.3",
            "--concurrency",
            "4",
            "--transfer-sessions",
            "2",
            "--transfer-mib",
            "2",
            "--scale-steps",
            "200,400",
            "--probe-sessions",
            "10",
            "--out",
            "-",
        ]);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&argv, &mut out, &mut err);
        let text = String::from_utf8_lossy(&out).to_string();
        assert_eq!(code, 0, "stderr: {}", String::from_utf8_lossy(&err));
        assert!(text.contains("own minimal TCP"), "{text}");
        assert!(text.contains("sessions per second"), "{text}");
        assert!(text.contains("socket scaling"), "{text}");
        assert!(text.contains("own.scale.400.idle_poll_us"), "{text}");
        assert!(text.contains("own.rate.sessions_per_s"), "{text}");
    }

    #[test]
    fn help_exits_with_zero_and_usage_errors_with_two() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(run(&args(&["--help"]), &mut out, &mut err), 0);
        assert!(String::from_utf8_lossy(&out).contains("usage: tcp_bench"));
        assert_eq!(run(&args(&["--wat", "1"]), &mut out, &mut err), 2);
    }

    #[test]
    fn loss_defaults_are_zero_point_one_and_one_percent_with_20_ms() {
        let o = parse(&args(&["--workload", "loss"])).expect("parse");
        assert!(o.selection.loss() && !o.selection.rate());
        assert_eq!(o.loss_levels, vec![0, 1_000, 10_000]);
        assert_eq!(o.link.delay_us, 20_000);
        assert_eq!(o.link.rate_bps, 0);
        assert_eq!(o.loss_transfers, 3);
        // The workload all does not include loss.
        assert!(!parse(&args(&[])).expect("parse").selection.loss());
    }

    #[test]
    fn link_flags_reach_the_link_settings() {
        let o = parse(&args(&[
            "--workload",
            "loss",
            "--loss-levels",
            "0,2.5",
            "--link-delay-ms",
            "50",
            "--link-reorder",
            "10",
            "--link-rate-mbit",
            "100",
            "--link-queue-kib",
            "128",
            "--link-seed",
            "9",
            "--loss-kib",
            "512",
            "--loss-transfers",
            "5",
            "--loss-timeout-secs",
            "30",
        ]))
        .expect("parse");
        assert_eq!(o.loss_levels, vec![0, 25_000]);
        assert_eq!(o.link.delay_us, 50_000);
        assert_eq!(o.link.reorder_ppm, 100_000);
        assert_eq!(o.link.rate_bps, 100_000_000);
        assert_eq!(o.link.queue_bytes, 128 * 1024);
        assert_eq!(o.link.seed, 9);
        assert_eq!(o.loss.bytes, 512 * 1024);
        assert_eq!(o.loss_transfers, 5);
        assert_eq!(o.loss.timeout, Duration::from_secs(30));
    }

    #[test]
    fn bad_link_input_is_reported() {
        for bad_args in [
            &["--loss-levels", "1,x"][..],
            &["--loss-levels", "101"],
            &["--link-delay-ms", "-1"],
            &["--link-rate-mbit", "nan"],
        ] {
            assert!(
                matches!(parse(&args(bad_args)), Err(UsageError::BadValue { .. })),
                "{bad_args:?}"
            );
        }
        // Reordering without a delay is not a link.
        assert!(matches!(
            parse(&args(&["--link-delay-ms", "0", "--link-reorder", "5"])),
            Err(UsageError::BadLink(_))
        ));
    }

    #[test]
    fn the_peer_flag_runs_a_peer_and_not_the_bench() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        // An unknown role makes the peer fail at once, without a network.
        let code = run(&args(&["--netns-peer", "nobody"]), &mut out, &mut err);
        assert_eq!(code, 1);
        assert!(out.is_empty());
    }
}
