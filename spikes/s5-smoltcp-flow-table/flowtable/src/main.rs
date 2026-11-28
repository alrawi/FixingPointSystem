// SPDX-License-Identifier: GPL-3.0-only
//! Runs the SPK-05 scenarios and prints the cost of each event.
//!
//! `flowtable-bench [--n N]... [--rounds R] [--single K] [--baseline]`

use fps_spike_smoltcp_flow_table::baseline;
use fps_spike_smoltcp_flow_table::scenario::{self, Phase};
use fps_spike_smoltcp_flow_table::stats::clock_overhead_ns;

fn print_phase(name: &str, p: &Phase) {
    println!("  phase {name}: wall {:.1} ms", p.wall_ns as f64 / 1e6);
    for (label, cat) in [
        ("frame ingest", &p.ingest),
        ("app read/write", &p.app),
        ("timer fire", &p.fire),
        ("wheel tick", &p.tick),
    ] {
        if cat.count > 0 {
            println!("    {label:<15} {}", cat.line());
        }
    }
}

fn main() {
    let mut ns: Vec<usize> = Vec::new();
    let mut rounds = 3;
    let mut single = 2000;
    let mut baseline_on = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--n" => ns.push(args.next().and_then(|v| v.parse().ok()).expect("--n N")),
            "--rounds" => {
                rounds = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .expect("--rounds R")
            }
            "--single" => {
                single = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .expect("--single K")
            }
            "--baseline" => baseline_on = true,
            other => panic!("unknown argument {other}"),
        }
    }
    if ns.is_empty() {
        ns = vec![1_000, 4_000, 16_000, 64_000];
    }
    println!(
        "clock read overhead per Instant::now pair, about {:.0} ns (included in every per event sample)",
        clock_overhead_ns()
    );
    println!(
        "size_of: tcp::Socket {} B, SocketStorage {} B",
        std::mem::size_of::<smoltcp::socket::tcp::Socket<'static>>(),
        std::mem::size_of::<smoltcp::iface::SocketStorage<'static>>()
    );
    for n in ns {
        println!(
            "\n== flow table, {n} connections ({} sockets, both ends) ==",
            2 * n
        );
        let r = scenario::run(n, rounds, single);
        println!(
            "  resident memory per socket after the handshake: {:.0} B",
            r.bytes_per_socket
        );
        print_phase("handshake", &r.handshake);
        print_phase("burst", &r.burst);
        print_phase("single", &r.single);
        print_phase("loss", &r.loss);
        print_phase("close", &r.close);
        println!(
            "  timer fires, whole run, both hosts: {} for a live flow, {} of them sent a frame",
            r.fires, r.fires_with_frame
        );
        println!(
            "  loss: {} frames dropped, {} retransmit fires on the client; server ingested {} frames; orphans {}; stale wheel entries {}; wheel entries left {}; flows left {}",
            r.dropped, r.retransmit_fires, r.server_ingested, r.orphans, r.stale, r.wheel_left, r.flows_left
        );
        if r.failures.is_empty() {
            println!("  checks: all passed");
        } else {
            println!("  checks FAILED: {:?}", r.failures);
            std::process::exit(1);
        }
        if baseline_on {
            println!("== interface loop baseline, {n} connections in one set per host ==");
            let b = baseline::run(n, single.min(500));
            println!("    loop turn       {}", b.turn.line());
            if !b.failures.is_empty() {
                println!("  baseline FAILED: {:?}", b.failures);
                std::process::exit(1);
            }
        }
    }
}
