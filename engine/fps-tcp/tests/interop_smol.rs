// SPDX-License-Identifier: GPL-3.0-only
//! The own stack against smoltcp over the lossy link in memory (TRN-26).
//!
//! One own endpoint and one smoltcp endpoint are joined by the impairment
//! model of TRN-22 with Ethernet framing in between, in both directions: the
//! own stack opens and smoltcp accepts, and the other way round. The loss
//! workload runs with a position dependent pattern that the receiver checks
//! byte by byte, at 0 and 1 percent loss with 20 ms in each direction. A tap
//! on the link reads every frame with the smoltcp parser, and the checks
//! below compare what the two stacks put on the wire with what the RFCs ask:
//! MSS, window scale, SACK permitted, and timestamps in the handshake, and
//! their use afterwards.
//!
//! The tests run in real time, because both stacks read the wall clock.

use std::sync::Arc;
use std::time::Duration;

use fps_tcp::interop::{InteropConfig, InteropStack, Pairing};
use fps_tcp::wiretap::{violations, OwnSide, WireSummary};
use fps_tcp::workload::{loss_transfer, LossConfig, LossRun};
use fps_tcp::{ImpairConfig, MonotonicClock, OwnConfig, SmolConfig};

const KIB: u64 = 1024;
/// The loss of the 1 percent runs, in parts per million, in each direction.
const ONE_PERCENT: u32 = 10_000;

fn link(loss_ppm: u32) -> ImpairConfig {
    ImpairConfig {
        loss_ppm,
        delay_us: 20_000,
        ..ImpairConfig::default()
    }
}

struct Outcome {
    run: LossRun,
    wire: WireSummary,
    own: OwnConfig,
    smol_rx: usize,
}

async fn transfer(config: InteropConfig, kib: u64) -> Outcome {
    let (own, smol_rx) = (config.own.clone(), config.smol.rx_buffer);
    let stack = InteropStack::new(config, Arc::new(MonotonicClock::new())).expect("the pair");
    let cfg = LossConfig {
        bytes: kib * KIB,
        timeout: Duration::from_secs(90),
        port: 9400,
        verify: true,
        ..LossConfig::default()
    };
    let run = loss_transfer(&stack, &cfg).await;
    Outcome {
        run,
        wire: stack.wire_summary(),
        own,
        smol_rx,
    }
}

fn assert_completed(label: &str, outcome: &Outcome, kib: u64) {
    let run = &outcome.run;
    assert!(run.completed, "{label}: {:?}", run.failure);
    assert_eq!(run.confirmed_bytes, kib * KIB, "{label}");
    eprintln!(
        "{label}: {:.2} Mbit/s in {:?}, {} retransmissions, {} frames lost, wire: {}",
        run.goodput_mbit_per_s(),
        run.elapsed,
        run.retransmits,
        run.link_lost,
        outcome.wire.to_line()
    );
}

/// The checks of [`violations`] for the own stack, and what smoltcp 0.14 is
/// known to put on the wire.
fn assert_conformant(label: &str, pairing: Pairing, loss: bool, outcome: &Outcome) {
    let (wire, own) = (&outcome.wire, &outcome.own);
    let side = OwnSide {
        opener: pairing == Pairing::OwnClient,
        mtu: own.mtu,
        window_scale: own.window_scale,
        sack: own.sack,
        timestamps: own.timestamps,
        rcv_buffer: own.rcv_buffer,
    };
    let found = violations(wire, &side, loss, Some(outcome.smol_rx as u64));
    assert!(
        found.is_empty(),
        "{label}: {found:?}\nwire: {}",
        wire.to_line()
    );

    let (Some(syn), Some(syn_ack)) = (wire.syn, wire.syn_ack) else {
        unreachable!("violations() reports a missing handshake");
    };
    let (own_syn, peer_syn) = match pairing {
        Pairing::OwnClient => (syn, syn_ack),
        Pairing::OwnServer => (syn_ack, syn),
    };
    assert_eq!(peer_syn.mss, Some(1460), "{label}: MSS of smoltcp");
    assert!(
        !peer_syn.timestamps,
        "{label}: smoltcp 0.14 has no timestamps"
    );
    match pairing {
        // smoltcp opens: it offers SACK and a scale whatever the peer is.
        Pairing::OwnServer => {
            assert!(peer_syn.sack_permitted, "{label}: smoltcp offers SACK");
            assert!(
                peer_syn.window_scale.is_some(),
                "{label}: smoltcp offers a scale"
            );
        }
        // smoltcp accepts: it answers the options that the own stack offered.
        Pairing::OwnClient => {
            assert_eq!(peer_syn.sack_permitted, own_syn.sack_permitted, "{label}");
            assert_eq!(
                peer_syn.window_scale.is_some(),
                own_syn.window_scale.is_some(),
                "{label}"
            );
        }
    }
}

