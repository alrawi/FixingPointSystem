// SPDX-License-Identifier: GPL-3.0-only
//! Interpreter tests of the sip_rtp scenarios. Every scenario runs as a
//! scenario instance, both sides of the signaling flow and of the spawned
//! media flow against each other, and the datagrams of each flow are parsed
//! with a model of RFC 3261, RFC 3264, and RFC 3550.
//!
//! The tests check the structure of the call first, which is the acceptance of
//! the module: one INVITE dialog on one pair of ports, an SDP answer that
//! picks a codec of the offer, and one media flow on the port of the answer
//! that carries RTP in both directions at the packet time of the codec and
//! ends before the BYE.

mod common;

use std::time::Duration;

use common::{rtp, run, run_losing_rtp, scenarios, scenarios_with, Conn, Run, Sip, SEED};
use fps_flow::Value;
use fps_interp::FlowState;

struct Call {
    name: &'static str,
    codec: &'static str,
    pt: u8,
    ptime_ms: u64,
    talk_ms: u64,
    ring_ms: u64,
    /// The RTP payload size in bytes, as a range.
    payload: (usize, usize),
}

const CALLS: [Call; 4] = [
    Call {
        name: "call_pcmu",
        codec: "PCMU",
        pt: 0,
        ptime_ms: 20,
        talk_ms: 10_000,
        ring_ms: 2000,
        payload: (160, 160),
    },
    Call {
        name: "call_pcma",
        codec: "PCMA",
        pt: 8,
        ptime_ms: 30,
        talk_ms: 9000,
        ring_ms: 1500,
        payload: (240, 240),
    },
    Call {
        name: "call_g729",
        codec: "G729",
        pt: 18,
        ptime_ms: 20,
        talk_ms: 12_000,
        ring_ms: 2500,
        payload: (20, 20),
    },
    Call {
        name: "call_opus",
        codec: "opus",
        pt: 111,
        ptime_ms: 20,
        talk_ms: 15_000,
        ring_ms: 1200,
        payload: (40, 110),
    },
];

fn call_scenario(name: &str) -> fps_flow::Scenario {
    scenarios()
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no scenario {name}"))
        .1
}

fn signaling(r: &Run) -> Conn {
    let c = r.conns_of("signaling");
    assert_eq!(c.len(), 1, "one SIP session per call");
    c.into_iter().next().unwrap()
}

fn media(r: &Run) -> Conn {
    let c = r.conns_of("media");
    assert_eq!(c.len(), 1, "one media session per call");
    c.into_iter().next().unwrap()
}

fn assert_succeeded(name: &str, r: &Run) {
    r.result.as_ref().unwrap_or_else(|e| panic!("{name}: {e}"));
    for (flow, state) in &r.states {
        assert_eq!(state, &Some(FlowState::Done), "{name} flow {flow}");
    }
    assert!(r.records.iter().all(|x| x.failed.is_none()), "{name}");
}

#[tokio::test(start_paused = true)]
async fn every_scenario_completes_with_both_sides() {
    for (name, s) in scenarios() {
        let r = run(&s, SEED, 1).await;
        assert_succeeded(&name, &r);
        // A call has four sides: signaling and media for the client and the server.
        let want = if name == "busy" { 2 } else { 4 };
        assert_eq!(r.records.len(), want, "{name}");
    }
}

