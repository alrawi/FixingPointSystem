// SPDX-License-Identifier: GPL-3.0-only
//! Interpreter tests: every dns scenario runs with its client and server
//! against each other, UDP flows over the in memory datagram pair and TCP
//! flows over the in memory duplex, and every message in each direction
//! parses as the DNS message that the scenario describes.

mod common;

use common::{run, scenarios, text, FlowRun, Wire};
use fps_flow::program::L4;
use fps_module_dns::{Message, RData, Rcode, RecordType};

/// What one flow of a scenario must show.
struct Want {
    scenario: &'static str,
    flow: &'static str,
    l4: L4,
    qname: &'static str,
    qtype: RecordType,
    rcode: Rcode,
    truncated: bool,
    /// Record types of the answer section in order.
    answers: &'static [RecordType],
    additional: usize,
    authority: usize,
    /// The address that the client binds to `server_ip`, if any.
    server_ip: Option<&'static str>,
}

const fn udp(
    scenario: &'static str,
    qname: &'static str,
    qtype: RecordType,
    rcode: Rcode,
    answers: &'static [RecordType],
) -> Want {
    Want {
        scenario,
        flow: "query",
        l4: L4::Udp,
        qname,
        qtype,
        rcode,
        truncated: false,
        answers,
        additional: 0,
        authority: 0,
        server_ip: None,
    }
}

use RecordType::{Aaaa, Cname, Mx, Txt, A};

fn wants() -> Vec<Want> {
    vec![
        Want {
            server_ip: Some("192.0.2.1"),
            ..udp("lookup_a", "www.example.test", A, Rcode::NoError, &[A])
        },
        Want {
            server_ip: Some("2001:db8::1"),
            ..udp(
                "lookup_aaaa",
                "www.example.test",
                Aaaa,
                Rcode::NoError,
                &[Aaaa],
            )
        },
        Want {
            server_ip: Some("192.0.2.80"),
            ..udp(
                "lookup_cname",
                "static.example.test",
                A,
                Rcode::NoError,
                &[Cname, Cname, A],
            )
        },
        Want {
            additional: 3,
            ..udp("lookup_mx", "example.test", Mx, Rcode::NoError, &[Mx, Mx])
        },
        udp("lookup_txt", "example.test", Txt, Rcode::NoError, &[Txt]),
        Want {
            authority: 1,
            ..udp("nxdomain", "missing.example.test", A, Rcode::NxDomain, &[])
        },
        Want {
            flow: "udp",
            truncated: true,
            ..udp("tcp_fallback", "big.example.test", Txt, Rcode::NoError, &[])
        },
        Want {
            flow: "tcp",
            l4: L4::Tcp,
            ..udp(
                "tcp_fallback",
                "big.example.test",
                Txt,
                Rcode::NoError,
                &[Txt],
            )
        },
    ]
}

