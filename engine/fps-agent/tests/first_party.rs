// SPDX-License-Identifier: GPL-3.0-only
//! AGT-13: a plan whose scenarios call the first party mail modules runs in
//! the standalone mode.
//!
//! The plan lists smtp, imap, and pop3 and never content, as the controller
//! compiles it, although every scenario calls `content.email` or
//! `content.binary` for its messages. The test loads the module trees of the
//! repository, so the digests are the real tree digests. The sessions pass
//! only when the agent dispatches the native functions of each mail crate and
//! of the content crate that the manifests require.

use std::path::{Path, PathBuf};

use fps_agent::modules::{sha256_digest, tree_digest};
use fps_agent::{run, Outcome, RunOptions};
use serde_json::Value as Json;

// The tests of this file run in parallel on a runner with two cores, and they
// need no lock. In AGT-17 the ftp test failed two of 95 sessions in the run
// where a third test first ran beside it, and a mutex made the tests take
// turns. The mutex hid the cause (AGT-18). The two failed sessions were one
// ftp instance: `mixed_session` picks the port of each data connection with
// `rand_int(data_port_min, data_port_max)`, and the plan seed makes those draws
// the same in every run. With the range of 200 ports that the test used, nine
// ports were drawn twice by 72 draws, so nine pairs of instances wanted the
// same data port. A pair that overlaps in time fails: both clients connect to
// the one listener, which accepts a connection and drops, and the kernel resets
// the one that waited in its backlog (`unexpected_reset` for that flow,
// `aborted` for the rest of the instance). Whether a pair overlaps depends on
// the CPU, and a slow second test beside it made that likely. The ftp test now
// draws from `FTP_DATA_PORTS`, a range in which the 72 draws of the seed of
// `mail_plan` are all different, so no two instances share a data port. I
// found the range by logging every bound data port of the runtime. A change to
// the scenario, to the seed, or to the rate changes the draws, so the repeat
// tests at the end of the file run the three tests together many times, and a
// collision shows up there first.

/// The data ports of the ftp scenario, `data_port_min` and `data_port_max`.
const FTP_DATA_PORTS: (u32, u32) = (53_000, 62_999);

fn repo_modules() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules")
}

fn example() -> Json {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/examples/plans/http_dns_smoke.json"),
    )
    .expect("example plan");
    serde_json::from_str(&text).expect("example plan JSON")
}

fn manifest_version(module: &str) -> String {
    let text = std::fs::read_to_string(repo_modules().join(module).join("manifest.yaml"))
        .expect("manifest");
    text.lines()
        .find_map(|l| l.strip_prefix("version:"))
        .map(|v| v.trim().trim_matches('"').to_owned())
        .expect("version")
}

/// Returns a loopback plan with one mix entry per `(scenario ref, port)`.
fn mail_plan(entries: &[(&str, u16)]) -> Json {
    let mut plan = example();
    let mut modules: Vec<&str> = entries
        .iter()
        .map(|(r, _)| r.split_once('/').expect("module/scenario").0)
        .collect();
    modules.sort_unstable();
    modules.dedup();
    plan["modules"] = modules
        .iter()
        .map(|m| {
            serde_json::json!({
                "name": m,
                "version": manifest_version(m),
                "digest": tree_digest(m, &repo_modules().join(m)).expect("digest"),
            })
        })
        .collect();
    let mut scenarios = Vec::new();
    let mut mix = Vec::new();
    let mut rates = Vec::new();
    for (scenario_ref, port) in entries {
        let (module, name) = scenario_ref.split_once('/').expect("module/scenario");
        let source = std::fs::read_to_string(
            repo_modules()
                .join(module)
                .join("scenarios")
                .join(format!("{name}.yaml")),
        )
        .expect("scenario source");
        scenarios.push(serde_json::json!({
            "ref": scenario_ref,
            "module": module,
            "digest": sha256_digest(source.as_bytes()),
            "source": source,
        }));
        mix.push(serde_json::json!({
            "id": name,
            "scenario": scenario_ref,
            "weight": 1,
            "params": { "server_port": port },
            "clientPool": "clients",
            "serverPool": "servers",
        }));
        rates.push(serde_json::json!({ "mixEntry": name, "sessionsPerSecond": 4 }));
    }
    plan["scenarios"] = scenarios.into();
    plan["mix"] = mix.into();
    plan["hostPools"][0]["ipv4Prefix"] = "127.81.0.0/24".into();
    plan["hostPools"][1]["ipv4Prefix"] = "127.82.0.0/24".into();
    for w in plan["workers"].as_array_mut().expect("workers") {
        w["rates"] = rates.clone().into();
    }
    plan["load"]["phases"] = serde_json::json!([
        { "name": "steady", "duration": "3s", "shape": "PHASE_SHAPE_HOLD",
          "startFraction": 1, "endFraction": 1 }
    ]);
    plan["warmup"] = "0s".into();
    plan
}

