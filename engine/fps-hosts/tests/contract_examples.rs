// SPDX-License-Identifier: GPL-3.0-only
//! Contract tests that read the host pools of the example test configurations
//! and the example plan under `contracts/examples/`.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use fps_hosts::{check_partition, HostPool, HostRange, HostSelector, PoolSpec, Selection};
use serde_json::Value as Json;
use serde_yaml::Value as Yaml;

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/examples")
}

fn yaml_str<'a>(v: &'a Yaml, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Yaml::as_str)
}

#[test]
fn every_example_test_config_pool_parses_and_maps_unique_addresses() {
    let mut seen = 0;
    for entry in fs::read_dir(examples().join("tests")).unwrap() {
        let path = entry.unwrap().path();
        let doc: Yaml = serde_yaml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let pools = doc["network"]["host_pools"].as_sequence().unwrap();
        for p in pools {
            let count = u32::try_from(p["count"].as_u64().unwrap()).unwrap();
            let gateway = p.get("gateway").and_then(Yaml::as_str);
            let pool = HostPool::parse(&PoolSpec {
                name: yaml_str(p, "name").unwrap(),
                ipv4: yaml_str(p, "ipv4"),
                ipv6: yaml_str(p, "ipv6"),
                count,
                gateway,
                mac_base: yaml_str(p, "mac_base"),
            })
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let mut addrs = HashSet::new();
            for i in pool.range().indexes() {
                let a = pool.ipv4_addr(i).unwrap().unwrap();
                assert_ne!(Some(a.to_string().as_str()), gateway);
                assert!(pool.ipv4().unwrap().contains(a));
                assert!(addrs.insert(a));
            }
            seen += 1;
        }
    }
    assert!(seen >= 4, "found only {seen} pools");
}

#[test]
fn enterprise_users_pool_starts_after_the_gateway() {
    let pool = HostPool::parse(&PoolSpec {
        name: "users",
        ipv4: Some("10.10.0.0/16"),
        count: 20000,
        gateway: Some("10.10.0.1"),
        ..PoolSpec::default()
    })
    .unwrap();
    assert_eq!(pool.ipv4_addr(0).unwrap().unwrap().to_string(), "10.10.0.2");
    assert_eq!(
        pool.ipv4_addr(19999).unwrap().unwrap().to_string(),
        "10.10.78.33"
    );
}

fn num(v: &Json, key: &str) -> u32 {
    // The proto3 JSON mapping omits zero values.
    v.get(key)
        .and_then(Json::as_u64)
        .map_or(0, |n| u32::try_from(n).unwrap())
}

#[test]
fn example_plan_worker_ranges_partition_each_agent_slice() {
    let plans = fs::read_dir(examples().join("plans")).unwrap();
    for entry in plans {
        let path = entry.unwrap().path();
        let plan: Json = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let seed = plan.get("seed").and_then(Json::as_str).map_or_else(
            || plan["seed"].as_u64().unwrap_or(0),
            |s| s.parse().unwrap(),
        );
        for slice in plan["hostPools"].as_array().unwrap() {
            let name = slice["name"].as_str().unwrap();
            let whole = HostRange::new(num(slice, "offset"), num(slice, "count"));
            assert!(whole.end() <= u64::from(num(slice, "poolCount")));
            let pool = HostPool::parse(&PoolSpec {
                name,
                ipv4: slice.get("ipv4Prefix").and_then(Json::as_str),
                ipv6: slice.get("ipv6Prefix").and_then(Json::as_str),
                count: num(slice, "poolCount"),
                gateway: slice.get("gateway").and_then(Json::as_str),
                mac_base: slice.get("macBase").and_then(Json::as_str),
            })
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let mut ranges = Vec::new();
            for w in plan["workers"].as_array().unwrap() {
                for h in w["hosts"].as_array().unwrap() {
                    if h["pool"].as_str() == Some(name) {
                        let r = HostRange::new(num(h, "offset"), num(h, "count"));
                        let sel = HostSelector::new(
                            name,
                            r,
                            Selection::Uniform,
                            seed,
                            num(w, "workerId"),
                        )
                        .unwrap();
                        let host = sel.select(0);
                        assert!(r.contains(host));
                        pool.ipv4_addr(host).unwrap();
                        ranges.push(r);
                    }
                }
            }
            ranges.sort_by_key(|r| r.offset);
            check_partition(&whole, &ranges)
                .unwrap_or_else(|e| panic!("{}: pool {name}: {e}", path.display()));
        }
    }
}
