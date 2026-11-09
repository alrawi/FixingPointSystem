// SPDX-License-Identifier: GPL-3.0-only
//! Golden byte transcripts: every ftp scenario runs as a scenario instance
//! with seed 42 and first session id 1, and the bytes of every connection in
//! each direction must equal the files under `tests/testdata/`. The control
//! connection gives `<scenario>.seed42.control.c2s.bin` and `.s2c.bin`, and
//! the n-th data connection gives `<scenario>.seed42.data<n>.c2s.bin` or
//! `.s2c.bin` for the direction that carries bytes. The test shrinks the file
//! sizes and the listings so that the golden files stay small enough to
//! review, and it keeps every other variable.
//!
//! Rerun with `UPDATE_GOLDEN=1` to record intended changes.

mod common;

use common::{module_dir, run, scenarios_with, SEED, SESSION_ID};
use fps_flow::Value;

fn overrides() -> Vec<(&'static str, Value)> {
    vec![
        ("file_min", Value::Int(300)),
        ("file_max", Value::Int(400)),
        ("list_entries", Value::Int(6)),
        ("files", Value::Int(2)),
    ]
}

#[tokio::test(start_paused = true)]
async fn every_scenario_matches_its_golden_transcript() {
    let testdata = module_dir().join("tests").join("testdata");
    for (name, s) in scenarios_with(&overrides()) {
        let r = run(&s, SEED, SESSION_ID).await;
        r.result.as_ref().unwrap_or_else(|e| panic!("{name}: {e}"));
        let control = r.conns_of("control");
        assert_eq!(control.len(), 1, "{name}");
        fps_golden::assert_golden(
            testdata.join(format!("{name}.seed42.control.c2s.bin")),
            control[0].c2s(),
        );
        fps_golden::assert_golden(
            testdata.join(format!("{name}.seed42.control.s2c.bin")),
            control[0].s2c(),
        );
        let data: Vec<_> = r
            .net
            .conns()
            .into_iter()
            .filter(|c| c.flow != "control")
            .collect();
        for (n, c) in data.iter().enumerate() {
            let (c2s, s2c) = (c.c2s(), c.s2c());
            // One direction carries the file or the listing, the other none.
            assert!(c2s.is_empty() != s2c.is_empty(), "{name} data{}", n + 1);
            let (dir, bytes) = if c2s.is_empty() {
                ("s2c", s2c)
            } else {
                ("c2s", c2s)
            };
            fps_golden::assert_golden(
                testdata.join(format!("{name}.seed42.data{}.{dir}.bin", n + 1)),
                &bytes,
            );
        }
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
        // Every scenario draws names, passwords, and ports from the seed.
        assert!(bytes(&a) != bytes(&c), "{name} ignores the seed");
    }
}
