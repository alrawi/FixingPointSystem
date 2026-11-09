// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of the standalone mode over loopback addresses.
//!
//! The tests derive plans from `contracts/examples/plans/http_dns_smoke.json`.
//! An unprivileged test cannot bind port 53, and the example pools are not
//! local addresses, so the derived plans move the pools into 127.0.0.0/8,
//! move the scenario ports above 1024, recompute the scenario digests, and
//! shorten the load phases. The module trees and their digests stay as the
//! example has them.

use std::path::{Path, PathBuf};
use std::time::Duration;

use fps_agent::assemble::Sides;
use fps_agent::modules::{sha256_digest, tree_digest};
use fps_agent::{run, Outcome, RunError, RunOptions};
use serde_json::Value as Json;

fn contracts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts")
}

fn example() -> Json {
    let text = std::fs::read_to_string(contracts().join("examples/plans/http_dns_smoke.json"))
        .expect("example plan");
    serde_json::from_str(&text).expect("example plan JSON")
}

/// Returns the example plan on loopback pools `127.<c>.0.0/24` and
/// `127.<s>.0.0/24`, with the given scenario ports and two one second
/// phases at a low rate.
fn loopback_plan(c: u8, s: u8, http_port: u16, dns_port: u16) -> Json {
    let mut plan = example();
    for sc in plan["scenarios"].as_array_mut().expect("scenarios") {
        let src = sc["source"].as_str().expect("source");
        let src = src
            .replace("server_port: 8080", &format!("server_port: {http_port}"))
            .replace("server_port: 53", &format!("server_port: {dns_port}"));
        sc["digest"] = sha256_digest(src.as_bytes()).into();
        sc["source"] = src.into();
    }
    plan["hostPools"][0]["ipv4Prefix"] = format!("127.{c}.0.0/24").into();
    plan["hostPools"][1]["ipv4Prefix"] = format!("127.{s}.0.0/24").into();
    for w in plan["workers"].as_array_mut().expect("workers") {
        w["rates"][0]["sessionsPerSecond"] = 40.into();
        w["rates"][1]["sessionsPerSecond"] = 10.into();
    }
    plan["load"]["phases"] = serde_json::json!([
        { "name": "ramp_up", "duration": "2s", "shape": "PHASE_SHAPE_LINEAR", "endFraction": 1 },
        { "name": "steady", "duration": "1s", "shape": "PHASE_SHAPE_HOLD",
          "startFraction": 1, "endFraction": 1 }
    ]);
    plan["warmup"] = "1s".into();
    plan
}

fn write_plan(name: &str, plan: &Json) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fps-agent-standalone");
    std::fs::create_dir_all(&dir).expect("plan dir");
    let path = dir.join(name);
    std::fs::write(
        &path,
        serde_json::to_string_pretty(plan).expect("plan text"),
    )
    .expect("write plan");
    path
}

fn options(plan: &Path) -> RunOptions {
    RunOptions::new(plan.to_path_buf(), contracts().join("examples/modules"))
}