fn succeeded(line: &Json, application: &str) -> u64 {
    line["counters"]
        .as_array()
        .map(|cs| {
            cs.iter()
                .filter(|c| {
                    c["metric"] == "sessions.succeeded"
                        && c["labels"]["application"] == application
                        && c["labels"]["side"] == "client"
                })
                .map(|c| {
                    c["count"]
                        .as_str()
                        .unwrap_or("0")
                        .parse::<u64>()
                        .unwrap_or(0)
                })
                .sum()
        })
        .unwrap_or(0)
}

/// AGT-16: `mixed_session` of the ftp module has a control flow and three
/// spawned data flows. The agent runs it as a scenario instance, so every
/// instance gives four client sessions: the control connection and one data
/// connection per transfer. The scenario needs the `spawn` action, which the
/// loader of single flow sessions refuses, and the `content` module through
/// `requires`.
#[test]
fn an_ftp_scenario_with_data_flows_runs_through_the_agent() {
    ftp_run();
}

fn ftp_run() {
    let mut plan = mail_plan(&[("ftp/mixed_session", 18_021)]);
    plan["mix"][0]["params"] = serde_json::json!({
        "server_port": 18_021,
        "server_ip": "127.82.0.1",
        "data_port_min": FTP_DATA_PORTS.0,
        "data_port_max": FTP_DATA_PORTS.1,
    });
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fps-agent-first-party");
    std::fs::create_dir_all(&dir).expect("plan dir");
    let path = dir.join("ftp.json");
    std::fs::write(&path, serde_json::to_string_pretty(&plan).unwrap()).expect("write plan");

    let (mut out, mut log) = (Vec::new(), Vec::new());
    let summary = run(&RunOptions::new(path, repo_modules()), &mut out, &mut log).expect("run");
    let log = String::from_utf8(log).unwrap();
    assert_eq!(summary.outcome, Outcome::Pass, "{log}");
    let last: Json = serde_json::from_str(
        String::from_utf8(out)
            .unwrap()
            .lines()
            .last()
            .expect("a snapshot line"),
    )
    .expect("JSON line");
    let ok = succeeded(&last, "ftp");
    // One full instance is the control connection and three data
    // connections, and the data connections count only inside an instance.
    assert!(ok >= 4, "{ok} ftp sessions succeeded\n{log}");
}

/// AGT-17: `interactive_shell` and `scp_upload` of the ssh module run a real
/// key exchange and an encrypted transport. A client session succeeds only when
/// both sides derived the same keys and every packet tag verified, so the test
/// shows that the agent gives each session side its own `Ssh` value: a value
/// that two sessions shared would carry the keys and the packet counters of
/// one session into the next and fail its first encrypted packet.
#[test]
fn ssh_scenarios_with_a_real_key_exchange_run_through_the_agent() {
    ssh_run();
}

