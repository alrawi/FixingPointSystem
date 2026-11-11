// SPDX-License-Identifier: GPL-3.0-only
//! Golden byte transcripts: every ntp scenario renders with seed 42, the
//! client clock pinned at 2026-11-10T12:00:00Z and the server clock 23 ms
//! later, both advancing by one second with every read, and the n-th flow
//! with session id 1 + n. The bytes in each direction must equal the files
//! under `tests/testdata/`. A transcript prefixes each datagram with its
//! length as two big endian bytes, so the file keeps the datagram boundaries.
//!
//! The golden files hold timestamps, so they depend on the pinned clock, as
//! contracts/README.md requires of a golden that uses `now`.
//!
//! Rerun with `UPDATE_GOLDEN=1` to record intended changes.

mod common;

use common::{run, run_at, scenarios, FlowRun, CLIENT_START_MS, SEED, SESSION_ID};

#[tokio::test(start_paused = true)]
async fn every_scenario_matches_its_golden_transcript() {
    let testdata = common::module_dir().join("tests").join("testdata");
    for (name, s) in scenarios() {
        for r in run(&s, SEED, SESSION_ID).await {
            let tag = format!("{name}.{}", r.flow);
            assert!(r.client.is_ok(), "{tag} client: {:?}", r.client);
            assert!(r.server.is_ok(), "{tag} server: {:?}", r.server);
            fps_golden::assert_golden(
                testdata.join(format!("{tag}.seed42.c2s.bin")),
                FlowRun::transcript(&r.c2s),
            );
            fps_golden::assert_golden(
                testdata.join(format!("{tag}.seed42.s2c.bin")),
                FlowRun::transcript(&r.s2c),
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_same_seed_and_clock_render_the_same_bytes() {
    for (name, s) in scenarios() {
        let a = run(&s, SEED, SESSION_ID).await;
        let b = run(&s, SEED, SESSION_ID).await;
        let seed = run(&s, SEED + 1, SESSION_ID).await;
        let clock = run_at(&s, SEED, SESSION_ID, CLIENT_START_MS + 3_600_000).await;
        for (((a, b), seed), clock) in a.iter().zip(&b).zip(&seed).zip(&clock) {
            assert_eq!(a.c2s, b.c2s, "{name}.{}", a.flow);
            assert_eq!(a.s2c, b.s2c, "{name}.{}", a.flow);
            // The low timestamp bits come from the session generator.
            assert_ne!(a.c2s, seed.c2s, "{name}.{} ignores the seed", a.flow);
            // The timestamps come from the clock, but the sizes do not.
            assert_ne!(a.c2s, clock.c2s, "{name}.{} ignores the clock", a.flow);
            let sizes = |d: &[Vec<u8>]| d.iter().map(Vec::len).collect::<Vec<_>>();
            assert_eq!(sizes(&a.c2s), sizes(&clock.c2s), "{name}.{}", a.flow);
            assert_eq!(sizes(&a.s2c), sizes(&clock.s2c), "{name}.{}", a.flow);
        }
    }
}
