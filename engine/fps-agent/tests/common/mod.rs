// SPDX-License-Identifier: GPL-3.0-only
//! Plans for the tests of the kernel fallback (AGT-29), shared by
//! `tests/fallback.rs` and `tests/session.rs`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use fps_agent::modules::{sha256_digest, tree_digest};
use serde_json::{json, Value as Json};

pub fn contracts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts")
}

/// The example plan of the agent protocol for AF_XDP: two ports, two pools,
/// http1 and dns, in the shape that the datapath serves.
pub fn served_plan() -> Json {
    let text = std::fs::read_to_string(contracts().join("examples/plans/http_dns_smoke.json"))
        .expect("example plan");
    let mut plan: Json = serde_json::from_str(&text).expect("example plan JSON");
    plan["backend"] = "BACKEND_AF_XDP".into();
    plan
}

/// Turns a test configuration of `contracts/examples/tests/` into a plan for
/// one agent that the controller selected AF_XDP for.
pub fn plan_of(test: &str) -> Json {
    plan_in(test, &contracts().join("examples/modules"))
}

/// The module tree that holds every scenario of a test configuration: the
/// example modules of the contracts, or else the first party modules of the
/// repository (`dns/lookup_a` and `http1/single_fetch` live only there).
pub fn modules_of(test: &str) -> PathBuf {
    let text = std::fs::read_to_string(contracts().join("examples/tests").join(test))
        .expect("test configuration");
    let config: serde_norway::Value = serde_norway::from_str(&text).expect("test YAML");
    let refs: Vec<&str> = config["profile"]["mix"]
        .as_sequence()
        .expect("mix")
        .iter()
        .map(|e| e["scenario"].as_str().expect("scenario"))
        .collect();
    let first_party = contracts().join("../modules");
    for dir in [contracts().join("examples/modules"), first_party] {
        let has = |r: &&str| {
            let (module, name) = r.split_once('/').expect("module/scenario");
            dir.join(module)
                .join("scenarios")
                .join(format!("{name}.yaml"))
                .exists()
        };
        if refs.iter().all(has) {
            return dir;
        }
    }
    panic!("{test}: no module tree holds every scenario {refs:?}");
}

/// Like [`plan_of`] for the module tree `modules`. It reads every shape of
/// the files in `contracts/examples/tests/`: a pool that leases its address
/// (`dhcp4`), a missing seed, a mix entry with its own pools, and module
/// settings.
pub fn plan_in(test: &str, modules: &Path) -> Json {
    let text = std::fs::read_to_string(contracts().join("examples/tests").join(test))
        .expect("test configuration");
    let config: serde_norway::Value = serde_norway::from_str(&text).expect("test YAML");
    let mut plan = served_plan();
    plan["testName"] = config["metadata"]["name"].as_str().expect("name").into();
    // The controller chooses a random nonzero seed for a file without one.
    plan["seed"] = config["timeline"]["seed"]
        .as_u64()
        .unwrap_or(99)
        .to_string()
        .into();

    let ports = config["network"]["ports"].as_sequence().expect("ports");
    plan["ports"] = Json::Array(
        ports
            .iter()
            .map(|p| {
                let location = p["location"].as_str().expect("location");
                json!({
                    "name": p["name"].as_str().expect("port name"),
                    "interfaceName": location.rsplit('/').next().expect("interface"),
                })
            })
            .collect(),
    );

    let (mut client_pool, mut server_pool) = (String::new(), String::new());
    let mut pools = Vec::new();
    let mut hosts = Vec::new();
    for (n, p) in config["network"]["host_pools"]
        .as_sequence()
        .expect("host pools")
        .iter()
        .enumerate()
    {
        let name = p["name"].as_str().expect("pool name").to_owned();
        let role = p["role"].as_str().expect("role");
        let count = p["count"].as_u64().expect("count");
        let mut pool = json!({
            "name": name,
            "port": p["port"].as_str().expect("pool port"),
            "role": if role == "client" { "HOST_ROLE_CLIENT" } else { "HOST_ROLE_SERVER" },
            "poolCount": count,
            "count": count,
            "macBase": format!("02:00:{:02x}:00:00:01", n + 1),
        });
        if p["dhcp4"].is_mapping() {
            pool["dhcp4"] = json!({});
        } else {
            pool["ipv4Prefix"] = p["ipv4"].as_str().expect("prefix").into();
        }
        for (key, field) in [("gateway", "gateway"), ("os_profile", "osProfile")] {
            if let Some(v) = p[key].as_str() {
                pool[field] = v.into();
            }
        }
        if role == "client" && client_pool.is_empty() {
            client_pool.clone_from(&name);
        } else if role != "client" && server_pool.is_empty() {
            server_pool.clone_from(&name);
        }
        hosts.push(json!({ "pool": name, "count": count }));
        pools.push(pool);
    }
    plan["hostPools"] = Json::Array(pools);

    let entries = config["profile"]["mix"].as_sequence().expect("mix");
    let total: f64 = entries
        .iter()
        .map(|e| e["weight"].as_f64().expect("weight"))
        .sum();
    let (mut mix, mut rates, mut scenarios) = (Vec::new(), Vec::new(), Vec::new());
    for e in entries {
        let scenario = e["scenario"].as_str().expect("scenario");
        let (module, name) = scenario.split_once('/').expect("module/scenario");
        let mut entry = json!({
            "id": name,
            "scenario": scenario,
            "weight": e["weight"].as_f64().expect("weight") / total,
            "clientPool": e["client_pool"].as_str().unwrap_or(&client_pool),
            "serverPool": e["server_pool"].as_str().unwrap_or(&server_pool),
        });
        if !e["params"].is_null() {
            entry["params"] = serde_json::to_value(&e["params"]).expect("params");
        }
        mix.push(entry);
        rates.push(json!({ "mixEntry": name, "sessionsPerSecond": 1 }));
        let path = modules
            .join(module)
            .join("scenarios")
            .join(format!("{name}.yaml"));
        let source = std::fs::read_to_string(&path).expect("scenario source");
        scenarios.push(json!({
            "ref": scenario,
            "module": module,
            "digest": sha256_digest(source.as_bytes()),
            "source": source,
        }));
    }
    let mut used: Vec<String> = scenarios
        .iter()
        .map(|s| s["module"].as_str().expect("module").to_owned())
        .collect();
    used.sort();
    used.dedup();
    plan["modules"] = Json::Array(
        used.iter()
            .map(|name| {
                let dir = modules.join(name);
                let manifest =
                    std::fs::read_to_string(dir.join("manifest.yaml")).expect("manifest");
                let version = manifest
                    .lines()
                    .find_map(|l| l.strip_prefix("version:"))
                    .map(|v| v.trim().trim_matches('"').to_owned())
                    .expect("version");
                let mut m = json!({
                    "name": name,
                    "version": version,
                    "digest": tree_digest(name, &dir).expect("tree digest"),
                });
                if !config["modules"][name.as_str()]["settings"].is_null() {
                    m["settings"] =
                        serde_json::to_value(&config["modules"][name.as_str()]["settings"])
                            .expect("settings");
                }
                m
            })
            .collect(),
    );
    plan["mix"] = Json::Array(mix);
    plan["scenarios"] = Json::Array(scenarios);
    plan["workers"] = json!([{ "core": 0, "hosts": hosts, "rates": rates }]);
    plan
}
