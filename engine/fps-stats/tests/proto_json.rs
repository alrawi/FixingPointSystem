// SPDX-License-Identifier: GPL-3.0-only
//! Contract test of the proto3 JSON mapping of `CounterSnapshot`.
//!
//! The test reads `contracts/proto/fps/agent/v1/agent.proto` directly and
//! checks that every JSON key the snapshot emits is the lower camel case name
//! of a field of the matching message, and that a snapshot with no default
//! values emits every field.

use std::collections::BTreeSet;
use std::path::PathBuf;

use base64::Engine as _;
use fps_stats::{metric, Shard, Snapshot, SnapshotMeta, Timestamp};
use hdrhistogram::serialization::Deserializer;
use hdrhistogram::Histogram;
use serde_json::Value;

fn proto_text() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/proto/fps/agent/v1/agent.proto");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Returns the JSON names of the fields of `message NAME { ... }`, ignoring
/// comments and `oneof` lines. A field line ends with `name = number;`.
fn json_fields(text: &str, name: &str) -> BTreeSet<String> {
    let head = format!("message {name} {{");
    let start = text
        .find(&head)
        .unwrap_or_else(|| panic!("{head} not in agent.proto"));
    let body = &text[start + head.len()..];
    let end = body
        .find("\n}")
        .unwrap_or_else(|| panic!("{head} is not closed"));
    body[..end]
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//") && l.ends_with(';') && l.contains(" = "))
        .map(|l| {
            let lhs = l.rsplit_once(" = ").unwrap().0;
            camel(lhs.split_whitespace().last().unwrap())
        })
        .collect()
}

fn camel(field: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in field.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().unwrap().keys().cloned().collect()
}

fn full_meta() -> SnapshotMeta {
    SnapshotMeta {
        plan_id: "run-7/agent-a".into(),
        sequence: 12,
        time: Timestamp {
            seconds: 1_793_675_400,
            nanos: 250_000_000,
        },
        phase: "steady".into(),
        is_final: true,
    }
}

fn sample_shard(worker: u32) -> Shard {
    let mut s = Shard::new(worker);
    let l = [("application", "http1"), ("scenario", "web_get")];
    let c = s.counter(metric::SESSIONS_ATTEMPTED, &l).unwrap();
    s.add(c, 10);
    let g = s.gauge(metric::SESSIONS_CONCURRENT, &l).unwrap();
    s.set(g, 3.0);
    let h = s.histogram(metric::SESSIONS_RESPONSE_TIME, &l).unwrap();
    for v in [100, 200, 300, 400] {
        s.record(h, v);
    }
    s
}

#[test]
fn json_keys_are_the_proto_field_names() {
    let proto = proto_text();
    let mut shard = sample_shard(5);
    // An aggregate snapshot carries aggregate, a worker snapshot workerId.
    let worker = shard.snapshot();
    let aggregate = Snapshot::merge_all([&worker]).unwrap();
    let meta = full_meta();

    let snapshot_fields = json_fields(&proto, "CounterSnapshot");
    let w = worker.to_proto_json(&meta).unwrap();
    let a = aggregate.to_proto_json(&meta).unwrap();
    let mut seen: BTreeSet<String> = keys(&w);
    seen.extend(keys(&a));
    assert_eq!(seen, snapshot_fields);
    assert!(keys(&w).contains("workerId") && !keys(&w).contains("aggregate"));
    assert!(keys(&a).contains("aggregate") && !keys(&a).contains("workerId"));

    let counter_fields = json_fields(&proto, "CounterValue");
    let mut seen = BTreeSet::new();
    for entry in w["counters"].as_array().unwrap() {
        seen.extend(keys(entry));
    }
    assert_eq!(seen, counter_fields);

    let histogram_fields = json_fields(&proto, "HistogramValue");
    let mut seen = BTreeSet::new();
    for entry in w["histograms"].as_array().unwrap() {
        seen.extend(keys(entry));
    }
    assert_eq!(seen, histogram_fields);

    // Shard values keep counting after a snapshot.
    let c = shard
        .counter(
            metric::SESSIONS_ATTEMPTED,
            &[("application", "http1"), ("scenario", "web_get")],
        )
        .unwrap();
    shard.inc(c);
    assert_eq!(shard.counter_value(c), Some(11));
}

#[test]
fn json_values_follow_the_mapping() {
    let snap =
        Snapshot::merge_all([&sample_shard(0).snapshot(), &sample_shard(1).snapshot()]).unwrap();
    let json = snap.to_proto_json(&full_meta()).unwrap();
    assert_eq!(json["planId"], "run-7/agent-a");
    assert_eq!(json["sequence"], "12");
    assert_eq!(json["time"], "2026-11-03T03:10:00.250Z");
    assert_eq!(json["phase"], "steady");
    assert_eq!(json["aggregate"], true);
    assert_eq!(json["isFinal"], true);

    let counters = json["counters"].as_array().unwrap();
    assert_eq!(counters.len(), 2);
    assert_eq!(counters[0]["metric"], "sessions.attempted");
    assert_eq!(counters[0]["count"], "20");
    assert_eq!(counters[0]["labels"]["application"], "http1");
    assert_eq!(counters[0]["labels"]["scenario"], "web_get");
    assert_eq!(counters[1]["metric"], "sessions.concurrent");
    assert_eq!(counters[1]["gauge"], 6.0);

    let h = &json["histograms"][0];
    assert_eq!(h["metric"], "sessions.response_time");
    assert_eq!(h["count"], "8");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(h["hdrV2"].as_str().unwrap())
        .unwrap();
    // 0x1c849314 is the cookie of the compressed V2 encoding.
    assert_eq!(&bytes[..4], &[0x1c, 0x84, 0x93, 0x14]);
    let decoded: Histogram<u64> = Deserializer::new().deserialize(&mut &bytes[..]).unwrap();
    assert_eq!(decoded.len(), 8);
    assert!(decoded.equivalent(decoded.max(), 400));
    assert!(decoded.equivalent(decoded.min(), 100));
}

#[test]
fn default_values_are_omitted() {
    let snap = Shard::new(0).snapshot();
    let json = snap.to_proto_json(&SnapshotMeta::default()).unwrap();
    assert_eq!(json, serde_json::json!({ "time": "1970-01-01T00:00:00Z" }));
    let text = snap.to_proto_json_string(&SnapshotMeta::default()).unwrap();
    assert_eq!(text, r#"{"time":"1970-01-01T00:00:00Z"}"#);
}

#[test]
fn serialization_is_deterministic() {
    let build = |order: &[&str]| {
        let mut s = Shard::new(2);
        for scenario in order {
            let c = s
                .counter(
                    metric::BYTES_TX,
                    &[("scenario", scenario), ("application", "dns")],
                )
                .unwrap();
            s.add(c, 1);
        }
        s.snapshot().to_proto_json_string(&full_meta()).unwrap()
    };
    assert_eq!(build(&["b", "a", "c"]), build(&["c", "b", "a"]));
}
