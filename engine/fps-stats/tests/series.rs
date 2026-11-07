// SPDX-License-Identifier: GPL-3.0-only
//! Tests for the `side` label and the failure reasons of catalog 0.2.0.
//!
//! The contract test reads `contracts/metrics/catalog.yaml` and compares its
//! `side` and `reason` values with the typed enums. The snapshot tests record
//! every new reason and both sides, then read them back from a merged
//! snapshot and from the proto3 JSON.

use std::collections::BTreeMap;
use std::path::PathBuf;

use fps_stats::{
    metric, FailureReason, SeriesKey, SessionCounters, Shard, Side, Snapshot, SnapshotMeta,
    TransportCounters,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Catalog {
    labels: BTreeMap<String, Label>,
}

#[derive(Deserialize)]
struct Label {
    values: Option<Vec<String>>,
}

fn label_values(name: &str) -> Vec<String> {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/metrics/catalog.yaml");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let cat: Catalog = serde_yaml::from_str(&text).unwrap();
    cat.labels[name].values.clone().unwrap()
}

#[test]
fn enums_match_the_catalog_file() {
    let reasons: Vec<&str> = FailureReason::ALL.iter().map(|r| r.as_str()).collect();
    assert_eq!(reasons, label_values("reason"));
    let sides: Vec<&str> = Side::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(sides, label_values("side"));
}

fn count(snap: &Snapshot, metric: fps_stats::MetricId, labels: &[(&str, &str)]) -> u64 {
    let key = SeriesKey::new(metric, labels).unwrap();
    match snap.get(&key) {
        Some(fps_stats::SeriesValue::Count(n)) => *n,
        other => panic!("no counter for {labels:?}: {other:?}"),
    }
}

#[test]
fn snapshot_shows_each_new_reason_on_both_sides() {
    let mut shard = Shard::new(0);
    let client = SessionCounters::register(&mut shard, "dns", "lookup", Side::Client).unwrap();
    let server = SessionCounters::register(&mut shard, "dns", "lookup", Side::Server).unwrap();
    let new_reasons = [
        FailureReason::DatagramRejected,
        FailureReason::TemplateError,
        FailureReason::ExtensionError,
    ];
    for reason in new_reasons {
        client.failed(&mut shard, reason);
        server.failed(&mut shard, reason);
        server.failed(&mut shard, reason);
    }
    client.attempted(&mut shard);
    server.attempted(&mut shard);
    client.add_bytes(&mut shard, 10, 20);
    server.add_bytes(&mut shard, 20, 10);

    let snap = Snapshot::merge_all([&shard.snapshot()]).unwrap();
    for reason in new_reasons {
        for (side, expected) in [(Side::Client, 1), (Side::Server, 2)] {
            let labels = [
                ("application", "dns"),
                ("scenario", "lookup"),
                ("side", side.as_str()),
                ("reason", reason.as_str()),
            ];
            assert_eq!(
                count(&snap, metric::SESSIONS_FAILED, &labels),
                expected,
                "{} on {}",
                reason.as_str(),
                side.as_str()
            );
        }
    }
    assert_eq!(snap.counter_total(metric::SESSIONS_FAILED), 9);
    let c = [
        ("application", "dns"),
        ("scenario", "lookup"),
        ("side", "client"),
    ];
    let s = [
        ("application", "dns"),
        ("scenario", "lookup"),
        ("side", "server"),
    ];
    assert_eq!(count(&snap, metric::BYTES_TX, &c), 10);
    assert_eq!(count(&snap, metric::BYTES_RX, &c), 20);
    assert_eq!(count(&snap, metric::BYTES_TX, &s), 20);
    assert_eq!(count(&snap, metric::SESSIONS_ATTEMPTED, &s), 1);
}

#[test]
fn json_carries_reason_and_side_labels() {
    let mut shard = Shard::new(0);
    let server = SessionCounters::register(&mut shard, "dns", "lookup", Side::Server).unwrap();
    server.failed(&mut shard, FailureReason::TemplateError);
    let snap = Snapshot::merge_all([&shard.snapshot()]).unwrap();
    let json = snap.to_proto_json(&SnapshotMeta::default()).unwrap();
    let hit = json["counters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["metric"] == "sessions.failed" && c["labels"]["reason"] == "template_error")
        .expect("template_error series in JSON");
    assert_eq!(hit["labels"]["side"], "server");
    assert_eq!(hit["count"], "1");
}

#[test]
fn transport_counters_split_by_side_and_merge_across_workers() {
    let mut a = Shard::new(0);
    let mut b = Shard::new(1);
    let a_client = TransportCounters::register(&mut a, "p1", Side::Client).unwrap();
    let a_server = TransportCounters::register(&mut a, "p1", Side::Server).unwrap();
    let b_client = TransportCounters::register(&mut b, "p1", Side::Client).unwrap();
    assert_eq!(a_client.side(), Side::Client);
    a_client.tcp_handshake(&mut a);
    a_client.tls_handshake_failure(&mut a);
    a_server.tcp_handshake(&mut a);
    a_server.tcp_reset_rx(&mut a);
    a_server.tcp_retransmissions(&mut a, 4);
    b_client.tcp_handshake(&mut b);

    let snap = Snapshot::merge_all([&a.snapshot(), &b.snapshot()]).unwrap();
    let client = [("port", "p1"), ("side", "client")];
    let server = [("port", "p1"), ("side", "server")];
    assert_eq!(count(&snap, metric::TCP_HANDSHAKES, &client), 2);
    assert_eq!(count(&snap, metric::TCP_HANDSHAKES, &server), 1);
    assert_eq!(count(&snap, metric::TLS_HANDSHAKE_FAILURES, &client), 1);
    assert_eq!(count(&snap, metric::TCP_RESETS_RX, &server), 1);
    assert_eq!(count(&snap, metric::TCP_RETRANSMISSIONS, &server), 4);
    assert_eq!(count(&snap, metric::TCP_RESETS_TX, &client), 0);
}

#[test]
fn registering_twice_reuses_the_series() {
    let mut shard = Shard::new(0);
    let one = SessionCounters::register(&mut shard, "a", "s", Side::Client).unwrap();
    let two = SessionCounters::register(&mut shard, "a", "s", Side::Client).unwrap();
    one.succeeded(&mut shard);
    two.succeeded(&mut shard);
    let snap = shard.snapshot();
    let labels = [("application", "a"), ("scenario", "s"), ("side", "client")];
    assert_eq!(count(&snap, metric::SESSIONS_SUCCEEDED, &labels), 2);
}

#[test]
fn concurrent_and_duration_carry_the_side_label() {
    let mut shard = Shard::new(0);
    let client = SessionCounters::register(&mut shard, "dns", "lookup", Side::Client).unwrap();
    let server = SessionCounters::register(&mut shard, "dns", "lookup", Side::Server).unwrap();
    client.started(&mut shard);
    client.started(&mut shard);
    server.started(&mut shard);
    client.ended(&mut shard, 1_500);
    server.ended(&mut shard, 2_500);

    let snap = Snapshot::merge_all([&shard.snapshot()]).unwrap();
    let key = |m, side| {
        SeriesKey::new(
            m,
            &[
                ("application", "dns"),
                ("scenario", "lookup"),
                ("side", side),
            ],
        )
        .unwrap()
    };
    let client_labels = [
        ("application", "dns"),
        ("scenario", "lookup"),
        ("side", "client"),
    ];
    assert_eq!(count(&snap, metric::SESSIONS_ATTEMPTED, &client_labels), 2);
    match snap.get(&key(metric::SESSIONS_CONCURRENT, "client")) {
        Some(fps_stats::SeriesValue::Gauge(v)) => assert_eq!(*v, 1.0),
        other => panic!("client gauge: {other:?}"),
    }
    match snap.get(&key(metric::SESSIONS_CONCURRENT, "server")) {
        Some(fps_stats::SeriesValue::Gauge(v)) => assert_eq!(*v, 0.0),
        other => panic!("server gauge: {other:?}"),
    }
    for side in ["client", "server"] {
        match snap.get(&key(metric::SESSIONS_DURATION, side)) {
            Some(fps_stats::SeriesValue::Histogram(h)) => assert_eq!(h.len(), 1, "{side}"),
            other => panic!("{side} histogram: {other:?}"),
        }
    }
}