async fn pairing_at(pairing: Pairing, loss_ppm: u32, kib: u64) {
    let label = format!("{} at {} ppm", pairing.name(), loss_ppm);
    let config = InteropConfig::new(pairing, link(loss_ppm));
    let outcome = transfer(config, kib).await;
    assert_completed(&label, &outcome, kib);
    if loss_ppm > 0 {
        assert!(outcome.run.link_lost > 0, "{label}: the link lost nothing");
        assert!(
            outcome.run.retransmits > 0,
            "{label}: nothing was sent again"
        );
    } else {
        assert_eq!(outcome.run.link_lost, 0, "{label}");
    }
    assert_conformant(&label, pairing, loss_ppm > 0, &outcome);
}

#[tokio::test(flavor = "current_thread")]
async fn own_client_to_smoltcp_server_without_loss() {
    pairing_at(Pairing::OwnClient, 0, 1024).await;
}

#[tokio::test(flavor = "current_thread")]
async fn smoltcp_client_to_own_server_without_loss() {
    pairing_at(Pairing::OwnServer, 0, 1024).await;
}

#[tokio::test(flavor = "current_thread")]
async fn own_client_to_smoltcp_server_at_one_percent_loss() {
    pairing_at(Pairing::OwnClient, ONE_PERCENT, 2048).await;
}

#[tokio::test(flavor = "current_thread")]
async fn smoltcp_client_to_own_server_at_one_percent_loss() {
    pairing_at(Pairing::OwnServer, ONE_PERCENT, 2048).await;
}

/// The own stack with every option off offers none, and the transfer still
/// completes at 1 percent loss with plain NewReno recovery and a window of
/// 65,535 bytes.
async fn without_options(pairing: Pairing) {
    let label = format!("{}, own options off", pairing.name());
    let mut config = InteropConfig::new(pairing, link(ONE_PERCENT));
    config.own.window_scale = false;
    config.own.sack = false;
    config.own.timestamps = false;
    let outcome = transfer(config, 512).await;
    assert_completed(&label, &outcome, 512);
    assert!(!outcome.wire.window_scale_agreed() && !outcome.wire.sack_agreed());
    assert_conformant(&label, pairing, true, &outcome);
}

#[tokio::test(flavor = "current_thread")]
async fn own_client_without_options_falls_back_with_smoltcp() {
    without_options(Pairing::OwnClient).await;
}

#[tokio::test(flavor = "current_thread")]
async fn own_server_without_options_falls_back_with_smoltcp() {
    without_options(Pairing::OwnServer).await;
}

/// A smoltcp buffer of 256 KiB gives a larger scale shift on its side, and
/// the own stack applies it when it reads the window.
async fn larger_smoltcp_window(pairing: Pairing) {
    let label = format!("{}, smoltcp buffer 256 KiB", pairing.name());
    let mut config = InteropConfig::new(pairing, link(0));
    config.smol = SmolConfig {
        rx_buffer: 256 * 1024,
        tx_buffer: 256 * 1024,
        idle_poll_micros: 1000,
        ..SmolConfig::default()
    };
    let outcome = transfer(config, 1024).await;
    assert_completed(&label, &outcome, 1024);
    assert!(outcome.wire.window_scale_agreed());
    let smol_dir = match pairing {
        Pairing::OwnClient => outcome.wire.acceptor,
        Pairing::OwnServer => outcome.wire.opener,
    };
    assert!(
        smol_dir.max_window > 65_535,
        "{label}: {}",
        smol_dir.max_window
    );
    assert_conformant(&label, pairing, false, &outcome);
}

#[tokio::test(flavor = "current_thread")]
async fn own_client_to_smoltcp_server_with_a_large_buffer() {
    larger_smoltcp_window(Pairing::OwnClient).await;
}

#[tokio::test(flavor = "current_thread")]
async fn smoltcp_client_to_own_server_with_a_large_buffer() {
    larger_smoltcp_window(Pairing::OwnServer).await;
}
