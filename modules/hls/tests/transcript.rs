// SPDX-License-Identifier: GPL-3.0-only
//! Golden byte transcripts: every hls scenario runs as a scenario instance
//! with seed 42 and first session id 1, and the bytes of every connection in
//! each direction must equal the files under `tests/testdata/`. The files are
//! `<scenario>.<flow>.<n>.seed42.c2s.bin` and `.s2c.bin`, with `n` the number
//! of the connection of the flow in the order of their `connect`. The test
//! shrinks the ladder and the segment duration so that the segments stay a few
//! packets long and the files stay reviewable, and keeps every other variable.
//!
//! Rerun with `UPDATE_GOLDEN=1` to record intended changes.

mod common;

use common::{module_dir, run, scenarios_with, Run, SEED, SESSION_ID};
use fps_flow::Value;

fn overrides() -> Vec<(&'static str, Value)> {
    vec![
        (
            "ladder",
            Value::Str("16000@256x144,32000@426x240,64000@640x360".into()),
        ),
        ("segment_ms", Value::Int(1000)),
        ("stages", Value::Int(2)),
        ("stage_segments", Value::Int(2)),
        ("segments", Value::Int(4)),
        ("cycles", Value::Int(3)),
    ]
}

#[tokio::test(start_paused = true)]
async fn every_scenario_matches_its_golden_transcript() {
    let testdata = module_dir().join("tests").join("testdata");
    for (name, s) in scenarios_with(&overrides()) {
        let r = run(&s, SEED, SESSION_ID).await;
        r.result.as_ref().unwrap_or_else(|e| panic!("{name}: {e}"));
        for flow in &s.flows {
            for (n, c) in r.conns_of(&flow.id).iter().enumerate() {
                for (dir, bytes) in [("c2s", c.c2s()), ("s2c", c.s2c())] {
                    fps_golden::assert_golden(
                        testdata.join(format!("{name}.{}.{n}.seed42.{dir}.bin", flow.id)),
                        bytes,
                    );
                }
            }
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_same_seed_renders_the_same_bytes() {
    for (name, s) in scenarios_with(&overrides()) {
        let a = run(&s, SEED, SESSION_ID).await;
        let b = run(&s, SEED, SESSION_ID).await;
        let c = run(&s, SEED + 1, SESSION_ID).await;
        let bytes = |r: &Run| -> Vec<(Vec<u8>, Vec<u8>)> {
            r.net.conns().iter().map(|c| (c.c2s(), c.s2c())).collect()
        };
        assert_eq!(bytes(&a), bytes(&b), "{name}");
        // The random payload of the segments comes from the seed.
        assert!(bytes(&a) != bytes(&c), "{name} ignores the seed");
    }
}