fn ssh_run() {
    let mut plan = mail_plan(&[
        ("ssh/interactive_shell", 18_022),
        ("ssh/scp_upload", 18_023),
    ]);
    plan["mix"][0]["params"] = serde_json::json!({
        "server_port": 18_022,
        "commands": 1,
        "typed": 3,
        "think_min_ms": 0,
        "think_max_ms": 20,
        "key_gap_min_ms": 0,
        "key_gap_max_ms": 10,
        "motd_min": 100,
        "motd_max": 300,
        "output_min": 100,
        "output_max": 1000,
    });
    plan["mix"][1]["params"] = serde_json::json!({ "server_port": 18_023, "file_bytes": 65_536 });
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fps-agent-first-party");
    std::fs::create_dir_all(&dir).expect("plan dir");
    let path = dir.join("ssh.json");
    std::fs::write(&path, serde_json::to_string_pretty(&plan).unwrap()).expect("write plan");

    let (mut out, mut log) = (Vec::new(), Vec::new());
    let summary = run(&RunOptions::new(path, repo_modules()), &mut out, &mut log).expect("run");
    let log = String::from_utf8(log).unwrap();
    assert_eq!(summary.outcome, Outcome::Pass, "{log}");
    let out = String::from_utf8(out).unwrap();
    let last: Json =
        serde_json::from_str(out.lines().last().expect("a snapshot line")).expect("JSON line");
    // Both mix entries are ssh sessions, so the application count covers the
    // two scenarios. Two entries at 4 sessions per second for 3 seconds leave
    // more than one session of each, and a sharing bug fails every one after
    // the first.
    let ok = succeeded(&last, "ssh");
    assert!(ok >= 4, "{ok} ssh sessions succeeded\n{log}");
}

#[test]
fn smtp_imap_and_pop3_scenarios_run_through_the_agent() {
    mail_run();
}

fn mail_run() {
    let plan = mail_plan(&[
        ("smtp/relay", 18_825),
        ("imap/mailbox_sync", 18_143),
        ("pop3/retrieve_all", 18_110),
    ]);
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fps-agent-first-party");
    std::fs::create_dir_all(&dir).expect("plan dir");
    let path = dir.join("mail.json");
    std::fs::write(&path, serde_json::to_string_pretty(&plan).unwrap()).expect("write plan");

    let (mut out, mut log) = (Vec::new(), Vec::new());
    let summary = run(&RunOptions::new(path, repo_modules()), &mut out, &mut log).expect("run");
    let log = String::from_utf8(log).unwrap();
    assert_eq!(summary.outcome, Outcome::Pass, "{log}");
    let last: Json = serde_json::from_str(
        String::from_utf8(out)
            .unwrap()
            .lines()
            .last()
            .expect("a snapshot line"),
    )
    .expect("JSON line");
    for app in ["smtp", "imap", "pop3"] {
        assert!(
            succeeded(&last, app) > 0,
            "no {app} session succeeded\n{log}"
        );
    }
    assert!(
        log.contains("module content, which"),
        "the run did not note the required content module\n{log}"
    );
}

/// How often the repeat tests run their test, 50 unless `FPS_REPEAT` says
/// otherwise.
fn repeats() -> usize {
    std::env::var("FPS_REPEAT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
}

/// Runs `body` [`repeats`] times in a row and names the round that failed.
fn repeat(name: &str, body: fn()) {
    for round in 1..=repeats() {
        if let Err(e) = std::panic::catch_unwind(body) {
            eprintln!("{name}: round {round} of {} failed", repeats());
            std::panic::resume_unwind(e);
        }
    }
}

// AGT-18: each repeat test runs one of the tests above 50 times in a row, and
// the three of them run beside each other, as the tests above do in `cargo
// test`. They take about 3 seconds a round, so they are ignored by default:
// `cargo test -p fps-agent --test first_party -- --ignored repeat`. Do not run
// a repeat test beside the test that it repeats (`--include-ignored`): both use
// the same server ports and the same plan file, so the second one fails to bind.

#[test]
#[ignore = "repeats a three second run 50 times"]
fn repeat_the_ftp_test() {
    repeat("ftp", ftp_run);
}

#[test]
#[ignore = "repeats a three second run 50 times"]
fn repeat_the_ssh_test() {
    repeat("ssh", ssh_run);
}

#[test]
#[ignore = "repeats a three second run 50 times"]
fn repeat_the_mail_test() {
    repeat("mail", mail_run);
}
