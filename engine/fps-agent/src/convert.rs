// SPDX-License-Identifier: GPL-3.0-only
//! Conversions between the generated messages in [`crate::pb`] and the types
//! that the run path uses.
//!
//! [`plan_from_pb`] turns the `Plan` of a `prepare` command into the
//! [`Plan`] that the standalone mode reads from a file, so both modes run the
//! same checks. [`snapshot_to_pb`] turns an `fps-stats` snapshot into a
//! `CounterSnapshot` through its proto3 JSON mapping, so both modes send the
//! same bytes for the same counters. [`capabilities_to_pb`] fills the
//! `Capabilities` of `Register` from the hand written mirror.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use serde_json::Value as Json;

use crate::pb;
use crate::plan::{self, Plan, PlanError};
use crate::proto;

/// Errors of converting a generated message.
#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    /// A field holds an enum number that the contract does not define.
    #[error("{field} holds the unknown enum number {value}")]
    Enum {
        /// The proto field.
        field: &'static str,
        /// The number.
        value: i32,
    },
    /// A duration is negative or out of range.
    #[error("{field} is not a valid non negative duration")]
    Duration {
        /// The proto field.
        field: &'static str,
    },
    /// A `params` value is not a number, a string, or a boolean.
    #[error("mix entry {entry}: parameter {name} is not a number, a string, or a boolean")]
    Param {
        /// The mix entry.
        entry: String,
        /// The parameter.
        name: String,
    },
    /// The snapshot JSON did not have the expected shape.
    #[error("snapshot: {0}")]
    Snapshot(String),
}

impl From<ConvertError> for PlanError {
    fn from(e: ConvertError) -> Self {
        PlanError::Rule(e.to_string())
    }
}

