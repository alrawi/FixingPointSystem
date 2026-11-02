use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};

use fps_spike_engine::engine::{self, BenchConfig, BenchResult, Mode};
use fps_spike_engine::program::Scenario;
use fps_spike_engine::template::Value;

#[derive(Parser)]
#[command(
    version,
    about = "FixingPointSystem Phase 0 spike S1: flow program engine benchmark"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Compile a scenario and print its operations.
    Check {
        #[arg(long)]
        scenario: PathBuf,
    },
    /// Run clients and emulated servers over loopback and report statistics.
    Bench {
        #[arg(long)]
        scenario: PathBuf,
        #[arg(long, value_enum, default_value = "interp")]
        mode: Mode,
        /// Concurrent client sessions (closed loop).
        #[arg(long, default_value_t = 64)]
        concurrency: usize,
        /// Measurement duration in seconds.
        #[arg(long, default_value_t = 10)]
        duration: u64,
        /// Tokio worker threads. Defaults to the number of CPUs.
        #[arg(long)]
        threads: Option<usize>,
        /// Override a scenario variable, as name=value. Repeatable.
        #[arg(long = "var", value_parser = parse_var)]
        vars: Vec<(String, Value)>,
        /// Cap on new sessions per second across all clients.
        #[arg(long)]
        rate: Option<f64>,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Server address for emulated servers.
        #[arg(long, default_value = "127.0.0.1")]
        server_ip: IpAddr,
        /// Client pool base address. Clients bind to base+1 .. base+hosts.
        #[arg(long, default_value = "127.1.0.0")]
        client_net: Ipv4Addr,
        #[arg(long, default_value_t = 254)]
        client_hosts: u32,
        /// Write the result as JSON to this file.
        #[arg(long)]
        json: Option<PathBuf>,
    },
}

fn parse_var(s: &str) -> Result<(String, Value), String> {
    let (k, v) = s
        .split_once('=')
        .ok_or_else(|| format!("expected name=value, got '{s}'"))?;
    Ok((k.to_string(), Value::parse_override(v)))
}

fn print_result(r: &BenchResult) {
    println!("scenario            {} ({:?})", r.scenario, r.mode);
    println!(
        "concurrency         {}  threads {}",
        r.concurrency, r.runtime_threads
    );
    println!(
        "sessions ok/failed  {} / {}",
        r.sessions_ok, r.sessions_failed
    );
    if !r.failures.is_empty() {
        println!("failures            {:?}", r.failures);
    }
    println!(
        "sessions/s          {:.0} (steady median {:.0})",
        r.sessions_per_s, r.steady_sessions_per_s
    );
    println!("L7 throughput       {:.3} Gbit/s", r.l7_bits_per_s / 1e9);
    let l = &r.session_latency_us;
    println!(
        "latency us          p50 {}  p90 {}  p99 {}  max {}",
        l.p50, l.p90, l.p99, l.max
    );
    println!("max concurrent      {}", r.max_concurrent_sessions);
    println!("cpu cores busy      {:.2}", r.cpu_cores_busy);
    println!("sessions/cpu-s      {:.0}", r.sessions_per_cpu_second);
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Check { scenario } => {
            let s = Scenario::load(&scenario)?;
            println!("scenario {} with {} flow(s)", s.name, s.flows.len());
            for f in &s.flows {
                println!("flow {} (module {})", f.id, f.module);
                for (i, op) in f.client.ops.iter().enumerate() {
                    println!("  client {i:>3}  {op:?}");
                }
                if let Some(server) = &f.server {
                    for (i, op) in server.ops.iter().enumerate() {
                        println!("  server {i:>3}  {op:?}");
                    }
                }
            }
            Ok(())
        }
        Cmd::Bench {
            scenario,
            mode,
            concurrency,
            duration,
            threads,
            vars,
            rate,
            seed,
            server_ip,
            client_net,
            client_hosts,
            json,
        } => {
            if client_hosts == 0 {
                bail!("client-hosts must be positive");
            }
            let mut s = Scenario::load(&scenario)?;
            s.override_vars(&vars)?;
            let threads = threads.unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1)
            });
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(threads)
                .enable_all()
                .build()?;
            let cfg = BenchConfig {
                mode,
                concurrency,
                duration: Duration::from_secs(duration),
                server_ip,
                client_net,
                client_hosts,
                seed,
                rate,
                ..BenchConfig::default()
            };
            let result = rt.block_on(engine::run(s, cfg, threads))?;
            print_result(&result);
            if let Some(path) = json {
                std::fs::write(&path, serde_json::to_string_pretty(&result)?)
                    .with_context(|| format!("writing {}", path.display()))?;
            }
            rt.shutdown_timeout(Duration::from_secs(1));
            Ok(())
        }
    }
}
