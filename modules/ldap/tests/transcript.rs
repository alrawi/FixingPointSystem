// SPDX-License-Identifier: GPL-3.0-only
//! Golden byte transcripts: every ldap scenario runs with seed 42 and first
//! session id 1, and the bytes of the connection in each direction must equal
//! the files under `tests/testdata/`. The messages carry no key material and
//! no clock, so the files change only when the codec, the directory, or a
//! scenario changes.
//!
//! Rerun with `UPDATE_GOLDEN=1` to record intended changes.

mod common;

use common::{module_dir, run, scenarios, SEED, SESSION_ID};

#[tokio::test(start_paused = true)]
async fn every_scenario_matches_its_golden_transcript() {
    let testdata = module_dir().join("tests").join("testdata");
    let all = scenarios();
    assert_eq!(all.len(), 4);
    for (name, s) in all {
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
    for (name, s) in scenarios() {
        let a = run(&s, SEED, SESSION_ID).await;
        let b = run(&s, SEED, SESSION_ID).await;
        let bytes = |r: &common::Run| -> Vec<(Vec<u8>, Vec<u8>)> {
            r.net.conns().iter().map(|c| (c.c2s(), c.s2c())).collect()
        };
        assert_eq!(bytes(&a), bytes(&b), "{name}");
    }
}

#[tokio::test(start_paused = true)]
async fn the_seed_chooses_the_account() {
    // The scenarios that bind as an account draw it from the session
    // generator, so another seed binds as another account.
    let all = scenarios();
    for wanted in ["bind_search", "failed_bind"] {
        let (name, s) = all.iter().find(|(n, _)| n == wanted).unwrap();
        let mut first_messages = Vec::new();
        for seed in [42u64, 43, 44, 45, 46] {
            let r = run(s, seed, SESSION_ID).await;
            r.result.as_ref().unwrap_or_else(|e| panic!("{name}: {e}"));
            first_messages.push(r.conns_of("session")[0].c2s());
        }
        first_messages.sort();
        first_messages.dedup();
        assert!(first_messages.len() >= 3, "{name} ignores the seed");
    }
}