fn enum_of<T>(field: &'static str, value: i32, map: &[(i32, T)]) -> Result<T, ConvertError>
where
    T: Copy,
{
    map.iter()
        .find(|(n, _)| *n == value)
        .map(|(_, t)| *t)
        .ok_or(ConvertError::Enum { field, value })
}

/// Converts a `google.protobuf.Duration`. An absent duration is zero.
pub fn duration(
    field: &'static str,
    d: Option<&prost_types::Duration>,
) -> Result<Duration, ConvertError> {
    let Some(d) = d else {
        return Ok(Duration::ZERO);
    };
    let secs = u64::try_from(d.seconds).map_err(|_| ConvertError::Duration { field })?;
    let nanos = u32::try_from(d.nanos).map_err(|_| ConvertError::Duration { field })?;
    if nanos >= 1_000_000_000 {
        return Err(ConvertError::Duration { field });
    }
    Ok(Duration::new(secs, nanos))
}

/// Converts a system time to a `google.protobuf.Timestamp`.
pub fn timestamp(t: SystemTime) -> prost_types::Timestamp {
    let since = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    prost_types::Timestamp {
        seconds: i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        // Below one billion, so the cast is lossless.
        nanos: i32::try_from(since.subsec_nanos()).unwrap_or(0),
    }
}

/// Converts a `google.protobuf.Timestamp` to a system time. Times before the
/// epoch become the epoch.
pub fn system_time(t: &prost_types::Timestamp) -> SystemTime {
    let secs = u64::try_from(t.seconds).unwrap_or(0);
    let nanos = u32::try_from(t.nanos).unwrap_or(0).min(999_999_999);
    UNIX_EPOCH + Duration::new(secs, nanos)
}

fn param(entry: &str, name: &str, v: &prost_types::Value) -> Result<Json, ConvertError> {
    use prost_types::value::Kind;
    let bad = || ConvertError::Param {
        entry: entry.to_owned(),
        name: name.to_owned(),
    };
    match &v.kind {
        Some(Kind::BoolValue(b)) => Ok(Json::Bool(*b)),
        Some(Kind::StringValue(s)) => Ok(Json::String(s.clone())),
        // An integral number becomes a JSON integer, as a plan file writes it.
        #[allow(clippy::cast_possible_truncation)]
        Some(Kind::NumberValue(n)) if n.fract() == 0.0 && n.abs() < 9.0e15 => {
            Ok(Json::from(*n as i64))
        }
        Some(Kind::NumberValue(n)) => serde_json::Number::from_f64(*n)
            .map(Json::Number)
            .ok_or_else(bad),
        _ => Err(bad()),
    }
}

/// Converts the `Plan` of a `prepare` command and applies the checks of
/// [`Plan::check`].
pub fn plan_from_pb(p: &pb::Plan) -> Result<Plan, PlanError> {
    use plan::{Backend, CriterionOp, HostRole, LoadObjective, PhaseShape, Topology};
    let topology = enum_of(
        "topology",
        p.topology,
        &[
            (0, Topology::Unspecified),
            (1, Topology::AppSim),
            (2, Topology::ClientSim),
            (3, Topology::ServerSim),
        ],
    )?;
    let backend = enum_of(
        "backend",
        p.backend,
        &[
            (0, Backend::Unspecified),
            (1, Backend::Kernel),
            (2, Backend::AfXdp),
            (3, Backend::Dpdk),
        ],
    )?;
    let mut mix = Vec::with_capacity(p.mix.len());
    for m in &p.mix {
        let mut params = BTreeMap::new();
        if let Some(s) = &m.params {
            for (k, v) in &s.fields {
                params.insert(k.clone(), param(&m.id, k, v)?);
            }
        }
        mix.push(plan::MixEntry {
            id: m.id.clone(),
            scenario: m.scenario.clone(),
            weight: m.weight,
            params,
            client_pool: m.client_pool.clone(),
            server_pool: m.server_pool.clone(),
        });
    }
    let mut host_pools = Vec::with_capacity(p.host_pools.len());
    for h in &p.host_pools {
        host_pools.push(plan::HostPoolSlice {
            name: h.name.clone(),
            port: h.port.clone(),
            role: enum_of(
                "host_pools.role",
                h.role,
                &[
                    (0, HostRole::Unspecified),
                    (1, HostRole::Client),
                    (2, HostRole::Server),
                ],
            )?,
            ipv4_prefix: h.ipv4_prefix.clone(),
            ipv6_prefix: h.ipv6_prefix.clone(),
            pool_count: h.pool_count,
            offset: h.offset,
            count: h.count,
            gateway: h.gateway.clone(),
            vlan: h.vlan,
            mac_base: h.mac_base.clone(),
            os_profile: h.os_profile.clone(),
        });
    }
    let workers = p
        .workers
        .iter()
        .map(|w| plan::WorkerPlan {
            worker_id: w.worker_id,
            core: w.core,
            hosts: w
                .hosts
                .iter()
                .map(|h| plan::HostRangePlan {
                    pool: h.pool.clone(),
                    offset: h.offset,
                    count: h.count,
                })
                .collect(),
            rates: w
                .rates
                .iter()
                .map(|r| {
                    use pb::worker_rate::Target;
                    plan::WorkerRate {
                        mix_entry: r.mix_entry.clone(),
                        sessions_per_second: match r.target {
                            Some(Target::SessionsPerSecond(v)) => Some(v),
                            _ => None,
                        },
                        concurrent_sessions: match r.target {
                            Some(Target::ConcurrentSessions(v)) => Some(v),
                            _ => None,
                        },
                    }
                })
                .collect(),
        })
        .collect();
    let load = match &p.load {
        None => plan::LoadPlan::default(),
        Some(l) => {
            let mut phases = Vec::with_capacity(l.phases.len());
            for ph in &l.phases {
                phases.push(plan::LoadPhase {
                    name: ph.name.clone(),
                    duration: duration("load.phases.duration", ph.duration.as_ref())?,
                    shape: enum_of(
                        "load.phases.shape",
                        ph.shape,
                        &[
                            (0, PhaseShape::Unspecified),
                            (1, PhaseShape::Hold),
                            (2, PhaseShape::Linear),
                            (3, PhaseShape::Step),
                        ],
                    )?,
                    start_fraction: ph.start_fraction,
                    end_fraction: ph.end_fraction,
                    steps: ph.steps,
                });
            }
            plan::LoadPlan {
                objective: enum_of(
                    "load.objective",
                    l.objective,
                    &[
                        (0, LoadObjective::Unspecified),
                        (1, LoadObjective::ConcurrentSessions),
                        (2, LoadObjective::SessionsPerSecond),
                        (3, LoadObjective::ThroughputBps),
                        (4, LoadObjective::SimulatedUsers),
                    ],
                )?,
                objective_value: l.objective_value,
                phases,
            }
        }
    };
    let mut criteria = Vec::with_capacity(p.criteria.len());
    for c in &p.criteria {
        criteria.push(plan::Criterion {
            metric: c.metric.clone(),
            op: enum_of(
                "criteria.op",
                c.op,
                &[
                    (0, CriterionOp::Unspecified),
                    (1, CriterionOp::Lt),
                    (2, CriterionOp::Le),
                    (3, CriterionOp::Gt),
                    (4, CriterionOp::Ge),
                    (5, CriterionOp::Eq),
                    (6, CriterionOp::Ne),
                ],
            )?,
            value: c.value,
        });
    }
    let out = Plan {
        plan_id: p.plan_id.clone(),
        run_id: p.run_id.clone(),
        agent_id: p.agent_id.clone(),
        test_name: p.test_name.clone(),
        engine_api: p.engine_api.clone(),
        seed: p.seed,
        topology,
        backend,
        modules: p
            .modules
            .iter()
            .map(|m| plan::ModuleRef {
                name: m.name.clone(),
                version: m.version.clone(),
                digest: m.digest.clone(),
            })
            .collect(),
        scenarios: p
            .scenarios
            .iter()
            .map(|s| plan::ScenarioSource {
                r#ref: s.r#ref.clone(),
                module: s.module.clone(),
                source: s.source.clone(),
                digest: s.digest.clone(),
            })
            .collect(),
        mix,
        ports: p
            .ports
            .iter()
            .map(|b| plan::PortBinding {
                name: b.name.clone(),
                interface_name: b.interface_name.clone(),
                mtu: b.mtu,
            })
            .collect(),
        host_pools,
        external_servers: p
            .external_servers
            .iter()
            .map(|e| plan::ExternalServer {
                name: e.name.clone(),
                address: e.address.clone(),
                port: e.port,
            })
            .collect(),
        workers,
        load,
        warmup: duration("warmup", p.warmup.as_ref())?,
        criteria,
    };
    out.check()?;
    Ok(out)
}

fn pb_duration(d: Duration) -> Option<prost_types::Duration> {
    if d.is_zero() {
        return None;
    }
    Some(prost_types::Duration {
        seconds: i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        nanos: i32::try_from(d.subsec_nanos()).unwrap_or(0),
    })
}

fn pb_param(v: &Json) -> prost_types::Value {
    use prost_types::value::Kind;
    let kind = match v {
        Json::Bool(b) => Kind::BoolValue(*b),
        Json::String(s) => Kind::StringValue(s.clone()),
        Json::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or_default()),
        Json::Null | Json::Array(_) | Json::Object(_) => Kind::NullValue(0),
    };
    prost_types::Value { kind: Some(kind) }
}

