// SPDX-License-Identifier: GPL-3.0-only
//! Golden byte transcripts: every sip_rtp scenario runs as a scenario instance
//! with seed 42 and first session id 1, and the datagrams of every flow in
//! each direction must equal the files under `tests/testdata/`. The signaling
//! flow gives `<scenario>.signaling.seed42.c2s.bin` and `.s2c.bin`, and the
//! media flow gives `<scenario>.media.seed42.c2s.bin` and `.s2c.bin`. Every
//! datagram carries a two byte length prefix in the file, so the file keeps
//! the datagram boundaries. The test shortens the call to ten packets in each
//! direction and keeps every other variable.
//!
//! Rerun with `UPDATE_GOLDEN=1` to record intended changes.

mod common;

use common::{module_dir, run, scenarios_with, SEED, SESSION_ID};
use fps_flow::Value;

/// The datagrams of one session, client to server and server to client.
type Directions = (Vec<Vec<u8>>, Vec<Vec<u8>>);

fn overrides() -> Vec<(&'static str, Value)> {
    vec![("talk_ms", Value::Int(200)), ("ring_ms", Value::Int(100))]
}

fn transcript(datagrams: &[Vec<u8>]) -> Vec<u8> {
    datagrams
        .iter()
        .flat_map(|g| (g.len() as u16).to_be_bytes().into_iter().chain(g.clone()))
        .collect()
}

#[tokio::test(start_paused = true)]
async fn every_scenario_matches_its_golden_transcript() {
    let testdata = module_dir().join("tests").join("testdata");
    for (name, s) in scenarios_with(&overrides()) {
        let r = run(&s, SEED, SESSION_ID).await;
        r.result.as_ref().unwrap_or_else(|e| panic!("{name}: {e}"));
        for flow in ["signaling", "media"] {
            let conns = r.conns_of(flow);
            if flow == "media" && name == "busy" {
                assert!(conns.is_empty(), "a refused call opens no media flow");
                continue;
            }
            assert_eq!(conns.len(), 1, "{name} {flow}");
            for (dir, datagrams) in [("c2s", conns[0].c2s_bytes()), ("s2c", conns[0].s2c_bytes())] {
                fps_golden::assert_golden(
                    testdata.join(format!("{name}.{flow}.seed42.{dir}.bin")),
                    transcript(&datagrams),
                );
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
        let bytes = |r: &common::Run| -> Vec<Directions> {
            r.net
                .conns()
                .iter()
                .map(|c| (c.c2s_bytes(), c.s2c_bytes()))
                .collect()
        };
        assert_eq!(bytes(&a), bytes(&b), "{name}");
        // Every scenario draws extensions, tags, branches, and ports from the seed.
        assert!(bytes(&a) != bytes(&c), "{name} ignores the seed");
    }
}
