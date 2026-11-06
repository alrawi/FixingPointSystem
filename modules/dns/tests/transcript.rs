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

use common::{module_dir, run, run_with, scenarios, SEED, SESSION_ID};
use fps_module_dns::Dns;

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

/// The `zone_file` setting that names the shipped zone gives the transcripts
/// of the default behavior byte for byte.
#[tokio::test(start_paused = true)]
async fn a_configured_zone_equal_to_the_bundled_one_changes_nothing() {
    let settings = serde_json::json!({ "zone_file": "zones/example.test.zone" });
    let configured = Dns::from_settings(&settings, &module_dir()).unwrap();
    for (name, s) in scenarios() {
        let a = run(&s, SEED, SESSION_ID).await;
        let b = run_with(&s, SEED, SESSION_ID, &configured).await;
        for (a, b) in a.iter().zip(&b) {
            assert!(b.client.is_ok() && b.server.is_ok(), "{name}.{}", a.flow);
            assert_eq!(a.c2s, b.c2s, "{name}.{}", a.flow);
            assert_eq!(a.s2c, b.s2c, "{name}.{}", a.flow);
        }
    }
}

/// A configured zone with other addresses changes the server answers and
/// leaves the queries alone, and the scenario still completes, because the
/// client binds whatever address the answer carries.
#[tokio::test(start_paused = true)]
async fn a_configured_zone_changes_the_answers() {
    let text = std::fs::read_to_string(module_dir().join("zones/example.test.zone"))
        .unwrap()
        .replace("192.0.2.1\n", "192.0.2.201\n");
    let path = std::env::temp_dir().join(format!("fps-dns-zone-{}.zone", std::process::id()));
    std::fs::write(&path, text).unwrap();
    let configured =
        Dns::from_settings(&serde_json::json!({ "zone_file": path }), &module_dir()).unwrap();
    let (name, s) = scenarios()
        .into_iter()
        .find(|(n, _)| n == "lookup_a")
        .unwrap();
    let a = run(&s, SEED, SESSION_ID).await;
    let b = run_with(&s, SEED, SESSION_ID, &configured).await;
    for (a, b) in a.iter().zip(&b) {
        assert!(b.client.is_ok() && b.server.is_ok(), "{name}.{}", a.flow);
        assert_eq!(a.c2s, b.c2s, "the queries do not change");
        assert_ne!(a.s2c, b.s2c, "the answers come from the configured zone");
        assert_eq!(
            common::text(&b.client_vars, "server_ip").as_deref(),
            Some("192.0.2.201")
        );
    }
}
