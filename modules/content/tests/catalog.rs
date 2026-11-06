// SPDX-License-Identifier: GPL-3.0-only
//! The catalog that the crate implements equals the `functions` list of the
//! manifest in every name, parameter name, order, type, and default, as the
//! contract section "Template function catalog" requires. The manifest also
//! carries the provenance note that every generator is synthetic.

use std::path::{Path, PathBuf};

use fps_flow::{Manifest, ValueType};
use fps_module_content::{DefaultValue, FUNCTIONS};
use serde_yaml::Value as Yaml;

fn module_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn type_name(t: ValueType) -> &'static str {
    match t {
        ValueType::Int => "int",
        ValueType::Float => "float",
        ValueType::Boolean => "boolean",
        ValueType::Text => "text",
        ValueType::Bytes => "bytes",
        _ => "any",
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn default_matches(d: DefaultValue, y: &Yaml) -> bool {
    match d {
        DefaultValue::Text(t) => y.as_str() == Some(t),
        DefaultValue::Int(i) => y.as_i64() == Some(i),
        DefaultValue::Boolean(b) => y.as_bool() == Some(b),
        DefaultValue::Bytes(b) => y.as_str().map(str::to_ascii_lowercase) == Some(hex(b)),
    }
}

#[test]
fn the_manifest_catalog_equals_the_native_catalog() {
    let text = std::fs::read_to_string(module_dir().join("manifest.yaml")).unwrap();
    let doc: Yaml = serde_yaml::from_str(&text).unwrap();
    let functions = doc["functions"].as_sequence().unwrap();
    let names: Vec<&str> = functions
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    let native: Vec<&str> = FUNCTIONS.iter().map(|f| f.name).collect();
    assert_eq!(names, native);
    for (f, spec) in functions.iter().zip(FUNCTIONS) {
        let params = f["params"].as_sequence().unwrap();
        assert_eq!(params.len(), spec.params.len(), "{}", spec.name);
        for (p, ps) in params.iter().zip(spec.params) {
            let what = format!("{}.{}", spec.name, ps.name);
            assert_eq!(p["name"].as_str(), Some(ps.name), "{what}");
            assert_eq!(p["type"].as_str(), Some(type_name(ps.ty)), "{what}");
            match (p.get("default"), ps.default) {
                (None, None) => {}
                (Some(y), Some(d)) => assert!(default_matches(d, y), "{what} default"),
                (y, d) => panic!("{what}: manifest default {y:?}, native default {d:?}"),
            }
        }
    }
}

#[test]
fn the_loader_reads_the_same_signatures() {
    let m = Manifest::load(&module_dir().join("manifest.yaml")).unwrap();
    assert_eq!(m.name, "content");
    for spec in FUNCTIONS {
        let sig = &m.functions[spec.name];
        let loaded: Vec<(&str, ValueType, bool)> = sig
            .params
            .iter()
            .map(|p| (p.name.as_str(), p.ty, p.required))
            .collect();
        let native: Vec<(&str, ValueType, bool)> = spec
            .params
            .iter()
            .map(|p| (p.name, p.ty, p.default.is_none()))
            .collect();
        assert_eq!(loaded, native, "{}", spec.name);
    }
}

#[test]
fn the_manifest_and_the_readme_state_that_the_content_is_synthetic() {
    let text = std::fs::read_to_string(module_dir().join("manifest.yaml")).unwrap();
    let doc: Yaml = serde_yaml::from_str(&text).unwrap();
    assert_eq!(doc["provenance"]["source"].as_str(), Some("synthetic"));
    let readme = std::fs::read_to_string(module_dir().join("README.md")).unwrap();
    assert!(readme.contains("## Provenance"));
    assert!(readme.contains("synthetic"));
    assert!(readme.contains("no real personal data"));
}
