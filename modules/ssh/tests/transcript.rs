// SPDX-License-Identifier: GPL-3.0-only
//! Golden byte transcripts: every ssh scenario runs with seed 42 and first
//! session id 1, and the bytes of the connection in each direction must equal
//! the files under `tests/testdata/`. The bytes include the ephemeral keys,
//! the host key signature, and the ciphertext, so the files change whenever
//! the key exchange, the key derivation, or the cipher changes. The test
//! shrinks the sessions so that the golden files stay small, and it keeps
//! every other variable.
//!
//! Rerun with `UPDATE_GOLDEN=1` to record intended changes.

mod common;

use common::{module_dir, run, scenarios_with, SEED, SESSION_ID};
use fps_flow::Value;

fn overrides() -> Vec<(&'static str, Value)> {
    vec![
        ("commands", Value::Int(2)),
        ("typed", Value::Int(4)),
        ("motd_min", Value::Int(100)),
        ("motd_max", Value::Int(200)),
        ("output_min", Value::Int(80)),
        ("output_max", Value::Int(300)),
        ("output_bytes", Value::Int(9_000)),
        ("file_bytes", Value::Int(10_000)),
        ("chunk", Value::Int(4_096)),
        ("batch", Value::Int(2)),
        ("window_step", Value::Int(8_192)),
    ]
}

#[tokio::test(start_paused = true)]
async fn every_scenario_matches_its_golden_transcript() {
    let testdata = module_dir().join("tests").join("testdata");
    for (name, s) in scenarios_with(&overrides()) {
        let r = run(&s, SEED, SESSION_ID).await;
        r.result.as_ref().unwrap_or_else(|e| panic!("{name}: {e}"));
        let conns = r.conns_of("session");
        assert_eq!(conns.len(), 1, "{name}");
        fps_golden::assert_golden(
            testdata.join(format!("{name}.seed42.c2s.bin")),
            conns[0].c2s(),
        );
        fps_golden::assert_golden(
            testdata.join(format!("{name}.seed42.s2c.bin")),
            conns[0].s2c(),
        );
    }
}

#[tokio::test(start_paused = true)]
async fn the_same_seed_renders_the_same_bytes() {
    for (name, s) in scenarios_with(&overrides()) {
        let a = run(&s, SEED, SESSION_ID).await;
        let b = run(&s, SEED, SESSION_ID).await;
        let c = run(&s, SEED + 1, SESSION_ID).await;
        let bytes = |r: &common::Run| -> Vec<(Vec<u8>, Vec<u8>)> {
            r.net.conns().iter().map(|c| (c.c2s(), c.s2c())).collect()
        };
        assert_eq!(bytes(&a), bytes(&b), "{name}");
        // Every scenario draws its keys, cookies, names, and data from the seed.
        assert!(bytes(&a) != bytes(&c), "{name} ignores the seed");
    }
}