fn check(name: &str, r: &FlowRun, want: &Want) {
    let tag = format!("{name}.{}", r.flow);
    assert!(r.client.is_ok(), "{tag} client: {:?}", r.client);
    assert!(r.server.is_ok(), "{tag} server: {:?}", r.server);
    assert_eq!(r.l4, want.l4, "{tag}");
    match (&r.c2s, want.l4) {
        (Wire::Datagrams(_), L4::Udp) | (Wire::Stream(_), L4::Tcp) => {}
        other => panic!("{tag}: wrong transport {other:?}"),
    }
    let queries = r.c2s.messages();
    let responses = r.s2c.messages();
    assert_eq!((queries.len(), responses.len()), (1, 1), "{tag}");
    let q = Message::parse(&queries[0]).unwrap();
    let a = Message::parse(&responses[0]).unwrap();
    assert!(!q.is_response() && a.is_response(), "{tag}");
    assert_eq!(q.id, a.id, "{tag} transaction id");
    assert_eq!(q.questions, a.questions, "{tag} question");
    assert_eq!(q.questions[0].name.to_string(), want.qname, "{tag}");
    assert_eq!(q.questions[0].qtype, want.qtype.code(), "{tag}");
    assert_eq!(a.rcode(), Some(want.rcode), "{tag}");
    assert_eq!(a.is_truncated(), want.truncated, "{tag}");
    let types: Vec<u16> = a.answers.iter().map(|x| x.data.type_code()).collect();
    let want_types: Vec<u16> = want.answers.iter().map(|t| t.code()).collect();
    assert_eq!(types, want_types, "{tag} answer types");
    assert_eq!(a.additional.len(), want.additional, "{tag} additional");
    assert_eq!(a.authority.len(), want.authority, "{tag} authority");
    if want.l4 == L4::Udp {
        assert!(responses[0].len() <= 512, "{tag} fits the UDP limit");
    } else {
        assert!(responses[0].len() > 512, "{tag} needs TCP");
    }
    assert_eq!(
        text(&r.client_vars, "server_ip").as_deref(),
        want.server_ip,
        "{tag} server_ip"
    );
    // Both sides count every payload byte they wrote and read.
    let c2s: u64 = queries.iter().map(|m| m.len() as u64).sum::<u64>()
        + if want.l4 == L4::Tcp { 2 } else { 0 };
    assert_eq!(r.client_stats.bytes_tx, c2s, "{tag}");
    assert_eq!(r.server_stats.bytes_rx, c2s, "{tag}");
    assert_eq!(r.client_stats.bytes_rx, r.server_stats.bytes_tx, "{tag}");
}

#[tokio::test(start_paused = true)]
async fn every_scenario_resolves_as_described() {
    let wants = wants();
    let loaded = scenarios();
    assert_eq!(loaded.len(), 7);
    let mut checked = 0;
    for (name, s) in &loaded {
        for first_id in [1, 2, 77, 1000] {
            let runs = run(s, 42, first_id).await;
            for r in &runs {
                let want = wants
                    .iter()
                    .find(|w| w.scenario == name && w.flow == r.flow)
                    .unwrap_or_else(|| panic!("no expectation for {name}.{}", r.flow));
                check(name, r, want);
                checked += 1;
            }
        }
    }
    assert_eq!(checked, wants.len() * 4);
}

#[tokio::test(start_paused = true)]
async fn the_mx_answer_carries_the_exchange_addresses() {
    let (_, s) = scenarios()
        .into_iter()
        .find(|(n, _)| n == "lookup_mx")
        .unwrap();
    let r = &run(&s, 42, 1).await[0];
    let a = Message::parse(&r.s2c.messages()[0]).unwrap();
    let exchanges: Vec<String> = a
        .answers
        .iter()
        .filter_map(|x| match &x.data {
            RData::Mx { exchange, .. } => Some(exchange.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(exchanges, ["mail.example.test", "mail2.example.test"]);
    let glue: Vec<String> = a.additional.iter().map(|x| x.name.to_string()).collect();
    assert_eq!(
        glue,
        [
            "mail.example.test",
            "mail.example.test",
            "mail2.example.test"
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn the_fallback_repeats_the_question_over_tcp() {
    let (_, s) = scenarios()
        .into_iter()
        .find(|(n, _)| n == "tcp_fallback")
        .unwrap();
    let runs = run(&s, 42, 1).await;
    assert_eq!(runs.len(), 2);
    let udp_q = Message::parse(&runs[0].c2s.messages()[0]).unwrap();
    let tcp_q = Message::parse(&runs[1].c2s.messages()[0]).unwrap();
    assert_eq!(udp_q.questions, tcp_q.questions);
    // Each flow is its own session, so the transaction ids differ.
    assert_ne!(udp_q.id, tcp_q.id);
    let full = Message::parse(&runs[1].s2c.messages()[0]).unwrap();
    let RData::Txt(strings) = &full.answers[0].data else {
        panic!("not TXT");
    };
    assert_eq!(strings.len(), 4);
    assert!(strings.iter().all(|s| s.len() == 200));
}
