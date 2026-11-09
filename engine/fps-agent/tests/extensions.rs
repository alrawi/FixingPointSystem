// SPDX-License-Identifier: GPL-3.0-only
//! WebAssembly module extensions in the agent (AGT-11).
//!
//! The extension is the WAT component of the `fps-wasm` tests, which
//! implements the `extension` world by hand. Its functions are `echo`, `spin`
//! (an endless loop), `grow`, `probe`, `rand`, and `fail`. The tests write a
//! module `xmod` that ships it, put the module tree digest into a plan, and
//! run the plan through the agent over loopback addresses, with one TCP and
//! one UDP scenario that call `echo` and one of each that call `spin`.

#[path = "../../fps-wasm/tests/support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use fps_agent::modules::{self, sha256_digest, tree_digest, ModuleError};
use fps_agent::trust::ModulePolicy;
use fps_agent::{run, RunError, RunOptions};
use fps_stats::{metric, SeriesValue, Snapshot};
use serde_json::Value as Json;
use support::component_bytes;

const MANIFEST: &str = r#"apiVersion: fixingpoint.dev/v1alpha1
kind: Module
name: xmod
version: 0.1.0
type: app
summary: Test module whose template functions are a WebAssembly extension.
engine_api: ">=0.2, <0.3"
license: GPL-3.0-only
functions:
  - name: echo
    params:
      - { name: data, type: bytes }
  - name: spin
  - name: grow
  - name: probe
  - name: rand
  - name: fail
"#;

/// Both sides call the extension, and the client reads the answer.
fn good(name: &str, l4: &str, port: u16) -> String {
    format!(
        r#"scenario: {name}
flows:
  - id: {name}
    module: xmod
    l4: {l4}
    client:
      - connect: {{ port: {port} }}
      - send: "{{{{ xmod.echo('ping') }}}}"
      - expect: {{ pattern: "pong", timeout_ms: 3000 }}
      - close: fin
    server:
      - accept: {{ port: {port} }}
      - expect: {{ pattern: "ping", timeout_ms: 3000 }}
      - send: "{{{{ xmod.echo('pong') }}}}"
      - close: fin
"#
    )
}

/// The client renders an endless loop of the extension.
fn looping(name: &str, l4: &str, port: u16) -> String {
    format!(
        r#"scenario: {name}
flows:
  - id: {name}
    module: xmod
    l4: {l4}
    client:
      - connect: {{ port: {port} }}
      - send: "{{{{ xmod.spin() }}}}"
      - close: fin
    server:
      - accept: {{ port: {port} }}
      - expect: {{ min_bytes: 1, timeout_ms: 500 }}
"#
    )
}

fn contracts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts")
}

/// Writes the module `xmod` under a directory of its own and returns the
/// directory. `extension` is the content of `extension.wasm`, or `None` for a
/// module without one.
fn module_dir(tag: &str, extension: Option<&[u8]>) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("agt11-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("xmod")).unwrap();
    std::fs::write(dir.join("xmod/manifest.yaml"), MANIFEST).unwrap();
    if let Some(bytes) = extension {
        std::fs::write(dir.join("xmod/extension.wasm"), bytes).unwrap();
    }
    dir
}

fn scenario(r#ref: &str, source: &str) -> Json {
    serde_json::json!({
        "ref": r#ref, "module": "xmod",
        "source": source, "digest": sha256_digest(source.as_bytes()),
    })
}

