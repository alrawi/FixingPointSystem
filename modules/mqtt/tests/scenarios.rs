// SPDX-License-Identifier: GPL-3.0-only
//! Interpreter tests of the mqtt scenarios. Every scenario runs with both
//! sides against each other over an in memory network, and the bytes of the
//! connection are split into control packets with an independent walk over
//! the fixed headers. The tests then check the packet sequence of each
//! direction against the flow that the scenario promises, in MQTT 3.1.1 and in
//! 5.0.

mod common;

use std::time::Duration;

use common::{run, run_flipping, scenarios_with, Conn, Run, SEED};
use fps_flow::Value;
use fps_interp::{FlowState, Side};
use fps_module_mqtt::read::{field, parse, Packet};
use fps_module_mqtt::wire::{get_varint, Kind, Version};

/// Splits a direction of the connection into packets and parses each one.
/// The version comes from the CONNECT of the client, which both sides follow.
fn packets(wire: &[u8], version: Version) -> Vec<Packet> {
    let mut at = 0;
    let mut out = Vec::new();
    while at < wire.len() {
        let (len, n) = get_varint(&wire[at + 1..]).expect("a Remaining Length");
        let end = at + 1 + n + len as usize;
        assert!(
            end <= wire.len(),
            "a packet runs past the end of the stream"
        );
        out.push(parse(version, &wire[at..end]).unwrap_or_else(|e| panic!("{e}")));
        at = end;
    }
    out
}

fn kinds(p: &[Packet]) -> Vec<Kind> {
    p.iter().map(|x| x.kind).collect()
}

fn small(version: i64) -> Vec<(&'static str, Value)> {
    vec![
        ("version", Value::Int(version)),
        ("messages", Value::Int(3)),
        ("pushes", Value::Int(2)),
        ("pings", Value::Int(2)),
        ("keepalive", Value::Int(7)),
        ("interval_min_ms", Value::Int(10)),
        ("interval_max_ms", Value::Int(40)),
        ("push_min_ms", Value::Int(10)),
        ("push_max_ms", Value::Int(40)),
    ]
}

fn session(run: &Run) -> Conn {
    let conns = run.conns_of("session");
    assert_eq!(conns.len(), 1, "one connection per session");
    assert_eq!(conns[0].port, 1883);
    conns[0].clone()
}

fn check(name: &str, run: &Run, version: Version) -> (Vec<Packet>, Vec<Packet>) {
    if let Err(e) = &run.result {
        panic!("{name}: {e}");
    }
    for (flow, state) in &run.states {
        assert_eq!(*state, Some(FlowState::Done), "{name} flow {flow}");
    }
    assert!(run.records.iter().all(|r| r.failed.is_none()), "{name}");
    let conn = session(run);
    (packets(&conn.c2s(), version), packets(&conn.s2c(), version))
}

fn version_of(n: i64) -> Version {
    if n == 5 {
        Version::V5
    } else {
        Version::V311
    }
}

