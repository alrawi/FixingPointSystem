// SPDX-License-Identifier: GPL-3.0-only
//! Golden byte transcripts: every mqtt scenario runs with seed 42 and first
//! session id 1 in both versions, and the bytes of the connection in each
//! direction must equal the files under `tests/testdata/`. The bytes include
//! the client identifier and the readings, which come from the session
//! generator, so the files change whenever a packet layout or a draw changes.
//! The test shrinks the sessions so that the golden files stay small, and it
//! keeps every other variable.
//!
//! Rerun with `UPDATE_GOLDEN=1` to record intended changes.

mod common;

use common::{module_dir, run, scenarios_with, SEED, SESSION_ID};
use fps_flow::Value;

fn overrides(version: i64) -> Vec<(&'static str, Value)> {
    vec![
        ("version", Value::Int(version)),
        ("messages", Value::Int(2)),
        ("pushes", Value::Int(1)),
        ("pings", Value::Int(2)),
        ("keepalive", Value::Int(5)),
        ("interval_min_ms", Value::Int(10)),
        ("interval_max_ms", Value::Int(20)),
        ("push_min_ms", Value::Int(10)),
        ("push_max_ms", Value::Int(20)),
    ]
}

#[tokio::test(start_paused = true)]
async fn every_scenario_matches_its_golden_transcript() {
    let testdata = module_dir().join("tests").join("testdata");
    for version in [4, 5] {
        for (name, s) in scenarios_with(&overrides(version)) {
            let r = run(&s, SEED, SESSION_ID).await;
            r.result.as_ref().unwrap_or_else(|e| panic!("{name}: {e}"));
            let conns = r.conns_of("session");
            assert_eq!(conns.len(), 1, "{name}");
            let label = format!("{name}.v{version}.seed42");
            fps_golden::assert_golden(testdata.join(format!("{label}.c2s.bin")), conns[0].c2s());
            fps_golden::assert_golden(testdata.join(format!("{label}.s2c.bin")), conns[0].s2c());
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_same_seed_renders_the_same_bytes() {
    for (name, s) in scenarios_with(&overrides(5)) {
        let a = run(&s, SEED, SESSION_ID).await;
        let b = run(&s, SEED, SESSION_ID).await;
        let c = run(&s, SEED + 1, SESSION_ID).await;
        let bytes = |r: &common::Run| -> Vec<(Vec<u8>, Vec<u8>)> {
            r.net.conns().iter().map(|c| (c.c2s(), c.s2c())).collect()
        };
        assert_eq!(bytes(&a), bytes(&b), "{name}");
        // The client identifier comes from the seed, so every scenario differs.
        assert!(bytes(&a) != bytes(&c), "{name} ignores the seed");
    }
}