#[tokio::test(start_paused = true)]
async fn the_signaling_is_one_invite_dialog_on_one_pair_of_ports() {
    for c in &CALLS {
        let r = run(&call_scenario(c.name), SEED, 1).await;
        assert_succeeded(c.name, &r);
        let sig = signaling(&r);
        assert_eq!(sig.port, 5060);
        let from_client: Vec<Sip> = sig.c2s_bytes().iter().map(|b| Sip::parse(b)).collect();
        let from_server: Vec<Sip> = sig.s2c_bytes().iter().map(|b| Sip::parse(b)).collect();
        let methods: Vec<_> = from_client.iter().map(Sip::code_or_method).collect();
        let codes: Vec<_> = from_server.iter().map(Sip::code_or_method).collect();
        assert_eq!(methods, ["INVITE", "ACK", "BYE"], "{}", c.name);
        assert_eq!(codes, ["100", "180", "200", "200"], "{}", c.name);

        // The dialog identifiers stay the same in every message.
        let (invite, ack, bye) = (&from_client[0], &from_client[1], &from_client[2]);
        let call_id = invite.header("Call-ID").unwrap();
        for m in from_client.iter().chain(&from_server) {
            assert_eq!(m.header("Call-ID"), Some(call_id), "{}", m.start);
            assert_eq!(m.header("From"), invite.header("From"), "{}", m.start);
        }
        let to_tag = from_server[1].tag("To").expect("180 carries the To tag");
        assert_eq!(from_server[0].tag("To"), None, "100 Trying has no To tag");
        for m in [&from_server[1], &from_server[2], &from_server[3], ack, bye] {
            assert_eq!(m.tag("To").as_deref(), Some(to_tag.as_str()), "{}", m.start);
        }
        // CSeq numbers: the ACK repeats the number of the INVITE, the BYE counts up.
        assert_eq!(invite.header("CSeq"), Some("1 INVITE"));
        assert_eq!(ack.header("CSeq"), Some("1 ACK"));
        assert_eq!(bye.header("CSeq"), Some("2 BYE"));
        assert_eq!(from_server[2].header("CSeq"), Some("1 INVITE"));
        assert_eq!(from_server[3].header("CSeq"), Some("2 BYE"));
        // The responses echo the Via of the request. The ACK of a 2xx and the BYE are
        // new transactions with new branches.
        for resp in &from_server[..3] {
            assert_eq!(resp.header("Via"), invite.header("Via"));
        }
        assert_eq!(from_server[3].header("Via"), bye.header("Via"));
        let branches = [invite.branch(), ack.branch(), bye.branch()];
        assert!(branches.iter().all(|b| b.starts_with("z9hG4bK")));
        assert_ne!(branches[0], branches[1]);
        assert_ne!(branches[1], branches[2]);
        assert_ne!(branches[0], branches[2]);
        // Every request carries Max-Forwards, and the INVITE announces its SDP.
        for m in &from_client {
            assert_eq!(m.header("Max-Forwards"), Some("70"), "{}", m.start);
        }
        assert_eq!(invite.header("Content-Type"), Some("application/sdp"));
        assert_eq!(
            from_server[2].header("Content-Type"),
            Some("application/sdp")
        );
        assert!(ack.body.is_empty() && bye.body.is_empty());
        // The ACK and the BYE go to the contact address of the answer.
        let contact = from_server[2].header("Contact").unwrap();
        let target = contact.trim_start_matches('<').trim_end_matches('>');
        assert_eq!(ack.start, format!("ACK {target} SIP/2.0"));
        assert_eq!(bye.start, format!("BYE {target} SIP/2.0"));
    }
}

