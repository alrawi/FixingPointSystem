// SPDX-License-Identifier: GPL-3.0-only
//! Interpreter tests: every ntp scenario runs with its client and server
//! against each other over the in memory datagram pair, and every datagram in
//! each direction parses as the NTP packet that the scenario describes, with
//! the timestamps of a plausible exchange.

mod common;

use common::{int, run, scenarios, text, FlowRun, CLIENT_START_MS, SERVER_AHEAD_MS, STEP_MS};
use fps_module_ntp::wire::{micros_to_fraction, unix_ms};
use fps_module_ntp::Packet;

/// What one flow of a scenario must show.
struct Want {
    scenario: &'static str,
    flow: &'static str,
    /// The version and poll exponent of each request.
    version: u8,
    polls: &'static [i8],
    /// The mode of each request and of each reply.
    request_mode: u8,
    reply_mode: u8,
    /// The stratum of each reply.
    strata: &'static [u8],
    /// The reference identifier of the first reply, as text.
    first_reference: &'static str,
}

const fn want(
    scenario: &'static str,
    flow: &'static str,
    polls: &'static [i8],
    strata: &'static [u8],
) -> Want {
    Want {
        scenario,
        flow,
        version: 4,
        polls,
        request_mode: 3,
        reply_mode: 4,
        strata,
        first_reference: "203.0.113.9",
    }
}

fn wants() -> Vec<Want> {
    vec![
        want("sntp_query", "query", &[0], &[2]),
        want("iburst", "burst", &[6, 6, 6, 6], &[2, 2, 2, 2]),
        want("poll_cycle", "poll", &[6, 6, 7], &[2, 2, 2]),
        Want {
            first_reference: "RATE",
            ..want("kod_rate", "rate", &[4, 6], &[0, 2])
        },
        Want {
            request_mode: 1,
            reply_mode: 2,
            ..want("symmetric_peer", "peer", &[6, 6], &[2, 2])
        },
    ]
}

fn parse_all(datagrams: &[Vec<u8>]) -> Vec<Packet> {
    datagrams
        .iter()
        .map(|d| Packet::parse(d).unwrap())
        .collect()
}

fn check(name: &str, r: &FlowRun, want: &Want) {
    let tag = format!("{name}.{}", r.flow);
    assert!(r.client.is_ok(), "{tag} client: {:?}", r.client);
    assert!(r.server.is_ok(), "{tag} server: {:?}", r.server);
    let n = want.polls.len();
    assert_eq!((r.c2s.len(), r.s2c.len()), (n, n), "{tag} datagram count");
    assert!(r.c2s.iter().chain(&r.s2c).all(|d| d.len() == 48), "{tag}");
    let requests = parse_all(&r.c2s);
    let replies = parse_all(&r.s2c);
    for (i, (q, a)) in requests.iter().zip(&replies).enumerate() {
        let at = format!("{tag} exchange {i}");
        assert_eq!(q.version, want.version, "{at}");
        assert_eq!(a.version, q.version, "{at} echoes the version");
        assert_eq!(q.mode, want.request_mode, "{at}");
        assert_eq!(a.mode, want.reply_mode, "{at}");
        assert_eq!(q.poll, want.polls[i], "{at}");
        assert_eq!(a.stratum, want.strata[i], "{at}");
        // The server copies the client transmit time, and the client time is
        // the one that the pinned clock gave: one tick per request.
        assert_eq!(a.origin, q.transmit, "{at} origin");
        let sent = CLIENT_START_MS + STEP_MS * i as i64;
        assert_eq!(unix_ms(q.transmit), Some(sent), "{at} client clock");
        assert_eq!(
            unix_ms(a.receive),
            Some(sent + SERVER_AHEAD_MS),
            "{at} server clock"
        );
        assert!(a.transmit >= a.receive, "{at}");
        assert!(a.transmit - a.receive <= micros_to_fraction(300), "{at}");
        if a.stratum > 0 {
            assert_eq!(a.leap, 0, "{at}");
            // A server that serves time has set its clock lately.
            assert!(a.reference > 0 && a.reference < a.receive, "{at}");
        }
    }
    if want.request_mode == 3 {
        assert_eq!(
            requests[0].reference_text(),
            "",
            "{tag} a client has no source"
        );
    }
    assert_eq!(replies[0].reference_text(), want.first_reference, "{tag}");
    // Both sides count every payload byte.
    let bytes = 48 * n as u64;
    assert_eq!(r.client_stats.bytes_tx, bytes, "{tag}");
    assert_eq!(r.server_stats.bytes_rx, bytes, "{tag}");
    assert_eq!(r.client_stats.bytes_rx, r.server_stats.bytes_tx, "{tag}");
}

