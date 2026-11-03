// SPDX-License-Identifier: GPL-3.0-only
//! Contract tests that read the contract files directly.

use std::path::PathBuf;
use std::time::Duration;

use fps_sched::{
    Arrival, LoadProfile, ManualClock, Objective, Phase, Schedule, ScheduleConfig, Shape,
};
use serde_json::Value;

fn contracts() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contracts")
}

fn read_json(rel: &str) -> Value {
    let path = contracts().join(rel);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap()
}

#[test]
fn every_shape_of_the_test_configuration_parses() {
    let schema = read_json("schemas/test-config.schema.json");
    let shapes = schema["properties"]["load"]["properties"]["phases"]["items"]["properties"]
        ["shape"]["enum"]
        .as_array()
        .expect("shape enum");
    assert!(!shapes.is_empty());
    for name in shapes {
        let name = name.as_str().unwrap();
        Shape::from_contract_name(name, 1).unwrap();
    }
    assert!(Shape::from_contract_name("ramp", 1).is_err());
}

/// Converts one `LoadPhase` of the proto3 JSON mapping, where absent fields
/// hold their default value.
fn phase_from_plan(p: &Value) -> Phase {
    let duration = p["duration"].as_str().unwrap();
    let secs: f64 = duration.strip_suffix('s').unwrap().parse().unwrap();
    let steps = p["steps"].as_u64().unwrap_or(0) as u32;
    let shape = match p["shape"].as_str().unwrap() {
        "PHASE_SHAPE_HOLD" => Shape::Hold,
        "PHASE_SHAPE_LINEAR" => Shape::Linear,
        "PHASE_SHAPE_STEP" => Shape::Step { steps },
        other => panic!("unexpected shape {other}"),
    };
    Phase::new(
        p["name"].as_str().unwrap(),
        Duration::from_secs_f64(secs),
        shape,
        p["startFraction"].as_f64().unwrap_or(0.0),
        p["endFraction"].as_f64().unwrap_or(0.0),
    )
}

#[test]
fn the_example_plan_schedules_its_expected_counts() {
    let plan = read_json("examples/plans/http_dns_smoke.json");
    let phases: Vec<Phase> = plan["load"]["phases"]
        .as_array()
        .unwrap()
        .iter()
        .map(phase_from_plan)
        .collect();
    let profile = LoadProfile::new(phases).unwrap();
    assert_eq!(profile.total_secs(), 45.0);
    let seed: u64 = plan["seed"].as_str().unwrap().parse().unwrap();
    let rate = plan["workers"][0]["rates"][0]["sessionsPerSecond"]
        .as_f64()
        .unwrap();
    let mut cfg = ScheduleConfig::new(Objective::SessionsPerSecond(rate), profile, seed);
    cfg.arrival = Arrival::Constant;
    let mut sched = Schedule::new(cfg).unwrap();
    let starts: Vec<_> = sched.drain(&ManualClock::new(Duration::MAX), 0).collect();
    let per_phase = |i: usize| starts.iter().filter(|s| s.phase == i).count();
    // Linear 0 to 1 over 10 s, hold for 30 s, and 5 steps from 1 down over 5 s.
    assert_eq!(per_phase(0), (rate * 5.0) as usize);
    assert_eq!(per_phase(1), (rate * 30.0) as usize);
    assert_eq!(per_phase(2), (rate * 3.0) as usize);
}
