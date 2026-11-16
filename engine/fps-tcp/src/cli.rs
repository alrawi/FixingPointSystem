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
use crate::report::BenchReport;
use crate::smol::{SmolConfig, SmolStack, TimeWait};
use crate::stack::TcpStack;
use crate::sysinfo::HostInfo;
use crate::workload::{
    session_rate, socket_scaling, throughput, BenchError, Direction, RateConfig, RateReport,
    RateRun, ScaleConfig, ThroughputConfig, ThroughputReport, ThroughputRun,
};

/// The text that `--help` prints.
pub const USAGE: &str = "\
usage: tcp_bench [--stack smoltcp] [--workload rate|throughput|scale|all] [--profile small|full]
                 [--label TEXT] [--out PATH|-]
  rate:        [--duration-secs N] [--concurrency N] [--threads N]
               [--request-bytes N] [--response-bytes N]
  throughput:  [--transfer-sessions N] [--transfer-mib N] [--chunk-kib N]
               [--direction upload|download] [--threads N]
  scale:       [--scale-steps N,N,...] [--shards N] [--scale-buffer-bytes N]
               [--step-seconds N] [--probe-sessions N] [--batch N]
  stack:       [--mtu N] [--checksums on|off] [--buffer-kib N] [--time-wait skip|keep]
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
    scale_buffer: usize,
    smol: SmolConfig,
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
    #[error("unknown stack {0}, the stacks are: smoltcp")]
    UnknownStack(String),
}

fn profile_defaults(profile: &str) -> Option<Options> {
    let smol = SmolConfig::default();
    let mut options = Options {
        stack: "smoltcp".into(),
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
        scale_buffer: 512,
        smol,
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
            "--scale-buffer-bytes" => o.scale_buffer = number(f, v)?,
            "--step-seconds" => o.scale.step_budget = Duration::from_secs(number(f, v)?),
            "--probe-sessions" => o.scale.probe_sessions = number(f, v)?,
            "--batch" => o.scale.batch = number(f, v)?,
            "--mtu" => o.smol.link.mtu = number(f, v)?,
            "--checksums" => {
                o.smol.link.checksums = match v {
                    "on" => true,
                    "off" => false,
                    _ => return Err(bad(f, v)),
                }
            }
            "--buffer-kib" => {
                let kib: usize = number(f, v)?;
                o.smol.rx_buffer = kib.saturating_mul(1024);
                o.smol.tx_buffer = kib.saturating_mul(1024);
            }
            "--time-wait" => {
                o.smol.time_wait = match v {
                    "skip" => TimeWait::Skip,
                    "keep" => TimeWait::Keep,
                    _ => return Err(bad(f, v)),
                }
            }
            _ => return Err(UsageError::UnknownFlag(flag.clone())),
        }
    }
    if o.stack != "smoltcp" {
        return Err(UsageError::UnknownStack(o.stack));
    }
    Ok(o)
}

/// Runs `body` on a current thread runtime with timers.
fn on_runtime<F: std::future::Future>(body: F) -> Result<F::Output, BenchError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
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

fn describe(o: &Options) -> Vec<(String, String)> {
    let mut config = vec![
        ("profile".to_string(), o.profile.clone()),
        ("mtu".into(), o.smol.link.mtu.to_string()),
        ("checksums".into(), o.smol.link.checksums.to_string()),
        ("time_wait".into(), format!("{:?}", o.smol.time_wait)),
        ("nagle".into(), o.smol.nagle.to_string()),
    ];
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
        config.push(("rate.buffer_bytes".into(), o.smol.rx_buffer.to_string()));
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
        config.push((
            "throughput.buffer_bytes".into(),
            o.smol.rx_buffer.to_string(),
        ));
    }
    if o.selection.scale() {
        let steps: Vec<String> = o.scale.steps.iter().map(usize::to_string).collect();
        config.push(("scale.steps".into(), steps.join(",")));
        config.push(("scale.shards".into(), o.shards.to_string()));
        config.push(("scale.buffer_bytes".into(), o.scale_buffer.to_string()));
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

fn make_smol(config: &SmolConfig, index: usize) -> SmolStack {
    let config = SmolConfig {
        seed: config.seed.wrapping_add(index as u64 * 2),
        ..config.clone()
    };
    SmolStack::new(config, Arc::new(MonotonicClock::new()))
}

fn measure<S: TcpStack>(
    options: &Options,
    key: &str,
    make: &(dyn Fn(usize, &SmolConfig) -> S + Sync),
) -> Result<BenchReport, BenchError> {
    let mut report = BenchReport {
        stack_key: key.into(),
        stack_name: make(0, &options.smol).name(),
        label: options.label.clone(),
        host: HostInfo::detect(),
        config: describe(options),
        rate: None,
        throughput: None,
        scale: None,
        finished_unix_secs: 0,
    };
    if options.selection.rate() {
        let runs: Vec<RateRun> = on_threads(options.rate_threads, |index, barrier| {
            let stack = make(index, &options.smol);
            barrier.wait();
            on_runtime(session_rate(&stack, &options.rate))?
        })?;
        report.rate = Some(RateReport::from_runs(runs));
    }
    if options.selection.throughput() {
        let runs: Vec<ThroughputRun> = on_threads(options.transfer_threads, |index, barrier| {
            let stack = make(index, &options.smol);
            barrier.wait();
            on_runtime(throughput(&stack, &options.transfer))?
        })?;
        report.throughput = Some(ThroughputReport::from_runs(&runs));
    }
    if options.selection.scale() {
        let mut smol = options.smol.clone();
        smol.rx_buffer = options.scale_buffer;
        smol.tx_buffer = options.scale_buffer;
        let stacks: Vec<S> = (0..options.shards).map(|i| make(i, &smol)).collect();
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
    let options = match parse(args) {
        Ok(options) => options,
        Err(error) => {
            let _ = writeln!(err, "tcp_bench: {error}\n{USAGE}");
            return 2;
        }
    };
    let report = match measure(&options, "smoltcp", &|index, config| {
        make_smol(config, index)
    }) {
        Ok(report) => report,
        Err(error) => {
            let _ = writeln!(err, "tcp_bench: {error}");
            if let Some(source) = std::error::Error::source(&error) {
                let _ = writeln!(err, "  caused by: {source}");
            }
            return 1;
        }
    };
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
    fn stack_flags_reach_the_smoltcp_config() {
        let o = parse(&args(&[
            "--mtu",
            "9000",
            "--checksums=off",
            "--buffer-kib",
            "128",
            "--time-wait",
            "keep",
        ]))
        .expect("parse");
        assert_eq!(o.smol.link.mtu, 9000);
        assert!(!o.smol.link.checksums);
        assert_eq!(o.smol.rx_buffer, 128 * 1024);
        assert_eq!(o.smol.time_wait, TimeWait::Keep);
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
        assert!(
            json.contains("smoltcp.scale.400.bytes_per_socket")
                || json.contains("smoltcp.scale.400.idle_poll_us")
        );
        assert!(json.contains("smoltcp.rate.sessions_per_s"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn help_exits_with_zero_and_usage_errors_with_two() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(run(&args(&["--help"]), &mut out, &mut err), 0);
        assert!(String::from_utf8_lossy(&out).contains("usage: tcp_bench"));
        assert_eq!(run(&args(&["--wat", "1"]), &mut out, &mut err), 2);
    }
}