/// Builds the generated `Plan` from a plan of the standalone mode, the
/// inverse of [`plan_from_pb`]. Tests and tools use it to send a plan file
/// in a `prepare` command.
pub fn plan_to_pb(p: &Plan) -> pb::Plan {
    use plan::{Backend, CriterionOp, HostRole, LoadObjective, PhaseShape, Topology};
    pb::Plan {
        plan_id: p.plan_id.clone(),
        run_id: p.run_id.clone(),
        agent_id: p.agent_id.clone(),
        test_name: p.test_name.clone(),
        engine_api: p.engine_api.clone(),
        seed: p.seed,
        topology: match p.topology {
            Topology::Unspecified => 0,
            Topology::AppSim => 1,
            Topology::ClientSim => 2,
            Topology::ServerSim => 3,
        },
        backend: match p.backend {
            Backend::Unspecified => 0,
            Backend::Kernel => 1,
            Backend::AfXdp => 2,
            Backend::Dpdk => 3,
        },
        modules: p
            .modules
            .iter()
            .map(|m| pb::ModuleRef {
                name: m.name.clone(),
                version: m.version.clone(),
                digest: m.digest.clone(),
            })
            .collect(),
        scenarios: p
            .scenarios
            .iter()
            .map(|s| pb::ScenarioSource {
                r#ref: s.r#ref.clone(),
                module: s.module.clone(),
                source: s.source.clone(),
                digest: s.digest.clone(),
            })
            .collect(),
        mix: p
            .mix
            .iter()
            .map(|m| pb::MixEntry {
                id: m.id.clone(),
                scenario: m.scenario.clone(),
                weight: m.weight,
                params: (!m.params.is_empty()).then(|| prost_types::Struct {
                    fields: m
                        .params
                        .iter()
                        .map(|(k, v)| (k.clone(), pb_param(v)))
                        .collect(),
                }),
                client_pool: m.client_pool.clone(),
                server_pool: m.server_pool.clone(),
            })
            .collect(),
        ports: p
            .ports
            .iter()
            .map(|b| pb::PortBinding {
                name: b.name.clone(),
                interface_name: b.interface_name.clone(),
                mtu: b.mtu,
            })
            .collect(),
        host_pools: p
            .host_pools
            .iter()
            .map(|h| pb::HostPoolSlice {
                name: h.name.clone(),
                port: h.port.clone(),
                role: match h.role {
                    HostRole::Unspecified => 0,
                    HostRole::Client => 1,
                    HostRole::Server => 2,
                },
                ipv4_prefix: h.ipv4_prefix.clone(),
                ipv6_prefix: h.ipv6_prefix.clone(),
                pool_count: h.pool_count,
                offset: h.offset,
                count: h.count,
                gateway: h.gateway.clone(),
                vlan: h.vlan,
                mac_base: h.mac_base.clone(),
                os_profile: h.os_profile.clone(),
            })
            .collect(),
        external_servers: p
            .external_servers
            .iter()
            .map(|e| pb::ExternalServer {
                name: e.name.clone(),
                address: e.address.clone(),
                port: e.port,
            })
            .collect(),
        workers: p
            .workers
            .iter()
            .map(|w| pb::WorkerPlan {
                worker_id: w.worker_id,
                core: w.core,
                hosts: w
                    .hosts
                    .iter()
                    .map(|h| pb::HostRange {
                        pool: h.pool.clone(),
                        offset: h.offset,
                        count: h.count,
                    })
                    .collect(),
                rates: w
                    .rates
                    .iter()
                    .map(|r| {
                        use pb::worker_rate::Target;
                        pb::WorkerRate {
                            mix_entry: r.mix_entry.clone(),
                            target: match (r.sessions_per_second, r.concurrent_sessions) {
                                (Some(v), _) => Some(Target::SessionsPerSecond(v)),
                                (None, Some(v)) => Some(Target::ConcurrentSessions(v)),
                                (None, None) => None,
                            },
                        }
                    })
                    .collect(),
            })
            .collect(),
        load: Some(pb::LoadPlan {
            objective: match p.load.objective {
                LoadObjective::Unspecified => 0,
                LoadObjective::ConcurrentSessions => 1,
                LoadObjective::SessionsPerSecond => 2,
                LoadObjective::ThroughputBps => 3,
                LoadObjective::SimulatedUsers => 4,
            },
            objective_value: p.load.objective_value,
            phases: p
                .load
                .phases
                .iter()
                .map(|ph| pb::LoadPhase {
                    name: ph.name.clone(),
                    duration: pb_duration(ph.duration),
                    shape: match ph.shape {
                        PhaseShape::Unspecified => 0,
                        PhaseShape::Hold => 1,
                        PhaseShape::Linear => 2,
                        PhaseShape::Step => 3,
                    },
                    start_fraction: ph.start_fraction,
                    end_fraction: ph.end_fraction,
                    steps: ph.steps,
                })
                .collect(),
        }),
        warmup: pb_duration(p.warmup),
        criteria: p
            .criteria
            .iter()
            .map(|c| pb::Criterion {
                metric: c.metric.clone(),
                op: match c.op {
                    CriterionOp::Unspecified => 0,
                    CriterionOp::Lt => 1,
                    CriterionOp::Le => 2,
                    CriterionOp::Gt => 3,
                    CriterionOp::Ge => 4,
                    CriterionOp::Eq => 5,
                    CriterionOp::Ne => 6,
                },
                value: c.value,
            })
            .collect(),
    }
}