/// Sums the counters of `metric` and `application` on `side` of one snapshot
/// line. A line holds both sides, so a sum without the side would count each
/// session twice.
fn sum(line: &Json, metric: &str, application: &str, side: &str) -> u64 {
    line["counters"]
        .as_array()
        .map(|cs| {
            cs.iter()
                .filter(|c| {
                    c["metric"] == metric
                        && c["labels"]["application"] == application
                        && c["labels"]["side"] == side
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

#[test]
fn the_example_module_trees_match_the_example_plan_digests() {
    let plan = example();
    for m in plan["modules"].as_array().expect("modules") {
        let name = m["name"].as_str().expect("name");
        let got = tree_digest(name, &contracts().join("examples/modules").join(name)).unwrap();
        assert_eq!(got, m["digest"].as_str().unwrap(), "{name}");
    }
}

#[test]
fn the_example_plan_runs_both_arms_on_loopback_and_passes() {
    let plan = write_plan("both.json", &loopback_plan(71, 72, 18_080, 15_353));
    let (mut out, mut log) = (Vec::new(), Vec::new());
    let summary = run(&options(&plan), &mut out, &mut log).expect("run");
    let log = String::from_utf8(log).unwrap();
    assert_eq!(summary.outcome, Outcome::Pass, "{log}");
    let lines: Vec<Json> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).expect("JSON line"))
        .collect();
    // Three seconds of load give a line at one and two seconds and the
    // final one.
    assert!(lines.len() >= 3, "{} lines\n{log}", lines.len());
    assert_eq!(lines.len() as u64, summary.lines);
    for (i, l) in lines.iter().enumerate() {
        assert_eq!(l["planId"], "run-0001/agent-a");
        assert_eq!(l["sequence"], (i + 1).to_string());
        assert_eq!(l["aggregate"], true);
        assert_eq!(
            l["isFinal"].as_bool().unwrap_or(false),
            i + 1 == lines.len()
        );
    }
    assert_eq!(lines[0]["phase"], "ramp_up");
    let last = lines.last().unwrap();
    let (http, dns) = (
        sum(last, "sessions.succeeded", "http1", "client"),
        sum(last, "sessions.succeeded", "dns", "client"),
    );
    assert!(http > 0 && dns > 0, "http1 {http} dns {dns}\n{log}");
    assert_eq!(sum(last, "sessions.attempted", "http1", "client"), http);
    // The line holds the server series next to the client series, and both
    // sides ran every session that completed.
    assert_eq!(sum(last, "sessions.succeeded", "http1", "server"), http);
    assert_eq!(sum(last, "sessions.succeeded", "dns", "server"), dns);
    assert!(
        last["counters"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["labels"].get("side").is_some()),
        "a series lacks the side label: {last}"
    );
    let client = summary.client.expect("client totals");
    let server = summary.server.expect("server totals");
    let m = fps_stats::metric::SESSIONS_SUCCEEDED;
    assert_eq!(client.counter_total(m), http + dns);
    assert_eq!(server.counter_total(m), http + dns, "{log}");
    assert!(log.contains("verdict pass"), "{log}");
    assert_eq!(summary.criteria.len(), 1);
}

#[test]
fn criteria_count_the_client_side_unless_the_process_runs_none() {
    let mut plan = loopback_plan(79, 80, 18_084, 15_357);
    // The criteria compare the measured values with the client totals, which
    // are half of what a total over both sides would give.
    plan["criteria"] = serde_json::json!([
        { "metric": "sessions.attempted", "op": "CRITERION_OP_GT", "value": 0 },
        { "metric": "sessions.failed", "op": "CRITERION_OP_EQ", "value": 0 }
    ]);
    plan["warmup"] = "0s".into();
    let path = write_plan("client-default.json", &plan);
    let (mut out, mut log) = (Vec::new(), Vec::new());
    let summary = run(&options(&path), &mut out, &mut log).expect("run");
    let log = String::from_utf8(log).unwrap();
    assert_eq!(summary.outcome, Outcome::Pass, "{log}");
    let client = summary.client.expect("client totals");
    let server = summary.server.expect("server totals");
    let attempted = fps_stats::metric::SESSIONS_ATTEMPTED;
    assert!(client.counter_total(attempted) > 0);
    assert_eq!(
        summary.criteria[0].value,
        client.counter_total(attempted) as f64,
        "the criterion counts the client side only, the server side has {}",
        server.counter_total(attempted)
    );

    // A process without client workers has no client series, so its criteria
    // count the server side, which saw no session here.
    let path = write_plan("server-default.json", &plan);
    let mut opts = options(&path);
    opts.sides = Sides::Server;
    opts.linger = Duration::from_millis(500);
    let summary = run(&opts, &mut Vec::new(), &mut Vec::new()).expect("run");
    assert!(summary.client.is_none());
    assert_eq!(summary.criteria[0].value, 0.0);
    assert!(!summary.criteria[0].holds);
}

/// Copies the first party dns module and a zone file that answers
/// `www.corp.test` into a module directory of its own, and returns the
/// directory with the `ModuleRef` of the copy.
fn corp_dns_modules(tag: &str) -> (PathBuf, fps_agent::plan::ModuleRef) {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            let target = to.join(e.file_name());
            if e.file_type().unwrap().is_dir() {
                copy(&e.path(), &target);
            } else {
                std::fs::copy(e.path(), target).unwrap();
            }
        }
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("fps-agent-dns-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    copy(&contracts().join("../modules/dns"), &dir.join("dns"));
    std::fs::write(
        dir.join("dns/zones/corp.zone"),
        "$ORIGIN corp.test.\n$TTL 60\n@ IN SOA ns hostmaster 1 7200 3600 1209600 60\n@ IN NS ns\nns IN A 192.0.2.7\nwww IN A 192.0.2.99\n",
    )
    .unwrap();
    let module = fps_agent::modules::installed(&dir)
        .into_iter()
        .find(|m| m.name == "dns")
        .expect("dns module");
    (dir, module)
}

/// A plan with the dns scenario `lookup_a` of the copied module, which asks
/// for `www.corp.test`, with the given module settings.
fn corp_dns_plan(dir: &Path, module: &fps_agent::plan::ModuleRef, settings: Option<Json>) -> Json {
    let mut plan = loopback_plan(81, 82, 18_085, 15_358);
    let source = std::fs::read_to_string(dir.join("dns/scenarios/lookup_a.yaml")).unwrap();
    let mut entry = serde_json::json!({
        "name": "dns", "version": module.version, "digest": module.digest,
    });
    if let Some(s) = settings {
        entry["settings"] = s;
    }
    plan["modules"] = serde_json::json!([entry]);
    plan["scenarios"] = serde_json::json!([{
        "ref": "dns/lookup_a", "module": "dns",
        "source": source, "digest": sha256_digest(source.as_bytes()),
    }]);
    plan["mix"] = serde_json::json!([{
        "id": "lookup", "scenario": "dns/lookup_a", "weight": 1,
        "params": { "host": "www.corp.test", "server_port": 15_358 },
        "clientPool": "clients", "serverPool": "servers",
    }]);
    for w in plan["workers"].as_array_mut().unwrap() {
        w["rates"] = serde_json::json!([{ "mixEntry": "lookup", "sessionsPerSecond": 20 }]);
    }
    plan["criteria"] = serde_json::json!([
        { "metric": "sessions.succeeded", "op": "CRITERION_OP_GT", "value": 0 },
        { "metric": "sessions.failed_ratio", "op": "CRITERION_OP_EQ", "value": 0 }
    ]);
    plan
}

#[test]
fn the_dns_zone_file_setting_of_a_plan_selects_the_zone_that_answers() {
    let (dir, module) = corp_dns_modules("zone");
    let opts = |plan: &Path| RunOptions::new(plan.to_path_buf(), dir.clone());

    let configured = corp_dns_plan(
        &dir,
        &module,
        Some(serde_json::json!({"zone_file": "zones/corp.zone"})),
    );
    let path = write_plan("dns-zone.json", &configured);
    let (mut out, mut log) = (Vec::new(), Vec::new());
    let summary = run(&opts(&path), &mut out, &mut log).expect("run");
    let log = String::from_utf8(log).unwrap();
    assert_eq!(summary.outcome, Outcome::Pass, "{log}");
    let ok = summary.client.expect("client totals");
    assert!(
        ok.counter_total(fps_stats::metric::SESSIONS_SUCCEEDED) > 0,
        "{log}"
    );

    // Without the setting the shipped zone answers, which has no corp.test
    // name, so the same plan fails. The setting made the difference.
    let path = write_plan("dns-bundled.json", &corp_dns_plan(&dir, &module, None));
    let summary = run(&opts(&path), &mut Vec::new(), &mut Vec::new()).expect("run");
    assert_eq!(summary.outcome, Outcome::Fail);
    let bad = summary.client.expect("client totals");
    assert_eq!(bad.counter_total(fps_stats::metric::SESSIONS_SUCCEEDED), 0);
    assert!(bad.counter_total(fps_stats::metric::SESSIONS_FAILED) > 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_setting_that_leaves_the_module_directory_or_names_no_file_ends_the_run() {
    let (dir, module) = corp_dns_modules("escape");
    std::fs::write(dir.join("outside.zone"), "x").unwrap();
    for (zone, want) in [
        ("../outside.zone", "has a .. component"),
        ("zones/../../outside.zone", "has a .. component"),
        ("/etc/hostname", "outside the module directory"),
        ("zones/missing.zone", "missing.zone"),
    ] {
        let plan = corp_dns_plan(
            &dir,
            &module,
            Some(serde_json::json!({ "zone_file": zone })),
        );
        let path = write_plan("dns-bad.json", &plan);
        let err = run(
            &RunOptions::new(path, dir.clone()),
            &mut Vec::new(),
            &mut Vec::new(),
        )
        .unwrap_err();
        let text = format!("{err:#}");
        assert!(matches!(err, RunError::Module(_)), "{zone}: {err}");
        assert!(
            text.contains("settings") && text.contains(want),
            "{zone}: {text}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_criterion_gives_fail_and_a_server_only_run_reports_the_server_side() {
    let mut plan = loopback_plan(73, 74, 18_081, 15_354);
    plan["criteria"] = serde_json::json!([
        { "metric": "sessions.attempted", "op": "CRITERION_OP_GT", "value": 1e9 }
    ]);
    let path = write_plan("fail.json", &plan);
    let (mut out, mut log) = (Vec::new(), Vec::new());
    let summary = run(&options(&path), &mut out, &mut log).expect("run");
    assert_eq!(summary.outcome, Outcome::Fail);

    let path = write_plan("server.json", &loopback_plan(75, 76, 18_082, 15_355));
    let mut opts = options(&path);
    opts.sides = Sides::Server;
    opts.linger = Duration::ZERO;
    let (mut out, mut log) = (Vec::new(), Vec::new());
    let summary = run(&opts, &mut out, &mut log).expect("run");
    assert!(summary.client.is_none());
    assert_eq!(
        summary
            .server
            .expect("server totals")
            .counter_total(fps_stats::metric::SESSIONS_ATTEMPTED),
        0
    );
    // A ratio over zero sessions is undefined, so the criterion fails.
    assert_eq!(summary.outcome, Outcome::Fail);
}

#[test]
fn a_changed_module_tree_or_scenario_is_an_error() {
    let mut plan = loopback_plan(77, 78, 18_083, 15_356);
    plan["modules"][0]["digest"] =
        "sha256:0000000000000000000000000000000000000000000000000000000000000000".into();
    let path = write_plan("digest.json", &plan);
    let err = run(&options(&path), &mut Vec::new(), &mut Vec::new()).unwrap_err();
    assert!(
        matches!(err, RunError::Module(_)) && err.to_string().contains("digest"),
        "{err}"
    );

    let mut plan = loopback_plan(77, 78, 18_083, 15_356);
    plan["scenarios"][1]["source"] = plan["scenarios"][1]["source"]
        .as_str()
        .unwrap()
        .replace("requests: 4", "requests: 5")
        .into();
    let path = write_plan("scenario.json", &plan);
    let err = run(&options(&path), &mut Vec::new(), &mut Vec::new()).unwrap_err();
    assert!(err.to_string().contains("scenario http1/web_get"), "{err}");

    let mut plan = loopback_plan(77, 78, 18_083, 15_356);
    plan["modules"][1]["version"] = "9.9.9".into();
    let path = write_plan("version.json", &plan);
    let err = run(&options(&path), &mut Vec::new(), &mut Vec::new()).unwrap_err();
    assert!(err.to_string().contains("version"), "{err}");

    let mut plan = loopback_plan(77, 78, 18_083, 15_356);
    plan["backend"] = "BACKEND_DPDK".into();
    let path = write_plan("backend.json", &plan);
    let err = run(&options(&path), &mut Vec::new(), &mut Vec::new()).unwrap_err();
    assert!(matches!(err, RunError::Unimplemented(_)), "{err}");
}
