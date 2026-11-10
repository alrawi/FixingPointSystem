// SPDX-License-Identifier: GPL-3.0-only
//! The catalog that the crate implements is the `functions` list of the
//! module manifest, in every name, parameter name, order, type, and default, as
//! the contract section "Template function catalog" requires.
//!
//! MOD-05 ships the crate and MOD-06 ships `modules/http2/manifest.yaml`, so
//! the manifest does not exist yet. Until it does, the test checks the
//! catalog against the manifest loader of the engine through the text that
//! `catalog::manifest_functions` renders, and once the manifest exists the test
//! compares it with the catalog as the catalog tests of the other modules do.

use std::collections::HashSet;
use std::path::Path;

use fps_flow::function::NamedArg;
use fps_flow::{CallArgs, FnCall, FnErrorKind, Manifest, TemplateFn, ValueType};
use fps_module_http2::catalog::manifest_functions;
use fps_module_http2::{DefaultValue, Http2, FUNCTIONS};
use serde_yaml::Value as Yaml;

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

/// Compares a parsed `functions` list with the native catalog.
fn assert_matches_catalog(functions: &[Yaml]) {
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
fn the_rendered_catalog_parses_back_to_the_same_catalog() {
    let doc: Yaml = serde_yaml::from_str(&manifest_functions()).unwrap();
    assert_matches_catalog(doc["functions"].as_sequence().unwrap());
}

#[test]
fn the_engine_loader_reads_the_rendered_catalog() {
    let text = format!("name: http2\ntype: app\n{}", manifest_functions());
    let m = Manifest::parse(&text).unwrap();
    assert_eq!(m.name, "http2");
    assert_eq!(m.functions.len(), FUNCTIONS.len());
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
fn the_manifest_of_the_module_equals_the_catalog_once_it_exists() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("manifest.yaml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("{} does not exist yet, MOD-06 adds it", path.display());
        return;
    };
    let doc: Yaml = serde_yaml::from_str(&text).unwrap();
    assert_matches_catalog(doc["functions"].as_sequence().unwrap());
    let loaded = Manifest::load(&path).unwrap();
    assert_eq!(loaded.name, "http2");
}

#[test]
fn the_catalog_is_well_formed() {
    let mut names = HashSet::new();
    for spec in FUNCTIONS {
        assert!(names.insert(spec.name), "{} is listed twice", spec.name);
        assert!(!spec.summary.is_empty(), "{} has no summary", spec.name);
        let mut params = HashSet::new();
        let mut optional = false;
        for p in spec.params {
            assert!(
                params.insert(p.name),
                "{}.{} is listed twice",
                spec.name,
                p.name
            );
            // The engine binds positional arguments in order, so a required
            // parameter behind an optional one could never be given.
            assert!(
                !(optional && p.default.is_none()),
                "{}.{} is required behind an optional parameter",
                spec.name,
                p.name
            );
            optional |= p.default.is_some();
        }
    }
}

#[test]
fn every_catalog_function_is_implemented() {
    for spec in FUNCTIONS {
        let call = FnCall {
            module: "http2",
            function: spec.name,
            args: CallArgs {
                positional: &[],
                named: &[] as &[NamedArg<'_>],
            },
        };
        let mut rng = rand_pcg::Pcg64Mcg::new(1);
        let result = Http2::default().render(&call, &mut rng, &mut Vec::new());
        // A call without arguments either renders, or fails because it lacks
        // a required argument, and never because the function is unknown.
        match result {
            Ok(()) => assert!(
                spec.params.iter().all(|p| p.default.is_some()),
                "{}",
                spec.name
            ),
            Err(e) => assert_eq!(e.kind, FnErrorKind::InvalidArgument, "{}: {e}", spec.name),
        }
    }
}