#[tokio::test(start_paused = true)]
async fn the_answer_picks_a_codec_of_the_offer_and_the_media_follows_it() {
    for c in &CALLS {
        let r = run(&call_scenario(c.name), SEED, 1).await;
        assert_succeeded(c.name, &r);
        let sig = signaling(&r);
        let offer = Sip::parse(&sig.c2s_bytes()[0]);
        let answer = Sip::parse(&sig.s2c_bytes()[2]);
        // The offer lists the preferred codec first and telephone-event last.
        let offered: Vec<&str> = offer.sdp("m=audio ")[0].split(' ').collect();
        assert_eq!(offered[1], "RTP/AVP");
        assert_eq!(offered.last(), Some(&"101"));
        assert_eq!(offered[2], c.pt.to_string(), "{}", c.name);
        assert!(offer
            .sdp("a=rtpmap:")
            .iter()
            .any(|m| m.ends_with("telephone-event/8000")));
        assert_eq!(offer.sdp("a=ptime:"), [c.ptime_ms.to_string()]);
        // The answer keeps the preferred codec only, the same packet time, and its own port.
        let chosen: Vec<&str> = answer.sdp("m=audio ")[0].split(' ').collect();
        assert_eq!(chosen[2], c.pt.to_string());
        assert_eq!(chosen[3], "101");
        assert_eq!(chosen.len(), 4, "one codec and telephone-event");
        assert!(answer.sdp("a=rtpmap:")[0].starts_with(&format!("{} {}", c.pt, c.codec)));
        assert_eq!(answer.sdp("a=ptime:"), [c.ptime_ms.to_string()]);
        let offer_port: u16 = offered[0].parse().unwrap();
        let answer_port: u16 = chosen[0].parse().unwrap();
        for p in [offer_port, answer_port] {
            assert!(p % 2 == 0 && (16384..=32766).contains(&p), "{p}");
        }
        assert_eq!(answer.sdp("c=IN IP4 "), ["192.0.2.20"]);
        assert_eq!(offer.sdp("c=IN IP4 "), ["192.0.2.10"]);
        // The media flow dials the port of the answer.
        let m = media(&r);
        assert_eq!(m.port, answer_port);
    }
}

#[tokio::test(start_paused = true)]
async fn rtp_flows_in_both_directions_at_the_packet_time() {
    for c in &CALLS {
        let r = run(&call_scenario(c.name), SEED, 1).await;
        assert_succeeded(c.name, &r);
        let m = media(&r);
        let want = (c.talk_ms / c.ptime_ms).max(1) as usize;
        let mut ssrcs = Vec::new();
        for (dir, sent) in [("client", m.c2s()), ("server", m.s2c())] {
            assert_eq!(sent.len(), want, "{} {dir} packets", c.name);
            let packets: Vec<_> = sent.iter().map(|s| rtp(&s.bytes)).collect();
            assert!(packets[0].marker && packets[1..].iter().all(|p| !p.marker));
            let step =
                (u32::try_from(c.ptime_ms).unwrap()) * if c.codec == "opus" { 48 } else { 8 };
            for w in packets.windows(2) {
                assert_eq!(w[1].seq, w[0].seq.wrapping_add(1), "{} {dir}", c.name);
                assert_eq!(
                    w[1].timestamp,
                    w[0].timestamp.wrapping_add(step),
                    "{} {dir}",
                    c.name
                );
                assert_eq!(w[1].ssrc, w[0].ssrc);
            }
            for p in &packets {
                assert_eq!(p.pt, c.pt, "{} {dir}", c.name);
                assert!(
                    (c.payload.0..=c.payload.1).contains(&p.payload),
                    "{} {dir} payload {}",
                    c.name,
                    p.payload
                );
            }
            // The paused clock makes the pacing exact: every gap is the packet time.
            for w in sent.windows(2) {
                assert_eq!(
                    w[1].at - w[0].at,
                    Duration::from_millis(c.ptime_ms),
                    "{} {dir} gap",
                    c.name
                );
            }
            ssrcs.push(packets[0].ssrc);
        }
        assert_ne!(ssrcs[0], ssrcs[1], "each direction has its own source");
    }
}