fn labels(v: &Json) -> HashMapLabels {
    v.get("labels")
        .and_then(Json::as_object)
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
                .collect()
        })
        .unwrap_or_default()
}

type HashMapLabels = std::collections::HashMap<String, String>;

fn u64_field(v: &Json, field: &str) -> Result<u64, ConvertError> {
    match v.get(field) {
        None => Ok(0),
        Some(Json::String(s)) => s
            .parse()
            .map_err(|_| ConvertError::Snapshot(format!("{field} {s:?} is not a uint64"))),
        Some(other) => other
            .as_u64()
            .ok_or_else(|| ConvertError::Snapshot(format!("{field} is not a uint64"))),
    }
}

/// Builds a `CounterSnapshot` from an `fps-stats` snapshot and its metadata.
pub fn snapshot_to_pb(
    snap: &fps_stats::Snapshot,
    meta: &fps_stats::SnapshotMeta,
) -> Result<pb::CounterSnapshot, ConvertError> {
    let json = snap
        .to_proto_json(meta)
        .map_err(|e| ConvertError::Snapshot(e.to_string()))?;
    let mut out = pb::CounterSnapshot {
        plan_id: meta.plan_id.clone(),
        sequence: meta.sequence,
        time: Some(prost_types::Timestamp {
            seconds: meta.time.seconds,
            nanos: i32::try_from(meta.time.nanos).unwrap_or(0),
        }),
        phase: meta.phase.clone(),
        worker_id: snap.worker_id().unwrap_or(0),
        aggregate: snap.worker_id().is_none(),
        is_final: meta.is_final,
        counters: Vec::new(),
        histograms: Vec::new(),
    };
    for c in json
        .get("counters")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
    {
        use pb::counter_value::Value;
        let value = if let Some(g) = c.get("gauge") {
            // The JSON mapping writes NaN and the infinities as strings.
            let g = match g {
                Json::String(s) => match s.as_str() {
                    "NaN" => f64::NAN,
                    "Infinity" => f64::INFINITY,
                    "-Infinity" => f64::NEG_INFINITY,
                    _ => return Err(ConvertError::Snapshot(format!("gauge {s:?}"))),
                },
                other => other.as_f64().unwrap_or_default(),
            };
            Value::Gauge(g)
        } else {
            Value::Count(u64_field(c, "count")?)
        };
        out.counters.push(pb::CounterValue {
            metric: c["metric"].as_str().unwrap_or_default().to_owned(),
            labels: labels(c),
            value: Some(value),
        });
    }
    for h in json
        .get("histograms")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
    {
        let hdr = h.get("hdrV2").and_then(Json::as_str).unwrap_or_default();
        let hdr_v2 = base64::engine::general_purpose::STANDARD
            .decode(hdr)
            .map_err(|e| ConvertError::Snapshot(format!("hdrV2: {e}")))?;
        out.histograms.push(pb::HistogramValue {
            metric: h["metric"].as_str().unwrap_or_default().to_owned(),
            labels: labels(h),
            count: u64_field(h, "count")?,
            hdr_v2,
        });
    }
    Ok(out)
}

