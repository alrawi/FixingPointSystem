// SPDX-License-Identifier: GPL-3.0-only
//! Contract test that compares the static catalog of `fps_stats::catalog`
//! with `contracts/metrics/catalog.yaml`, and checks that every metric name a
//! snapshot emits is a catalog name.
//!
//! The test reads the YAML file directly, as rule 10 of the coding standards asks.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use fps_stats::catalog::{label_values, LABEL_NAMES};
use fps_stats::{MetricId, MetricKind, Shard, Snapshot, SnapshotMeta, CATALOG};
use serde::Deserialize;

#[derive(Deserialize)]
struct Catalog {
    labels: BTreeMap<String, Label>,
    metrics: Vec<Metric>,
}

#[derive(Deserialize)]
struct Label {
    values: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct Metric {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    unit: String,
    labels: Vec<String>,
}

fn catalog() -> Catalog {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts/metrics/catalog.yaml");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_yaml::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

#[test]
fn table_matches_the_catalog_file() {
    let yaml = catalog();
    let from_yaml: Vec<(String, String, String, Vec<String>)> = yaml
        .metrics
        .iter()
        .map(|m| {
            (
                m.name.clone(),
                m.kind.clone(),
                m.unit.clone(),
                m.labels.clone(),
            )
        })
        .collect();
    let from_rust: Vec<(String, String, String, Vec<String>)> = CATALOG
        .iter()
        .map(|m| {
            (
                m.name.to_owned(),
                m.kind.as_str().to_owned(),
                m.unit.to_owned(),
                m.labels.iter().map(|l| (*l).to_owned()).collect(),
            )
        })
        .collect();
    assert_eq!(from_rust, from_yaml);
    assert_eq!(CATALOG.len(), 55);
}

#[test]
fn label_values_match_the_catalog_file() {
    let yaml = catalog();
    let names: Vec<&str> = yaml.labels.keys().map(String::as_str).collect();
    let mut rust_names: Vec<&str> = LABEL_NAMES.to_vec();
    rust_names.sort_unstable();
    assert_eq!(rust_names, names);
    for (name, label) in &yaml.labels {
        let rust = label_values(name).map(|v| v.iter().map(|s| (*s).to_owned()).collect());
        assert_eq!(rust, label.values, "values of label {name}");
    }
}

#[test]
fn metric_constants_and_lookup_agree() {
    for (i, def) in CATALOG.iter().enumerate() {
        let id = MetricId::from_name(def.name).unwrap();
        assert_eq!(id.def(), def);
        assert_eq!(MetricId::all().nth(i), Some(id));
    }
    assert_eq!(MetricId::from_name("sessions.unknown"), None);
    assert_eq!(fps_stats::metric::HOST_ACTIVE.name(), "host.active");
    assert_eq!(fps_stats::metric::MIX_SHARE_PCT.name(), "mix.share_pct");
}

/// Builds a shard with one series of every metric that a shard can record,
/// with every catalog label set, and returns the metric names of its snapshot.
#[test]
fn snapshot_names_match_the_catalog() {
    let yaml = catalog();
    let catalog_names: BTreeSet<String> = yaml.metrics.iter().map(|m| m.name.clone()).collect();

    let mut shard = Shard::new(3);
    let mut recordable = BTreeSet::new();
    for id in MetricId::all() {
        let def = id.def();
        let labels: Vec<(&str, &str)> = def
            .labels
            .iter()
            .map(|l| {
                (
                    *l,
                    label_values(l)
                        .and_then(|v| v.first().copied())
                        .unwrap_or("x"),
                )
            })
            .collect();
        match def.kind {
            MetricKind::Counter => {
                let c = shard.counter(id, &labels).unwrap();
                shard.add(c, 2);
            }
            MetricKind::Gauge if def.merges_by_sum() => {
                let g = shard.gauge(id, &labels).unwrap();
                shard.set(g, 1.5);
            }
            MetricKind::Gauge => {
                assert!(shard.gauge(id, &labels).is_err(), "{}", def.name);
                continue;
            }
            MetricKind::Histogram => {
                let h = shard.histogram(id, &labels).unwrap();
                shard.record(h, 42);
            }
        }
        recordable.insert(def.name.to_owned());
    }

    let merged = Snapshot::merge_all([&shard.snapshot(), &Shard::new(4).snapshot()]).unwrap();
    let json = merged.to_proto_json(&SnapshotMeta::default()).unwrap();
    let mut emitted = BTreeSet::new();
    for list in ["counters", "histograms"] {
        for entry in json[list].as_array().unwrap() {
            let name = entry["metric"].as_str().unwrap().to_owned();
            assert!(
                catalog_names.contains(&name),
                "{name} is not in the catalog"
            );
            emitted.insert(name);
        }
    }
    assert_eq!(emitted, recordable);

    // Only the derived ratios and percentages are left out of a shard.
    let derived: BTreeSet<String> = yaml
        .metrics
        .iter()
        .filter(|m| {
            m.kind == "gauge"
                && ["ratio", "percent", "percentage_points"].contains(&m.unit.as_str())
        })
        .map(|m| m.name.clone())
        .collect();
    let all: BTreeSet<String> = emitted.union(&derived).cloned().collect();
    assert_eq!(all, catalog_names);
}
