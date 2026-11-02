// SPDX-License-Identifier: GPL-3.0-only
//! The `fps-agent` binary.
//!
//! Today the binary has one command. `fps-agent capabilities [--root DIR]`
//! detects the host capabilities and prints them, one fact per line. The
//! `AgentService` server arrives with a later package.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;
use std::process::ExitCode;

use fps_agent::capabilities::{AfXdpSupport, HostCapabilities};
use fps_agent::{detect, SystemRoot};

const USAGE: &str = "usage: fps-agent capabilities [--root DIR]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = match parse_args(&args) {
        Ok(root) => root,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(2);
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
