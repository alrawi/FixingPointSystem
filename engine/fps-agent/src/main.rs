// SPDX-License-Identifier: GPL-3.0-only
//! The `fps-agent` binary.
//!
//! The binary has two commands.
//!
//! - `fps-agent capabilities [--root DIR]` detects the host capabilities and
//!   prints them, one fact per line.
//! - `fps-agent run --plan FILE --modules DIR [--side both|client|server]
//!   [--linger SECONDS]` runs a plan without a controller and prints one
//!   counter snapshot per second as JSON lines. It exits with
//!   [`EXIT_PASS`] when every criterion holds, [`EXIT_FAIL`] when one fails,
//!   and [`EXIT_ERROR`] on any error.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use fps_agent::assemble::Sides;
use fps_agent::capabilities::{AfXdpSupport, HostCapabilities};
use fps_agent::{detect, Outcome, RunOptions, SystemRoot};

const USAGE: &str = "usage: fps-agent capabilities [--root DIR]
       fps-agent run --plan FILE --modules DIR [--side both|client|server] [--linger SECONDS]";

/// The exit code of a run whose criteria hold.
const EXIT_PASS: u8 = 0;
/// The exit code of a run with a failed criterion.
const EXIT_FAIL: u8 = 1;
/// The exit code of a usage error, an invalid plan or module, or a failed
/// worker.
const EXIT_ERROR: u8 = 2;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("run") {
        return run(&args[1..]);
    }
    let root = match parse_args(&args) {
        Ok(root) => root,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(EXIT_ERROR);
        }
    };
    match detect(&root) {
        Ok(host) => {
            print!("{}", render(&host));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("capability detection failed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> ExitCode {
    let opts = match parse_run_args(args) {
        Ok(o) => o,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(EXIT_ERROR);
        }
    };
    let mut out = std::io::stdout().lock();
    let mut log = std::io::stderr();
    match fps_agent::run(&opts, &mut out, &mut log) {
        Ok(summary) => match summary.outcome {
            Outcome::Pass => ExitCode::from(EXIT_PASS),
            Outcome::Fail => ExitCode::from(EXIT_FAIL),
        },
        Err(e) => {
            eprintln!("fps-agent: error: {}", chain(&e));
            ExitCode::from(EXIT_ERROR)
        }
    }
}

fn chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        s.push_str(": ");
        s.push_str(&c.to_string());
        cur = c.source();
    }
    s
}

fn parse_run_args(args: &[String]) -> Result<RunOptions, String> {
    let (mut plan, mut modules, mut sides, mut linger) = (None, None, Sides::Both, None);
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--plan" => plan = Some(PathBuf::from(value)),
            "--modules" => modules = Some(PathBuf::from(value)),
            "--side" => {
                sides = Sides::parse(value).ok_or_else(|| format!("unknown side {value:?}"))?;
            }
            "--linger" => {
                let secs: f64 = value
                    .parse()
                    .ok()
                    .filter(|s: &f64| s.is_finite() && *s >= 0.0)
                    .ok_or_else(|| format!("invalid --linger {value:?}"))?;
                linger = Some(Duration::from_secs_f64(secs));
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    let mut opts = RunOptions::new(
        plan.ok_or("missing --plan")?,
        modules.ok_or("missing --modules")?,
    );
    opts.sides = sides;
    if let Some(l) = linger {
        opts.linger = l;
    }
    Ok(opts)
}

fn parse_args(args: &[String]) -> Result<SystemRoot, String> {
    match args {
        [cmd] if cmd == "capabilities" => Ok(SystemRoot::host()),
        [cmd, flag, dir] if cmd == "capabilities" && flag == "--root" => Ok(SystemRoot::new(dir)),
        [] => Err("missing command".to_owned()),
        _ => Err(format!("unknown arguments {args:?}")),
    }
}

/// Renders the detected facts as `key value` lines.
fn render(host: &HostCapabilities) -> String {
    let caps = host.to_capabilities();
    let mut out = String::new();
    let backends: Vec<&str> = caps.backends.iter().map(|b| b.proto_name()).collect();
    let af_xdp = match host.af_xdp {
        AfXdpSupport::Supported(e) => format!("supported ({e:?})"),
        AfXdpSupport::Unsupported(e) => format!("unsupported ({e:?})"),
        AfXdpSupport::Unknown => "unknown".to_owned(),
    };
    // Writing to a String cannot fail, so the results are ignored.
    let _ = writeln!(out, "agent_version {}", caps.agent_version);
    let _ = writeln!(out, "engine_api {}", caps.engine_api);
    let _ = writeln!(out, "backends {}", backends.join(","));
    let _ = writeln!(out, "cores {}", caps.cores);
    let _ = writeln!(out, "memory_bytes {}", caps.memory_bytes);
    let _ = writeln!(out, "memory_total_bytes {}", host.memory_total_bytes);
    let _ = writeln!(out, "kernel {}", host.kernel.release);
    let _ = writeln!(out, "af_xdp {af_xdp}");
    let _ = writeln!(out, "numa_nodes {}", host.numa_nodes.len());
    for node in &host.numa_nodes {
        let _ = writeln!(
            out,
            "numa_node {} cpus={} memory_bytes={}",
            node.id,
            node.cpus.len(),
            node.memory_bytes
        );
    }
    for port in &host.ports {
        let p = &port.info;
        let numa = port.numa_node.map_or("none".to_owned(), |n| n.to_string());
        let _ = writeln!(
            out,
            "port {} driver={} speed_mbps={} mac={} mtu={} numa_node={} xdp_native={} xdp_zero_copy={}",
            p.name,
            if p.driver.is_empty() { "none" } else { &p.driver },
            p.speed_mbps,
            p.mac,
            p.mtu,
            numa,
            port.xdp.native,
            port.xdp.zero_copy
        );
    }
    out
}