#[tokio::test(start_paused = true)]
async fn every_scenario_exchanges_the_packets_it_describes() {
    let wants = wants();
    let loaded = scenarios();
    assert_eq!(loaded.len(), wants.len());
    for (name, s) in &loaded {
        for first_id in [1, 2, 77, 1000] {
            let runs = run(s, 42, first_id).await;
            assert_eq!(runs.len(), 1, "{name}");
            for r in &runs {
                let want = wants
                    .iter()
                    .find(|w| w.scenario == name && w.flow == r.flow)
                    .unwrap_or_else(|| panic!("no expectation for {name}.{}", r.flow));
                check(name, r, want);
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_client_binds_what_the_reply_says() {
    let by_name = |n: &str| scenarios().into_iter().find(|(s, _)| s == n).unwrap().1;
    let r = &run(&by_name("sntp_query"), 42, 1).await[0];
    assert_eq!(int(&r.client_vars, "server_stratum"), Some(2));
    let r = &run(&by_name("poll_cycle"), 42, 1).await[0];
    assert_eq!(int(&r.client_vars, "server_poll"), Some(7));
    let r = &run(&by_name("kod_rate"), 42, 1).await[0];
    // The code that the client bound before it backed off.
    assert_eq!(text(&r.client_vars, "kiss_code").as_deref(), Some("RATE"));
    assert_eq!(int(&r.client_vars, "server_stratum"), Some(2));
    let r = &run(&by_name("symmetric_peer"), 42, 1).await[0];
    assert_eq!(int(&r.client_vars, "peer_mode"), Some(2));
}

#[tokio::test(start_paused = true)]
async fn the_kiss_of_death_has_the_shape_of_rfc_5905() {
    let (_, s) = scenarios()
        .into_iter()
        .find(|(n, _)| n == "kod_rate")
        .unwrap();
    let r = &run(&s, 42, 1).await[0];
    let kod = Packet::parse(&r.s2c[0]).unwrap();
    assert_eq!((kod.leap, kod.stratum, kod.mode), (3, 0, 4));
    assert_eq!(&kod.reference_id, b"RATE");
    assert_eq!(
        (kod.root_delay, kod.root_dispersion, kod.reference),
        (0, 0, 0)
    );
    // The second answer is a normal one.
    let ok = Packet::parse(&r.s2c[1]).unwrap();
    assert_eq!((ok.leap, ok.stratum), (0, 2));
}

#[tokio::test(start_paused = true)]
async fn a_symmetric_peer_states_its_own_clock() {
    let (_, s) = scenarios()
        .into_iter()
        .find(|(n, _)| n == "symmetric_peer")
        .unwrap();
    let r = &run(&s, 42, 1).await[0];
    for d in &r.c2s {
        let p = Packet::parse(d).unwrap();
        assert_eq!((p.mode, p.stratum), (1, 3));
        assert_eq!(p.reference_text(), "198.51.100.4");
        assert!(p.root_delay > 0 && p.root_dispersion > 0);
        assert!(p.reference > 0 && p.reference < p.transmit);
    }
}

#[tokio::test(start_paused = true)]
async fn the_pause_between_requests_follows_the_poll_interval() {
    // The paused Tokio clock advances only through the pauses of the
    // scenario, so the elapsed virtual time is the sum of the delays.
    for (name, pauses_ms) in [
        ("sntp_query", 0u64),
        ("iburst", 3 * 2_000),
        ("poll_cycle", 64_000 + 128_000),
        ("kod_rate", 64_000),
        ("symmetric_peer", 64_000),
    ] {
        let (_, s) = scenarios().into_iter().find(|(n, _)| n == name).unwrap();
        let start = tokio::time::Instant::now();
        let r = &run(&s, 42, 1).await[0];
        assert!(r.client.is_ok() && r.server.is_ok(), "{name}");
        assert_eq!(start.elapsed().as_millis() as u64, pauses_ms, "{name}");
    }
}
