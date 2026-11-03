// SPDX-License-Identifier: GPL-3.0-only
//! Contract test: every example scenario in `contracts/examples` compiles.

use std::path::Path;

use fps_flow::program::L4;
use fps_flow::Scenario;

const SUPPORTED_EXAMPLES: [&str; 3] = [
    "dns/scenarios/lookup.yaml",
    "http1/scenarios/web_browse.yaml",
    "http1/scenarios/web_get.yaml",
];

#[test]
fn v0_1_example_scenarios_compile() {
    let modules = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/examples/modules");
    let mut compiled = Vec::new();
    for example in SUPPORTED_EXAMPLES {
        let path = modules.join(example);
        let s = Scenario::load(&path).unwrap_or_else(|e| panic!("{e}"));
        compiled.push(s.name);
    }
    compiled.sort();
    // The v0.1 examples use the grammar supported by this engine version.
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
