// SPDX-License-Identifier: GPL-3.0-only
//! Contract test: every example scenario in `contracts/examples` compiles, and
//! its compiled programs match the golden dumps under `tests/testdata/examples/`.

use std::path::{Path, PathBuf};

use fps_flow::program::{Op, L4};
use fps_flow::Scenario;

fn examples() -> Vec<(String, PathBuf)> {
    let modules = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/examples/modules");
    let mut out = Vec::new();
    for module in std::fs::read_dir(&modules).expect("examples directory") {
        let module = module.expect("module entry").path();
        let Ok(scenarios) = std::fs::read_dir(module.join("scenarios")) else {
            continue;
        };
        for entry in scenarios {
            let path = entry.expect("scenario entry").path();
            if path.extension().is_some_and(|e| e == "yaml") {
                let name = format!(
                    "{}.{}",
                    module.file_name().expect("module name").to_string_lossy(),
                    path.file_stem().expect("scenario name").to_string_lossy()
                );
                out.push((name, path));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn every_example_scenario_compiles_to_its_golden_dump() {
    let examples = examples();
    // The v0.2 examples include module calls with keyword arguments and `last`.
    for name in [
        "dns.lookup",
        "dns.resolve",
        "http1.web_browse",
        "http1.web_get",
        "http1.web_page",
    ] {
        assert!(examples.iter().any(|(n, _)| n == name), "{name} missing");
    }
    for (name, path) in examples {
        let mut s = Scenario::load(&path).unwrap_or_else(|e| panic!("{e}"));
        let mut dump = String::from("== compiled\n");
        dump.push_str(&s.dump());
        s.fold_constants().unwrap_or_else(|e| panic!("{name}: {e}"));
        dump.push_str("== folded\n");
        dump.push_str(&s.dump());
        fps_golden::assert_golden!(format!("examples/{name}.dump"), dump);
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
        Op::Send(t) => assert!(t.is_static()),
        other => panic!("first client op is {other:?}"),
    }
}

#[test]
fn dns_resolve_keeps_set_targets_as_references() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/examples/modules/dns/scenarios/resolve.yaml");
    let mut s = Scenario::load(&path).expect("resolve.yaml");
    assert_eq!(
        s.set_targets().into_iter().collect::<Vec<_>>(),
        ["server_ip"]
    );
    s.fold_constants().expect("fold");
    let client = &s.flows[0].client;
    assert_eq!(client.port.as_static(), Some(&b"53"[..]));
    match &client.ops[0] {
        Op::Send(t) => {
            assert!(t.variables().is_empty());
            let calls = t.module_calls();
            assert_eq!(calls[0].module, "dns");
            assert_eq!(calls[0].keywords, ["name", "qtype"]);
        }
        other => panic!("first client op is {other:?}"),
    }
    match &client.ops[2] {
        Op::Set(binds) => assert!(binds[0].1.uses_last()),
        other => panic!("third client op is {other:?}"),
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

#[test]
fn every_example_passes_the_checks_with_its_manifest() {
    let modules = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/examples/modules");
    let set = fps_flow::ModuleSet::load_dir(&modules).expect("example manifests");
    let mut checked = 0;
    for manifest in set.manifests.values() {
        let ctx = set.context(&manifest.name).expect("context");
        for rel in &manifest.scenarios {
            let path = modules.join(&manifest.name).join(rel);
            let warnings =
                Scenario::check_file(&path, Some(&ctx)).unwrap_or_else(|e| panic!("{e}"));
            assert!(warnings.is_empty(), "{warnings:?}");
            Scenario::load_with(&path, Some(&ctx)).unwrap_or_else(|e| panic!("{e}"));
            checked += 1;
        }
    }
    assert_eq!(checked, 5);
}
