// SPDX-License-Identifier: GPL-3.0-only
//! Contract test that the failure reasons match the `reason` label of the
//! metric catalog, so a catalog change fails here until the code follows.

use std::path::Path;

use fps_interp::{Reason, Side};
use serde_yaml::Value;

fn catalog() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/metrics/catalog.yaml");
    let text = std::fs::read_to_string(&path).unwrap();
    serde_yaml::from_str(&text).unwrap()
}

fn label_values(doc: &Value, label: &str) -> Vec<String> {
    doc["labels"][label]["values"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn reasons_match_the_catalog_in_order() {
    let ours: Vec<&str> = Reason::ALL.iter().map(|r| r.as_str()).collect();
    assert_eq!(ours, label_values(&catalog(), "reason"));
}

#[test]
fn sides_and_action_types_are_catalog_values() {
    let doc = catalog();
    let sides = label_values(&doc, "side");
    for side in [Side::Client, Side::Server] {
        assert!(sides.iter().any(|s| s == side.as_str()));
    }
    let types = label_values(&doc, "action_type");
    for t in [
        "connect", "accept", "send", "expect", "delay", "set", "loop", "branch", "close",
    ] {
        assert!(types.iter().any(|s| s == t), "{t}");
    }
}