async fn scenario(
    name: &str,
    version: i64,
    extra: &[(&'static str, Value)],
) -> (Vec<Packet>, Vec<Packet>, Run) {
    let mut vars = small(version);
    vars.extend(extra.iter().cloned());
    let all = scenarios_with(&vars);
    let (_, s) = all
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("{name}"));
    let r = run(s, SEED, 1).await;
    let (c, b) = check(name, &r, version_of(version));
    (c, b, r)
}

#[tokio::test(start_paused = true)]
async fn the_manifest_lists_five_scenarios() {
    let all = scenarios_with(&[]);
    let names: Vec<&str> = all.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "publish_qos0",
            "publish_qos1",
            "publish_qos2",
            "subscribe",
            "keepalive"
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn publish_qos0_sends_publishes_without_acknowledgments() {
    for v in [4, 5] {
        let (c, b, _) = scenario("publish_qos0", v, &[]).await;
        let mut want = vec![Kind::Connect];
        want.extend([Kind::Publish; 3]);
        want.push(Kind::Disconnect);
        assert_eq!(kinds(&c), want, "version {v}");
        assert_eq!(kinds(&b), [Kind::Connack], "version {v}");
        assert!(c[1..4]
            .iter()
            .all(|p| p.qos == 0 && p.packet_id.is_none() && !p.retain));
        assert!(c[1..4]
            .iter()
            .all(|p| p.topic == "fps/site-3/sensors/telemetry"));
        // The payloads are JSON readings with a rising sequence number.
        for (i, p) in c[1..4].iter().enumerate() {
            let text = String::from_utf8(p.payload.clone()).unwrap();
            assert!(text.contains(&format!("\"seq\":{}", i + 1)), "{text}");
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_connect_and_the_connack_follow_the_version() {
    let (c4, b4, _) = scenario("publish_qos0", 4, &[]).await;
    assert_eq!(
        (c4[0].version, c4[0].keepalive, c4[0].clean),
        (Version::V311, 7, true)
    );
    assert_eq!(c4[0].user.as_deref(), Some("sensor-0412"));
    assert!(c4[0].client_id.starts_with("mqttjs_") && c4[0].client_id.len() == 15);
    assert!(c4[0].props.is_empty() && b4[0].props.is_empty());

    let (c5, b5, _) = scenario("publish_qos0", 5, &[("session_expiry", Value::Int(300))]).await;
    assert_eq!(c5[0].version, Version::V5);
    assert_eq!(field(&c5[0], "session_expiry_interval").unwrap(), b"300");
    assert_eq!(field(&b5[0], "receive_maximum").unwrap(), b"20");
    assert_eq!(field(&b5[0], "topic_alias_maximum").unwrap(), b"10");
    // The wire holds the protocol level that the variable names.
    let (_, _, run5) = scenario("publish_qos0", 5, &[]).await;
    assert_eq!(&session(&run5).c2s()[2..9], b"\x00\x04MQTT\x05");
}

#[tokio::test(start_paused = true)]
async fn publish_qos1_pairs_every_publish_with_its_puback() {
    for v in [4, 5] {
        let (c, b, _) = scenario(
            "publish_qos1",
            v,
            &[(
                "content_type",
                Value::Str(if v == 5 {
                    "application/json".into()
                } else {
                    String::new()
                }),
            )],
        )
        .await;
        // CONNECT, the retained status, three readings, DISCONNECT.
        let mut want = vec![Kind::Connect];
        want.extend([Kind::Publish; 4]);
        want.push(Kind::Disconnect);
        assert_eq!(kinds(&c), want, "version {v}");
        let mut want = vec![Kind::Connack];
        want.extend([Kind::Puback; 4]);
        assert_eq!(kinds(&b), want, "version {v}");
        let ids: Vec<Option<u16>> = c[1..5].iter().map(|p| p.packet_id).collect();
        assert_eq!(ids, [Some(1), Some(2), Some(3), Some(4)]);
        let acked: Vec<Option<u16>> = b[1..].iter().map(|p| p.packet_id).collect();
        assert_eq!(acked, ids);
        assert!(
            c[1].retain && c[1].topic == "fps/site-3/sensors/status" && c[1].payload == b"online"
        );
        assert!(c[2..5].iter().all(|p| !p.retain && p.qos == 1));
        if v == 5 {
            assert_eq!(field(&c[2], "content_type").unwrap(), b"application/json");
        } else {
            assert!(c[2].props.is_empty());
        }
    }
}

#[tokio::test(start_paused = true)]
async fn publish_qos2_runs_the_four_packet_exchange_for_each_identifier() {
    for v in [4, 5] {
        let (c, b, run) = scenario("publish_qos2", v, &[]).await;
        let mut want = vec![Kind::Connect];
        for _ in 0..3 {
            want.extend([Kind::Publish, Kind::Pubrel]);
        }
        want.push(Kind::Disconnect);
        assert_eq!(kinds(&c), want, "version {v}");
        let mut want = vec![Kind::Connack];
        for _ in 0..3 {
            want.extend([Kind::Pubrec, Kind::Pubcomp]);
        }
        assert_eq!(kinds(&b), want, "version {v}");
        for i in 0..3 {
            let id = Some(i as u16 + 1);
            assert_eq!(c[1 + 2 * i].qos, 2);
            assert_eq!(c[1 + 2 * i].packet_id, id);
            assert_eq!(c[2 + 2 * i].packet_id, id);
            assert_eq!(b[1 + 2 * i].packet_id, id);
            assert_eq!(b[2 + 2 * i].packet_id, id);
        }
        // PUBREL has the fixed flags 0b0010, which the first byte shows as 0x62.
        let wire = session(&run).c2s();
        assert!(wire.windows(2).any(|w| w == [0x62, 0x02]));
    }
}

#[tokio::test(start_paused = true)]
async fn subscribe_grants_the_filters_and_pushes_both_qos() {
    for v in [4, 5] {
        let (c, b, _) = scenario("subscribe", v, &[]).await;
        // CONNECT, SUBSCRIBE, two PUBACKs, UNSUBSCRIBE, DISCONNECT.
        assert_eq!(
            kinds(&c),
            [
                Kind::Connect,
                Kind::Subscribe,
                Kind::Puback,
                Kind::Puback,
                Kind::Unsubscribe,
                Kind::Disconnect
            ],
            "version {v}"
        );
        // CONNACK, SUBACK, two pairs of messages, UNSUBACK.
        assert_eq!(
            kinds(&b),
            [
                Kind::Connack,
                Kind::Suback,
                Kind::Publish,
                Kind::Publish,
                Kind::Publish,
                Kind::Publish,
                Kind::Unsuback
            ],
            "version {v}"
        );
        let sub = &c[1];
        assert_eq!(sub.filters.len(), 2);
        assert_eq!(sub.filters[0].filter, "fps/site-3/commands/#");
        assert_eq!(sub.filters[1].filter, "fps/site-3/config");
        assert!(sub.filters.iter().all(|f| f.qos() == 1));
        assert_eq!(b[1].packet_id, sub.packet_id);
        assert_eq!(b[1].codes, [1, 1]);
        // The pushes alternate between QoS 0 and QoS 1, and the client
        // acknowledges the identifiers of the broker.
        let qos: Vec<u8> = b[2..6].iter().map(|p| p.qos).collect();
        assert_eq!(qos, [0, 1, 0, 1]);
        assert_eq!(c[2].packet_id, b[3].packet_id);
        assert_eq!(c[3].packet_id, b[5].packet_id);
        assert_eq!(b[3].packet_id, Some(1));
        assert_eq!(b[5].packet_id, Some(2));
        assert_eq!(b[6].packet_id, c[4].packet_id);
        if v == 5 {
            assert_eq!(b[6].codes, [0, 0]);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_subscribe_options_of_5_0_reach_the_wire() {
    let (c, _, _) = scenario(
        "subscribe",
        5,
        &[
            ("no_local", Value::Bool(true)),
            ("retain_handling", Value::Int(2)),
            ("qos", Value::Int(2)),
        ],
    )
    .await;
    assert!(c[1].filters.iter().all(|f| f.options == 0b0010_0110));
    // The broker grants at most vars.max_qos.
    let (_, b, _) = scenario("subscribe", 4, &[("qos", Value::Int(2))]).await;
    assert_eq!(b[1].codes, [1, 1]);
}

#[tokio::test(start_paused = true)]
async fn keepalive_pings_keep_the_schedule_of_the_interval() {
    for v in [4, 5] {
        let started = tokio::time::Instant::now();
        let (c, b, _) = scenario("keepalive", v, &[]).await;
        let took = started.elapsed();
        assert_eq!(
            kinds(&c),
            [
                Kind::Connect,
                Kind::Pingreq,
                Kind::Pingreq,
                Kind::Publish,
                Kind::Disconnect
            ],
            "version {v}"
        );
        assert_eq!(
            kinds(&b),
            [Kind::Connack, Kind::Pingresp, Kind::Pingresp],
            "version {v}"
        );
        // Two pings 7 seconds apart: the schedule starts at the CONNACK, so
        // the session lasts 14 seconds plus the handshake and the last message.
        assert!(took >= Duration::from_secs(14), "{took:?}");
        assert!(
            took < Duration::from_secs(14) + Duration::from_secs(3),
            "{took:?}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn keepalive_survives_a_session_of_many_pings() {
    let started = tokio::time::Instant::now();
    let (c, _, _) = scenario(
        "keepalive",
        4,
        &[("pings", Value::Int(200)), ("keepalive", Value::Int(30))],
    )
    .await;
    assert_eq!(c.iter().filter(|p| p.kind == Kind::Pingreq).count(), 200);
    // 200 pings 30 seconds apart: a session of an hour and a half.
    let took = started.elapsed();
    assert!(
        took >= Duration::from_secs(6000) && took < Duration::from_secs(6003),
        "{took:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn the_client_identifier_and_the_readings_differ_between_sessions() {
    let all = scenarios_with(&small(4));
    let (_, s) = all.iter().find(|(n, _)| n == "publish_qos0").unwrap();
    let mut ids = Vec::new();
    let mut payloads = Vec::new();
    for session_id in 1..=6 {
        let r = run(s, SEED, session_id).await;
        let (c, _) = check("publish_qos0", &r, Version::V311);
        ids.push(c[0].client_id.clone());
        payloads.push(c[1].payload.clone());
    }
    ids.sort();
    ids.dedup();
    payloads.sort();
    payloads.dedup();
    assert_eq!(ids.len(), 6);
    assert!(payloads.len() >= 5);
}

#[tokio::test(start_paused = true)]
async fn the_wire_variables_reach_the_packets() {
    let (c, _, _) = scenario(
        "publish_qos0",
        4,
        &[
            ("user", Value::Str("gateway-9".into())),
            ("password", Value::Str("Tst-linen-badger-10".into())),
            ("client_prefix", Value::Str("gw_".into())),
            ("topic", Value::Str("plant/line-4/temp".into())),
            ("keepalive", Value::Int(300)),
        ],
    )
    .await;
    assert_eq!(c[0].user.as_deref(), Some("gateway-9"));
    assert_eq!(c[0].password.as_deref(), Some(&b"Tst-linen-badger-10"[..]));
    assert!(c[0].client_id.starts_with("gw_"));
    assert_eq!(c[0].keepalive, 300);
    assert_eq!(c[1].topic, "plant/line-4/temp");
}

#[tokio::test(start_paused = true)]
async fn long_packets_use_longer_remaining_lengths() {
    // The topic of a PUBLISH moves the Remaining Length to two bytes at 200
    // characters and to three bytes at 20000, and the reads of the broker follow.
    for (n, length_bytes) in [(200usize, 2usize), (20_000, 3)] {
        let topic = format!("fps/{}", "x".repeat(n));
        let (c, _, run) =
            scenario("publish_qos0", 4, &[("topic", Value::Str(topic.clone()))]).await;
        assert_eq!(c[1].topic, topic);
        // Walk the fixed headers to the first PUBLISH.
        let wire = session(&run).c2s();
        let first =
            1 + get_varint(&wire[1..]).unwrap().1 + get_varint(&wire[1..]).unwrap().0 as usize;
        assert_eq!(wire[first] >> 4, 3);
        assert_eq!(get_varint(&wire[first + 1..]).unwrap().1, length_bytes);
    }
}

#[tokio::test(start_paused = true)]
async fn a_corrupt_packet_fails_the_session() {
    for v in [4, 5] {
        let vars = small(v);
        let all = scenarios_with(&vars);
        for (name, s) in &all {
            // The first byte is the type and the flags of the CONNECT. Setting
            // a bit of the reserved flags is a protocol error.
            let r = run_flipping(s, SEED, 1, Some(0)).await;
            assert!(r.result.is_err(), "{name}: a flipped flag went unnoticed");
            assert!(
                r.records
                    .iter()
                    .any(|x| x.side == Side::Server && x.failed.is_some()),
                "{name}: the broker did not fail"
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_scenario_that_runs_in_one_version_refuses_the_properties_of_the_other() {
    // content_type is a property of MQTT 5.0, and the session of 3.1.1 refuses it.
    let all = scenarios_with(&[
        ("content_type", Value::Str("text/plain".into())),
        ("messages", Value::Int(1)),
    ]);
    let (_, s) = all.iter().find(|(n, _)| n == "publish_qos1").unwrap();
    let r = run(s, SEED, 1).await;
    assert!(r.result.is_err());
}

/// The sessions that the estimate averages over. Sizes that the session
/// generator draws vary a lot from session to session, so one session says
/// little about the mean that the controller needs.
const ESTIMATE_SESSIONS: u64 = 20;

/// A session of each scenario at the default parameters moves the payload
/// bytes of `expected_bytes` in the manifest, both directions of every
/// connection, within 2 percent. A scenario whose size depends on the session
/// generator is averaged over [`ESTIMATE_SESSIONS`] session ids. The controller
/// turns byte weights into session shares with these estimates, so a change of
/// a template or a fixture that alters the size of a session must change the
/// manifest.
#[tokio::test(start_paused = true)]
async fn default_parameters_move_the_expected_bytes() {
    let doc: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(common::module_dir().join("manifest.yaml")).unwrap(),
    )
    .unwrap();
    let all = common::scenarios();
    let mut bad = Vec::new();
    if let Some(keys) = doc["expected_bytes"].as_mapping() {
        for key in keys.keys() {
            let key = key.as_str().unwrap_or_default();
            if !all.iter().any(|(name, _)| name == key) {
                bad.push(format!("{key}: an estimate for no scenario"));
            }
        }
    }
    for (name, s) in all {
        let mut sizes = Vec::new();
        for id in 1..=ESTIMATE_SESSIONS {
            let r = common::run(&s, common::SEED, id).await;
            r.result.as_ref().unwrap_or_else(|e| panic!("{name}: {e}"));
            sizes.push(
                r.net
                    .conns()
                    .iter()
                    .map(|c| c.c2s().len() + c.s2c().len())
                    .sum::<usize>(),
            );
        }
        let mean = sizes.iter().sum::<usize>() as f64 / sizes.len() as f64;
        match doc["expected_bytes"][name.as_str()].as_f64() {
            Some(want) if (mean - want).abs() / want < 0.02 => {}
            Some(want) => bad.push(format!(
                "{name}: {mean:.0} bytes against the estimate {want}"
            )),
            None => bad.push(format!("{name}: {mean:.0} bytes and no estimate")),
        }
        eprintln!(
            "MEASURED {name} mean {mean:.1} min {} max {}",
            sizes.iter().min().unwrap(),
            sizes.iter().max().unwrap()
        );
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
