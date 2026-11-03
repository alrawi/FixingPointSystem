// SPDX-License-Identifier: GPL-3.0-only
//! Golden byte transcripts: every http1 scenario renders with seed 42 and
//! session id 1, and the bytes in each direction must equal the files under
//! `tests/testdata/`. The test shrinks the body sizes so that the golden files
//! stay small enough to review, and it keeps every other variable.
//!
//! Rerun with `UPDATE_GOLDEN=1` to record intended changes.

mod common;

use common::{module_dir, run, scenarios_with, SEED, SESSION_ID};
use fps_flow::Value;

fn overrides() -> Vec<(&'static str, Value)> {
    vec![
        ("body_size", Value::Int(256)),
        ("page_size", Value::Int(700)),
        ("chunk_size", Value::Int(256)),
        ("objects", Value::Int(2)),
        ("object_size", Value::Int(128)),
        ("upload_size", Value::Int(600)),
    ]
}

#[tokio::test(start_paused = true)]
async fn every_scenario_matches_its_golden_transcript() {
    let testdata = module_dir().join("tests").join("testdata");
    for (name, s) in scenarios_with(&overrides()) {
        let r = run(&s, SEED, SESSION_ID).await;
        assert!(r.client.is_ok(), "{name} client: {:?}", r.client);
        assert!(r.server.is_ok(), "{name} server: {:?}", r.server);
        fps_golden::assert_golden(testdata.join(format!("{name}.seed42.c2s.bin")), &r.c2s);
        fps_golden::assert_golden(testdata.join(format!("{name}.seed42.s2c.bin")), &r.s2c);
    }
}

#[tokio::test(start_paused = true)]
async fn the_same_seed_renders_the_same_bytes() {
    for (name, s) in scenarios_with(&overrides()) {
        let a = run(&s, SEED, SESSION_ID).await;
        let b = run(&s, SEED, SESSION_ID).await;
        assert_eq!(a.c2s, b.c2s, "{name}");
        assert_eq!(a.s2c, b.s2c, "{name}");
        let c = run(&s, SEED + 1, SESSION_ID).await;
        let differs = a.c2s != c.c2s || a.s2c != c.s2c;
        // Every scenario sends at least one fill() body, which depends on the seed.
        assert!(differs, "{name} ignores the seed");
    }
}