#[tokio::test(start_paused = true)]
async fn the_media_ends_before_the_bye_and_starts_after_the_ring() {
    for c in &CALLS {
        let r = run(&call_scenario(c.name), SEED, 1).await;
        assert_succeeded(c.name, &r);
        let (sig, m) = (signaling(&r), media(&r));
        let invite_at = sig.c2s()[0].at;
        let bye_at = sig.c2s()[2].at;
        let ok_at = sig.s2c()[2].at;
        assert!(
            ok_at - invite_at >= Duration::from_millis(c.ring_ms),
            "{} rings for the ring time",
            c.name
        );
        let first_rtp = m.c2s()[0].at.min(m.s2c()[0].at);
        let last_rtp = m.c2s().last().unwrap().at.max(m.s2c().last().unwrap().at);
        assert!(first_rtp >= ok_at, "{} media starts after the 200", c.name);
        assert!(last_rtp < bye_at, "{} media ends before the BYE", c.name);
        // The BYE follows the last packet within a few packet times.
        assert!(
            bye_at - last_rtp < Duration::from_millis(c.talk_ms / 10 + 400),
            "{} the BYE comes soon after the media",
            c.name
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_busy_callee_refuses_the_call_with_no_media() {
    let s = call_scenario("busy");
    let r = run(&s, SEED, 1).await;
    assert_succeeded("busy", &r);
    assert!(r.conns_of("media").is_empty());
    let sig = signaling(&r);
    let from_client: Vec<Sip> = sig.c2s_bytes().iter().map(|b| Sip::parse(b)).collect();
    let from_server: Vec<Sip> = sig.s2c_bytes().iter().map(|b| Sip::parse(b)).collect();
    let methods: Vec<_> = from_client.iter().map(Sip::code_or_method).collect();
    let codes: Vec<_> = from_server.iter().map(Sip::code_or_method).collect();
    assert_eq!(methods, ["INVITE", "ACK"]);
    assert_eq!(codes, ["100", "486"]);
    assert_eq!(from_server[1].start, "SIP/2.0 486 Busy Here");
    // The ACK of a failure belongs to the INVITE transaction and repeats its branch.
    assert_eq!(from_client[1].branch(), from_client[0].branch());
    assert_eq!(from_client[1].header("CSeq"), Some("1 ACK"));
    assert_eq!(from_client[1].tag("To"), from_server[1].tag("To"));
    assert!(from_server[1].tag("To").is_some());
    assert!(from_server[1].body.is_empty());
}

#[tokio::test(start_paused = true)]
async fn each_call_draws_its_own_identifiers_and_ports() {
    let s = call_scenario("call_pcmu");
    let a = run(&s, SEED, 1).await;
    let b = run(&s, SEED, 101).await;
    for r in [&a, &b] {
        assert_succeeded("call_pcmu", r);
    }
    let id = |r: &Run| {
        Sip::parse(&signaling(r).c2s_bytes()[0])
            .header("Call-ID")
            .unwrap()
            .to_string()
    };
    assert_ne!(id(&a), id(&b));
    assert_ne!(media(&a).port, media(&b).port);
}

#[tokio::test(start_paused = true)]
async fn a_server_that_accepts_no_offered_codec_fails_the_call() {
    // The offer lists PCMU and PCMA, so a server that accepts only G.729 has no common codec.
    let overrides = [("accept", Value::Str("G729".into()))];
    let (_, s) = scenarios_with(&overrides)
        .into_iter()
        .find(|(n, _)| n == "call_pcmu")
        .unwrap();
    let r = run(&s, SEED, 1).await;
    let err = r
        .result
        .as_ref()
        .expect_err("no common codec must fail the instance");
    let text = err.to_string();
    assert!(text.contains("extension_error"), "{text}");
    assert!(r.conns_of("media").is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_lost_rtp_packet_fails_the_media_flow_and_the_call() {
    // Each side waits for the packet of its peer before it sends the next one, so a
    // packet that the network loses stops the exchange, and the expect times out
    // after its two seconds. The engine does not retransmit datagrams.
    let s = call_scenario("call_g729");
    let r = run_losing_rtp(&s, SEED, 1, 7).await;
    let err = r
        .result
        .as_ref()
        .expect_err("a lost packet must fail the instance");
    let text = err.to_string();
    assert!(
        text.contains("timeout") || text.contains("timed out"),
        "{text}"
    );
    let m = media(&r);
    // The client sent 7 packets, the server answered the 6 that it received, and the
    // media flow ended there, so the BYE never went out.
    assert_eq!(m.s2c().len(), 6);
    assert_eq!(signaling(&r).c2s().len(), 2, "INVITE and ACK only");
}
