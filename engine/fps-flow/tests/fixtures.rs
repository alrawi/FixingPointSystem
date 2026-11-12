// SPDX-License-Identifier: GPL-3.0-only
//! Rust half of the loader parity test of ENG-02.
//!
//! `test_fixtures.py` runs `tools/contracts/validate.py` on the same fixture
//! modules and the same `fixtures/verdicts.yaml`, so every verdict here is
//! also the verdict of the Python validator.

use std::path::{Path, PathBuf};

use fps_flow::program::Op;
use fps_flow::{LoadError, ModuleSet, Scenario};
use serde_yaml::Value as Yaml;

struct Case {
    scenario: String,
    accept: bool,
    error: Option<String>,
    pointer: Option<String>,
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn cases() -> Vec<Case> {
    let text = std::fs::read_to_string(fixtures().join("verdicts.yaml")).expect("verdicts.yaml");
    let doc: Yaml = serde_yaml::from_str(&text).expect("verdicts YAML");
    let field = |c: &Yaml, k: &str| c.get(k).and_then(Yaml::as_str).map(String::from);
    doc.get("cases")
        .and_then(Yaml::as_sequence)
        .expect("cases")
        .iter()
        .map(|c| Case {
            scenario: field(c, "scenario").expect("scenario"),
            accept: match field(c, "verdict").as_deref() {
                Some("accept") => true,
                Some("reject") => false,
                other => panic!("bad verdict {other:?}"),
            },
            error: field(c, "error"),
            pointer: field(c, "pointer"),
        })
        .collect()
}

fn modules() -> ModuleSet {
    ModuleSet::load_dir(&fixtures().join("modules")).expect("fixture manifests")
}

fn check(set: &ModuleSet, case: &Case) -> Result<Vec<fps_flow::Warning>, LoadError> {
    let module = case.scenario.split('/').next().expect("module");
    let ctx = set.context(module).expect("module context");
    Scenario::check_file(&fixtures().join("modules").join(&case.scenario), Some(&ctx))
}

#[test]
fn every_fixture_gets_its_shared_verdict() {
    let set = modules();
    let mut failures = Vec::new();
    for case in cases() {
        match (check(&set, &case), case.accept) {
            (Ok(_), true) => {}
            (Ok(_), false) => failures.push(format!("{}: accepted, want reject", case.scenario)),
            (Err(e), true) => failures.push(format!("{}: rejected with {e}", case.scenario)),
            (Err(e), false) => {
                if let Some(kind) = &case.error {
                    if e.kind() != kind {
                        failures.push(format!(
                            "{}: {} ({e}), want {kind}",
                            case.scenario,
                            e.kind()
                        ));
                    }
                }
                if let Some(pointer) = &case.pointer {
                    let got = e.location().map(|l| l.pointer()).unwrap_or_default();
                    if &got != pointer {
                        failures.push(format!("{}: pointer {got}, want {pointer}", case.scenario));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn every_fixture_is_listed_in_its_manifest_and_the_verdicts() {
    let set = modules();
    let mut listed: Vec<String> = cases().into_iter().map(|c| c.scenario).collect();
    let mut shipped: Vec<String> = set
        .manifests
        .values()
        .flat_map(|m| {
            m.scenarios
                .iter()
                .map(move |s| format!("{}/{}", m.name, s.display()))
        })
        .collect();
    listed.sort();
    shipped.sort();
    assert_eq!(listed, shipped);
}

#[test]
fn load_accepts_what_the_check_accepts_unless_unsupported() {
    let set = modules();
    for case in cases().into_iter().filter(|c| c.accept) {
        let module = case.scenario.split('/').next().expect("module");
        let ctx = set.context(module).expect("context");
        let path = fixtures().join("modules").join(&case.scenario);
        match Scenario::load_with(&path, Some(&ctx)) {
            Ok(_) => {}
            Err(LoadError::Unsupported { .. }) if case.scenario.contains("spawn") => {}
            Err(e) => panic!("{}: {e}", case.scenario),
        }
    }
}

#[test]
fn rejections_name_file_flow_side_and_action() {
    let set = modules();
    let case = Case {
        scenario: "fx/scenarios/last_after_loop.yaml".into(),
        accept: false,
        error: None,
        pointer: None,
    };
    let err = check(&set, &case).expect_err("last after a loop");
    let at = err.location().expect("location");
    assert!(at
        .file
        .as_deref()
        .is_some_and(|f| f.ends_with("last_after_loop.yaml")));
    assert_eq!(at.flow.as_ref().and_then(|f| f.id.as_deref()), Some("a"));
    assert_eq!(at.side, Some(fps_flow::Side::Client));
    assert_eq!(at.action_index(), Some(2));
    assert!(
        err.to_string()
            .ends_with("flow 'a': client: action 3: 'last' needs an expect that always runs earlier on the same side"),
        "{err}"
    );
}

#[test]
fn calls_into_a_module_without_manifest_warn() {
    let set = modules();
    let case = Case {
        scenario: "fx/scenarios/unchecked_module.yaml".into(),
        accept: true,
        error: None,
        pointer: None,
    };
    let warnings = check(&set, &case).expect("accepted");
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].message.contains("ghost"), "{}", warnings[0]);
    assert_eq!(warnings[0].at.pointer(), "/flows/0/client/1/send");
}

/// Loads a fixture scenario the way a runner does, in the module context of its directory.
fn load(scenario: &str) -> Result<Scenario, LoadError> {
    let set = modules();
    let module = scenario.split('/').next().expect("module");
    let ctx = set.context(module).expect("module context");
    Scenario::load_with(&fixtures().join("modules").join(scenario), Some(&ctx))
}

#[test]
fn a_strike_module_loads_raw_as_a_send_of_its_exact_bytes() {
    let s = load("st/scenarios/raw_ok.yaml").expect("strike module may use raw");
    let ops = &s.flows[0].client.ops;
    assert_eq!(ops.len(), 1, "{ops:?}");
    let Op::Send(t) = &ops[0] else {
        panic!("raw compiled to {:?}", ops[0]);
    };
    assert_eq!(t.as_static(), Some(&b"\r\n"[..]));
}

#[test]
fn a_raw_action_in_an_app_module_fails_to_load_with_a_named_error() {
    let err = load("fx/scenarios/raw_reserved.yaml").expect_err("raw in an app module");
    assert!(matches!(err, LoadError::ReservedAction { .. }), "{err:?}");
    assert_eq!(err.kind(), "ReservedAction");
    assert!(
        err.to_string()
            .ends_with("flow 'a': client: action 2: 'raw' is reserved for module types [\"fuzzer\", \"strike\"]"),
        "{err}"
    );
}