/// Derives a plan from the example plan: the module `xmod` from `dir`, four
/// scenarios (TCP and UDP, each good and looping), one mix entry of each at
/// 20 sessions per second, on loopback pools `127.<c>.0.0/24` and
/// `127.<s>.0.0/24`, with scenario ports from `port`.
fn plan(dir: &Path, c: u8, s: u8, port: u16) -> Json {
    let text = std::fs::read_to_string(contracts().join("examples/plans/http_dns_smoke.json"))
        .expect("example plan");
    let mut plan: Json = serde_json::from_str(&text).expect("example plan JSON");
    let digest = tree_digest("xmod", &dir.join("xmod")).unwrap();
    plan["modules"] = serde_json::json!([
        { "name": "xmod", "version": "0.1.0", "digest": digest }
    ]);
    let entries = [
        ("tcp_good", good("tcp_good", "tcp", port)),
        ("tcp_spin", looping("tcp_spin", "tcp", port + 1)),
        ("udp_good", good("udp_good", "udp", port + 2)),
        ("udp_spin", looping("udp_spin", "udp", port + 3)),
    ];
    plan["scenarios"] = Json::Array(
        entries
            .iter()
            .map(|(n, src)| scenario(&format!("xmod/{n}"), src))
            .collect(),
    );
    plan["mix"] = Json::Array(
        entries
            .iter()
            .map(|(n, _)| {
                serde_json::json!({
                    "id": n, "scenario": format!("xmod/{n}"), "weight": 0.25,
                    "clientPool": "clients", "serverPool": "servers",
                })
            })
            .collect(),
    );
    plan["hostPools"][0]["ipv4Prefix"] = format!("127.{c}.0.0/24").into();
    plan["hostPools"][1]["ipv4Prefix"] = format!("127.{s}.0.0/24").into();
    // The test component refuses session id 13 in `on-open`, which is local
    // session 13 of engine worker 0, so the plan workers are 1 and 2.
    for (i, w) in plan["workers"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        w["workerId"] = (i + 1).into();
        w["rates"] = Json::Array(
            entries
                .iter()
                .map(|(n, _)| serde_json::json!({ "mixEntry": n, "sessionsPerSecond": 20 }))
                .collect(),
        );
    }
    plan["load"]["phases"] = serde_json::json!([
        { "name": "steady", "duration": "2s", "shape": "PHASE_SHAPE_HOLD",
          "startFraction": 1, "endFraction": 1 }
    ]);
    plan["warmup"] = "0s".into();
    plan["criteria"] = serde_json::json!([
        { "metric": "sessions.attempted", "op": "CRITERION_OP_GT", "value": 0 }
    ]);
    plan
}

fn write_plan(name: &str, plan: &Json) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("agt11-plans");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_string_pretty(plan).unwrap()).unwrap();
    path
}

/// Sums the counters of `name` for `scenario` and, when `reason` is set, with
/// that failure reason.
fn count(
    snapshot: &Snapshot,
    name: fps_stats::MetricId,
    scenario: &str,
    reason: Option<&str>,
) -> u64 {
    snapshot
        .iter()
        .filter(|(k, _)| {
            let has =
                |label: &str, want: &str| k.labels().iter().any(|(l, v)| *l == label && v == want);
            k.metric() == name
                && has("scenario", scenario)
                && reason.is_none_or(|r| has("reason", r))
        })
        .map(|(_, v)| match v {
            SeriesValue::Count(c) => *c,
            _ => 0,
        })
        .sum()
}

/// The acceptance test. A plan whose module ships a WAT built extension runs
/// over TCP and over UDP. The good scenarios succeed on both sides. Every
/// session of a looping scenario fails with `extension_error` at its fuel
/// limit, and only those sessions fail, although all four scenarios share one
/// compiled extension and the same worker threads.
#[test]
fn a_looping_extension_fails_only_its_own_sessions_over_tcp_and_udp() {
    let dir = module_dir("run", Some(&component_bytes()));
    let path = write_plan("run.json", &plan(&dir, 85, 86, 18_310));
    let mut options = RunOptions::new(path, dir);
    options.linger = Duration::from_secs(2);
    let (mut out, mut log) = (Vec::new(), Vec::new());
    let summary = run(&options, &mut out, &mut log).expect("run");
    let log = String::from_utf8(log).unwrap();
    assert!(
        log.contains("module xmod runs its WebAssembly extension"),
        "{log}"
    );
    let client = summary.client.expect("client totals");
    let server = summary.server.expect("server totals");

    for name in ["tcp_good", "udp_good"] {
        let attempted = count(&client, metric::SESSIONS_ATTEMPTED, name, None);
        assert!(attempted >= 10, "{name}: {attempted} sessions\n{log}");
        assert_eq!(
            count(&client, metric::SESSIONS_SUCCEEDED, name, None),
            attempted,
            "{name}\n{log}"
        );
        assert_eq!(count(&client, metric::SESSIONS_FAILED, name, None), 0);
        let served = count(&server, metric::SESSIONS_SUCCEEDED, name, None);
        assert!(served > 0, "{name}: no server session succeeded\n{log}");
        assert_eq!(count(&server, metric::SESSIONS_FAILED, name, None), 0);
    }
    for name in ["tcp_spin", "udp_spin"] {
        let attempted = count(&client, metric::SESSIONS_ATTEMPTED, name, None);
        assert!(attempted >= 10, "{name}: {attempted} sessions\n{log}");
        assert_eq!(count(&client, metric::SESSIONS_SUCCEEDED, name, None), 0);
        assert_eq!(
            count(
                &client,
                metric::SESSIONS_FAILED,
                name,
                Some("extension_error")
            ),
            attempted,
            "{name}: every session fails with extension_error\n{log}"
        );
        assert_eq!(
            count(&client, metric::SESSIONS_FAILED, name, None),
            attempted,
            "{name}: no other reason\n{log}"
        );
    }
}

