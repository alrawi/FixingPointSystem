// SPDX-License-Identifier: GPL-3.0-only
//! The impaired link (TRN-22) under the own stack and smoltcp, through the
//! loss workload. The tests run in real time, because both stacks read the
//! wall clock, so each keeps its transfer small.

use std::sync::Arc;
use std::time::Duration;

use fps_tcp::workload::{loss_transfer, LossConfig, LossRun};
use fps_tcp::{ImpairConfig, LossyOwn, MonotonicClock, OwnConfig, SmolConfig, SmolStack, TcpStack};

const KIB: u64 = 1024;

fn config(kib: u64) -> LossConfig {
    LossConfig {
        bytes: kib * KIB,
        timeout: Duration::from_secs(40),
        ..LossConfig::default()
    }
}

fn link(loss_ppm: u32, delay_ms: i64) -> ImpairConfig {
    ImpairConfig {
        loss_ppm,
        delay_us: delay_ms * 1000,
        ..ImpairConfig::default()
    }
}

fn own(link: ImpairConfig) -> LossyOwn {
    LossyOwn::new(OwnConfig::default(), link, Arc::new(MonotonicClock::new())).expect("own pair")
}

fn smol(link: ImpairConfig) -> SmolStack {
    let config = SmolConfig {
        impair: Some(link),
        idle_poll_micros: 1000,
        ..SmolConfig::default()
    };
    SmolStack::new(config, Arc::new(MonotonicClock::new()))
}

fn assert_delayed_without_loss(run: &LossRun, delay_ms: u64) {
    assert!(run.completed, "{:?}", run.failure);
    // A loaded host can fire a timer early, and smoltcp resends a few
    // segments without a loss, so the count of retransmissions is not asserted.
    assert_eq!(run.link_lost, 0);
    // The data takes one delay to arrive and the confirmation one more.
    assert!(
        run.elapsed >= Duration::from_millis(2 * delay_ms - 2),
        "elapsed {:?}",
        run.elapsed
    );
    assert!(run.goodput_mbit_per_s() > 0.0);
}

#[tokio::test(flavor = "current_thread")]
async fn own_stack_completes_over_a_delayed_link() {
    let stack = own(link(0, 20));
    let run = loss_transfer(&stack, &config(256)).await;
    assert_delayed_without_loss(&run, 20);
}

#[tokio::test(flavor = "current_thread")]
async fn smoltcp_completes_over_a_delayed_link() {
    let stack = smol(link(0, 20));
    let run = loss_transfer(&stack, &config(256)).await;
    assert_delayed_without_loss(&run, 20);
}

#[tokio::test(flavor = "current_thread")]
async fn own_stack_retransmits_over_a_lossy_link_and_the_two_counts_agree() {
    let stack = own(link(20_000, 5));
    let run = loss_transfer(&stack, &config(192)).await;
    assert!(run.link_lost > 0, "the link lost nothing");
    assert!(
        run.retransmits > 0,
        "the own stack never sent a segment again"
    );
    // The link counts at its ingress what the host counts as it resends, so
    // the counter of the stack cannot be larger than the count at the link.
    let host = stack.stack().counters();
    assert!(host.client.retransmits > 0);
    assert!(
        host.client.retransmits <= run.retransmits,
        "host {} link {}",
        host.client.retransmits,
        run.retransmits
    );
    assert!(run.completed, "{:?}", run.failure);
}

#[tokio::test(flavor = "current_thread")]
async fn smoltcp_retransmits_over_a_lossy_link() {
    let stack = smol(link(20_000, 5));
    let run = loss_transfer(&stack, &config(192)).await;
    assert!(run.link_lost > 0, "the link lost nothing");
    assert!(run.completed, "{:?}", run.failure);
    assert!(run.retransmits > 0, "smoltcp never sent a segment again");
}

#[tokio::test(flavor = "current_thread")]
async fn a_rate_limit_caps_the_goodput_on_both_stacks() {
    let limited = ImpairConfig {
        rate_bps: 8_000_000,
        queue_bytes: 64 * 1024,
        ..link(0, 2)
    };
    let (own_run, smol_run) = (
        loss_transfer(&own(limited), &config(128)).await,
        loss_transfer(&smol(limited), &config(128)).await,
    );
    for (name, run) in [("own", own_run), ("smoltcp", smol_run)] {
        assert!(run.completed, "{name}: {:?}", run.failure);
        // 128 KiB at 8 Mbit/s take 131 ms, so the goodput stays below it.
        assert!(
            run.goodput_mbit_per_s() < 8.0,
            "{name}: {}",
            run.goodput_mbit_per_s()
        );
        assert!(
            run.elapsed >= Duration::from_millis(120),
            "{name}: {:?}",
            run.elapsed
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_small_queue_at_a_slow_rate_drops_frames_and_the_stack_sends_again() {
    // The window of 64 KiB is far above the queue, so the first burst
    // overflows it. smoltcp recovers from a loss by timeout, so a transfer
    // through such a queue is slow, and the test looks at the drops only.
    let limited = ImpairConfig {
        rate_bps: 8_000_000,
        queue_bytes: 12 * 1024,
        ..link(0, 2)
    };
    let stack = smol(limited);
    let cfg = LossConfig {
        timeout: Duration::from_secs(3),
        ..config(96)
    };
    let run = loss_transfer(&stack, &cfg).await;
    assert!(run.link_queue_dropped > 0, "the queue never overflowed");
    assert_eq!(run.link_lost, 0);
    assert!(run.retransmits > 0);
}

#[tokio::test(flavor = "current_thread")]
async fn a_transfer_that_cannot_finish_is_reported_not_raised() {
    // Every frame is lost, so the handshake never completes.
    let stack = own(link(1_000_000, 1));
    let cfg = LossConfig {
        timeout: Duration::from_millis(600),
        ..config(16)
    };
    let run = loss_transfer(&stack, &cfg).await;
    assert!(!run.completed);
    assert_eq!(run.goodput_mbit_per_s(), 0.0);
    assert!(run.failure.is_some());
    assert!(run.link_lost > 0);
    assert!(stack.name().contains("own"));
}
