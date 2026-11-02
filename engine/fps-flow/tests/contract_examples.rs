// SPDX-License-Identifier: GPL-3.0-only
//! Contract test: every example scenario in `contracts/examples` compiles.

use std::path::Path;

use fps_flow::program::L4;
use fps_flow::Scenario;

#[test]
fn every_example_scenario_compiles() {
    let modules = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/examples/modules");
    let mut compiled = Vec::new();
    for module in std::fs::read_dir(&modules).expect("modules directory") {
        let scenarios = module.expect("entry").path().join("scenarios");
        let Ok(entries) = std::fs::read_dir(&scenarios) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("entry").path();
            if path.extension().is_some_and(|e| e == "yaml") {
                let s = Scenario::load(&path).unwrap_or_else(|e| panic!("{e}"));
                compiled.push(s.name);
            }
        }
    }
    compiled.sort();
    // The examples of CON-01 at the time of ENG-01. A new example joins the
    // check on its own, so this list only guards against an empty walk.
    for name in ["lookup", "web_browse", "web_get"] {
        assert!(compiled.iter().any(|n| n == name), "{name} in {compiled:?}");
    }
}

#[test]
fn dns_lookup_records_udp_and_hex_payloads() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/examples/modules/dns/scenarios/lookup.yaml");
    let s = Scenario::load(&path).expect("lookup.yaml");
    let flow = &s.flows[0];
    assert_eq!(flow.l4, L4::Udp);
    match &flow.client.ops[0] {
        fps_flow::program::Op::Send(t) => assert!(t.is_static()),
        other => panic!("first client op is {other:?}"),
    }
}

#[test]
fn load_error_names_the_file() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/testdata");
    let err = Scenario::load(&dir.join("bad_close.yaml")).expect_err("bad close mode");
    let text = err.to_string();
    assert!(
        text.contains("bad_close.yaml: flow 'a': client: action 2"),
        "{text}"
    );
}