fn prepare_error(dir: &Path, tag: &str) -> RunError {
    let path = write_plan(&format!("{tag}.json"), &plan(dir, 87, 88, 18_320));
    let options = RunOptions::new(path, dir.to_path_buf());
    run(&options, &mut Vec::new(), &mut Vec::new()).expect_err("the plan is refused")
}

#[test]
fn a_module_without_an_extension_costs_nothing() {
    let dir = module_dir("none", None);
    let plan = plan(&dir, 89, 90, 18_330);
    let plan = fps_agent::plan::Plan::parse(&plan.to_string()).expect("plan");
    let loaded = modules::load(&plan, &dir, &ModulePolicy::default()).expect("load");
    assert!(
        loaded.extensions.is_none(),
        "no engine without an extension"
    );
    assert!(
        loaded
            .notes
            .iter()
            .any(|n| n.contains("no native functions")),
        "{:?}",
        loaded.notes
    );
}

#[test]
fn an_extension_that_does_not_compile_refuses_the_plan() {
    let dir = module_dir("garbage", Some(b"not a component"));
    let err = prepare_error(&dir, "garbage");
    assert!(
        matches!(&err, RunError::Module(ModuleError::Extension { module, .. }) if module == "xmod"),
        "{err}"
    );
    assert!(err.to_string().contains("xmod"), "{err}");
}

#[test]
fn an_extension_whose_catalog_differs_from_the_manifest_refuses_the_plan() {
    let dir = module_dir("catalog", Some(&component_bytes()));
    let manifest = MANIFEST.replace("  - name: fail\n", "");
    std::fs::write(dir.join("xmod/manifest.yaml"), manifest).unwrap();
    let err = prepare_error(&dir, "catalog");
    assert!(
        matches!(&err, RunError::Module(ModuleError::Extension { .. })),
        "{err}"
    );
    let mut chain = String::new();
    let mut source: Option<&dyn std::error::Error> = Some(&err);
    while let Some(e) = source {
        chain.push_str(&format!("{e}: "));
        source = e.source();
    }
    assert!(chain.contains("'fail'"), "{chain}");
}

/// The extension is part of the module tree, so the plan digest, and the
/// signature over it, cover its bytes. An extension that changed after the
/// plan was made fails the digest check before anything compiles it.
#[test]
fn the_tree_digest_covers_the_extension() {
    let dir = module_dir("digest", Some(&component_bytes()));
    let plan = plan(&dir, 91, 92, 18_340);
    std::fs::write(dir.join("xmod/extension.wasm"), b"another extension").unwrap();
    let path = write_plan("digest.json", &plan);
    let err = run(
        &RunOptions::new(path, dir),
        &mut Vec::new(),
        &mut Vec::new(),
    )
    .expect_err("the plan is refused");
    assert!(
        matches!(err, RunError::Module(ModuleError::Digest { .. })),
        "{err}"
    );
}
