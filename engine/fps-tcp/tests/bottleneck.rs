// SPDX-License-Identifier: GPL-3.0-only
//! A short tail-drop queue under the own sender (bug 652).
//!
//! The own client sends through a bottleneck of 20 Mbit/s with a queue of
//! 32 KiB and no added delay, which is the link of the "bottleneck with tail
//! drop" scenario of the kernel interop test, here in memory and without
//! root. With the TCP options on (window scale, SACK, timestamps, CUBIC) the
//! sender once collapsed to a fraction of what it reaches with the options
//! off: a retransmission that the full queue dropped again was never found
//! lost, so the recovery stalled until the retransmission timer, and the
//! flight that the stalled recovery had grown made the next reduction too
//! small. The test compares the two runs on the same link.
//!
//! The test runs in real time, because the stack reads the wall clock. It
//! takes the best of three runs of each kind, so that a loaded host does not
//! decide.
//!
//! The own sender against smoltcp is not part of it: the assembler of
//! smoltcp 0.14 holds four out of order ranges and drops the rest without
//! reporting it, which strands the pipe of the sender until the timer fires,
//! with or without a short queue.

use std::sync::Arc;
use std::time::Duration;

use fps_tcp::workload::{loss_transfer, LossConfig, LossRun};
use fps_tcp::{ImpairConfig, LossyOwn, MonotonicClock, OwnConfig};

const KIB: u64 = 1024;
/// The size of one transfer. At 20 Mbit/s it takes about 0.9 s.
const TRANSFER_KIB: u64 = 2048;

fn bottleneck() -> ImpairConfig {
    ImpairConfig {
        rate_bps: 20_000_000,
        queue_bytes: 32 * 1024,
        ..ImpairConfig::default()
    }
}

fn options(on: bool) -> OwnConfig {
    OwnConfig {
        window_scale: on,
        sack: on,
        timestamps: on,
        ..OwnConfig::default()
    }
}

/// What one transfer measured.
struct Measured {
    run: LossRun,
    /// The retransmission timeouts of the sender.
    timeouts: u64,
}

async fn transfer(on: bool, port: u16) -> Measured {
    let stack =
        LossyOwn::new(options(on), bottleneck(), Arc::new(MonotonicClock::new())).expect("pair");
    let config = LossConfig {
        bytes: TRANSFER_KIB * KIB,
        timeout: Duration::from_secs(60),
        port,
        verify: true,
        ..LossConfig::default()
    };
    let run = loss_transfer(&stack, &config).await;
    let timeouts = stack.stack().counters().client.timeouts;
    Measured { run, timeouts }
}

/// The run with the best goodput of `runs` transfers.
async fn best_of(runs: usize, on: bool, port: u16) -> Measured {
    let mut best: Option<Measured> = None;
    for _ in 0..runs {
        let one = transfer(on, port).await;
        assert!(one.run.completed, "{:?}", one.run.failure);
        let better = best
            .as_ref()
            .is_none_or(|b| one.run.goodput_mbit_per_s() > b.run.goodput_mbit_per_s());
        if better {
            best = Some(one);
        }
    }
    best.expect("at least one run")
}

fn show(label: &str, m: &Measured) {
    eprintln!(
        "{label}: {:.2} Mbit/s in {:?}, {} retransmissions, {} queue drops, {} timeouts",
        m.run.goodput_mbit_per_s(),
        m.run.elapsed,
        m.run.retransmits,
        m.run.link_queue_dropped,
        m.timeouts
    );
}

#[tokio::test(flavor = "current_thread")]
async fn options_do_not_collapse_the_sender_in_a_short_queue() {
    let off = best_of(3, false, 9600).await;
    let on = best_of(3, true, 9601).await;
    show("options off", &off);
    show("options on", &on);

    let (g_on, g_off) = (on.run.goodput_mbit_per_s(), off.run.goodput_mbit_per_s());
    assert!(
        g_on >= 0.8 * g_off,
        "goodput with options on is {g_on:.2} Mbit/s, below 80% of {g_off:.2} Mbit/s"
    );
    // Twice the retransmissions of the run without options, plus a floor for
    // the packets that the probing of the queue costs. The collapse sent
    // more than a thousand.
    let bound = 2 * off.run.retransmits + 40;
    assert!(
        on.run.retransmits <= bound,
        "{} retransmissions with options on, bound {bound} ({} with options off)",
        on.run.retransmits,
        off.run.retransmits
    );
    // The collapse ran into the timer nine times; a loaded host may fire it
    // once without a loss.
    assert!(
        on.timeouts <= 1,
        "{} retransmission timeouts with options on",
        on.timeouts
    );
}