/// Fills the generated `Capabilities` from the hand written mirror.
pub fn capabilities_to_pb(c: &proto::Capabilities) -> pb::Capabilities {
    pb::Capabilities {
        agent_version: c.agent_version.clone(),
        engine_api: c.engine_api.clone(),
        backends: c.backends.iter().map(|b| b.number()).collect(),
        cores: c.cores,
        memory_bytes: c.memory_bytes,
        ports: c
            .ports
            .iter()
            .map(|p| pb::PortInfo {
                name: p.name.clone(),
                driver: p.driver.clone(),
                speed_mbps: p.speed_mbps,
                mac: p.mac.clone(),
                mtu: p.mtu,
            })
            .collect(),
        modules: c
            .modules
            .iter()
            .map(|m| pb::ModuleRef {
                name: m.name.clone(),
                version: m.version.clone(),
                digest: m.digest.clone(),
            })
            .collect(),
        labels: c
            .labels
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_reject_negative_values() {
        let d = prost_types::Duration {
            seconds: -1,
            nanos: 0,
        };
        assert!(duration("x", Some(&d)).is_err());
        let d = prost_types::Duration {
            seconds: 2,
            nanos: 500_000_000,
        };
        assert_eq!(
            duration("x", Some(&d)).unwrap(),
            Duration::from_millis(2500)
        );
        assert_eq!(duration("x", None).unwrap(), Duration::ZERO);
    }

    #[test]
    fn unknown_enum_numbers_are_rule_errors() {
        let p = pb::Plan {
            plan_id: "p".into(),
            topology: 9,
            ..Default::default()
        };
        let e = plan_from_pb(&p).unwrap_err();
        assert!(e.to_string().contains("topology"), "{e}");
    }

    #[test]
    fn params_reject_lists() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "x".to_owned(),
            prost_types::Value {
                kind: Some(prost_types::value::Kind::ListValue(Default::default())),
            },
        );
        let p = pb::Plan {
            plan_id: "p".into(),
            mix: vec![pb::MixEntry {
                id: "m".into(),
                params: Some(prost_types::Struct { fields }),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(plan_from_pb(&p).is_err());
    }

    #[test]
    fn the_example_plan_survives_the_round_trip_through_the_generated_types() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/examples/plans/http_dns_smoke.json");
        let plan = Plan::load(&path).unwrap();
        let back = plan_from_pb(&plan_to_pb(&plan)).unwrap();
        assert_eq!(format!("{back:?}"), format!("{plan:?}"));
    }

    #[test]
    fn timestamps_round_trip() {
        let t = UNIX_EPOCH + Duration::new(1_700_000_000, 123);
        assert_eq!(system_time(&timestamp(t)), t);
    }
}
