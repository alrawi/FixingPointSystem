// SPDX-License-Identifier: GPL-3.0-only
//! Golden byte transcripts: every dns scenario renders with seed 42, the
//! n-th flow with session id 1 + n, and the bytes in each direction must
//! equal the files under `tests/testdata/`. A UDP transcript prefixes each
//! datagram with its length as two big endian bytes, so the file keeps the
//! datagram boundaries. A TCP transcript holds the stream as sent, which
//! carries the same prefix by RFC 1035 section 4.2.2.
//!
//! Rerun with `UPDATE_GOLDEN=1` to record intended changes.

mod common;

use common::{module_dir, run, scenarios, SEED, SESSION_ID};

#[tokio::test(start_paused = true)]
async fn every_scenario_matches_its_golden_transcript() {
    let testdata = module_dir().join("tests").join("testdata");
    for (name, s) in scenarios() {
        for r in run(&s, SEED, SESSION_ID).await {
            let tag = format!("{name}.{}", r.flow);
            assert!(r.client.is_ok(), "{tag} client: {:?}", r.client);
            assert!(r.server.is_ok(), "{tag} server: {:?}", r.server);
            fps_golden::assert_golden(
                testdata.join(format!("{tag}.seed42.c2s.bin")),
                r.c2s.transcript(),
            );
            fps_golden::assert_golden(
                testdata.join(format!("{tag}.seed42.s2c.bin")),
                r.s2c.transcript(),
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_same_seed_renders_the_same_bytes() {
    for (name, s) in scenarios() {
        let a = run(&s, SEED, SESSION_ID).await;
        let b = run(&s, SEED, SESSION_ID).await;
        let c = run(&s, SEED + 1, SESSION_ID).await;
        for ((a, b), c) in a.iter().zip(&b).zip(&c) {
            assert_eq!(a.c2s, b.c2s, "{name}.{}", a.flow);
            assert_eq!(a.s2c, b.s2c, "{name}.{}", a.flow);
            // The transaction id comes from the session generator.
            assert_ne!(a.c2s, c.c2s, "{name}.{} ignores the seed", a.flow);
        }
    }
}
