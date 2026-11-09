// SPDX-License-Identifier: GPL-3.0-only
//! The plan file of the standalone mode.
//!
//! The standalone mode reads `fps.agent.v1.Plan` in the canonical proto3 JSON
//! mapping that `contracts/README.md` defines. Field names are lowerCamelCase,
//! enum values are names, 64 bit integers are strings, durations are strings
//! such as `"30s"`, and an absent field holds its default value. Unknown
//! fields are errors, as in `tools/contracts/check_plans.py`. The structs
//! mirror the proto messages field for field, with the proto field names in
//! snake case.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::path::Path;
use std::time::Duration;

use serde::de::{self, Deserializer};
use serde::Deserialize;
use serde_json::Value as Json;

use crate::run::RunError;

/// Mirrors `fps.agent.v1.Plan`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Plan {
    /// Unique per run and agent.
    pub plan_id: String,
    /// The run name.
    pub run_id: String,
    /// The agent that executes the plan.
    pub agent_id: String,
    /// `metadata.name` of the test configuration.
    pub test_name: String,
    /// The contract version that the plan targets.
    pub engine_api: String,
    /// The run seed.
    #[serde(deserialize_with = "uint64")]
    pub seed: u64,
    /// The topology of the test configuration.
    pub topology: Topology,
    /// The transport backend of every port.
    pub backend: Backend,
    /// Every module that the plan needs, with its digest.
    pub modules: Vec<ModuleRef>,
    /// The source of every scenario that the mix uses.
    pub scenarios: Vec<ScenarioSource>,
    /// The resolved mix entries.
    pub mix: Vec<MixEntry>,
    /// The traffic ports of this agent.
    pub ports: Vec<PortBinding>,
    /// The slice of each host pool that this agent emulates.
    pub host_pools: Vec<HostPoolSlice>,
    /// Real servers for `TOPOLOGY_CLIENT_SIM`.
    pub external_servers: Vec<ExternalServer>,
    /// One entry per worker.
    pub workers: Vec<WorkerPlan>,
    /// The objective and the load phases.
    pub load: LoadPlan,
    /// The time from the start that criteria exclude.
    #[serde(deserialize_with = "duration")]
    pub warmup: Duration,
    /// Pass and fail rules.
    pub criteria: Vec<Criterion>,
}

/// Mirrors `fps.agent.v1.Topology`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub enum Topology {
    /// Invalid in a plan.
    #[default]
    #[serde(rename = "TOPOLOGY_UNSPECIFIED")]
    Unspecified,
    /// The agent emulates clients and servers.
    #[serde(rename = "TOPOLOGY_APP_SIM")]
    AppSim,
    /// The agent emulates clients of real servers.
    #[serde(rename = "TOPOLOGY_CLIENT_SIM")]
    ClientSim,
    /// The agent emulates servers of real clients.
    #[serde(rename = "TOPOLOGY_SERVER_SIM")]
    ServerSim,
}

/// Mirrors `fps.agent.v1.Backend`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub enum Backend {
    /// Invalid in a plan.
    #[default]
    #[serde(rename = "BACKEND_UNSPECIFIED")]
    Unspecified,
    /// Kernel sockets.
    #[serde(rename = "BACKEND_KERNEL")]
    Kernel,
    /// AF_XDP sockets.
    #[serde(rename = "BACKEND_AF_XDP")]
    AfXdp,
    /// DPDK poll mode drivers.
    #[serde(rename = "BACKEND_DPDK")]
    Dpdk,
}

/// Mirrors `fps.agent.v1.ModuleRef`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct ModuleRef {
    /// The module name.
    pub name: String,
    /// The module version.
    pub version: String,
    /// `sha256:<hex>` over the module tree.
    pub digest: String,
    /// The Ed25519 signature over the digest text, which the canonical JSON
    /// mapping writes as base64, or empty for an unsigned module.
    #[serde(deserialize_with = "base64_bytes")]
    pub signature: Vec<u8>,
    /// The id of the key that made the signature, or empty.
    pub signature_key_id: String,
    /// The module settings of the test, a JSON object that conforms to the
    /// settings schema of the module manifest. The agent carries them and
    /// applies none yet.
    pub settings: serde_json::Map<String, serde_json::Value>,
}

/// Mirrors `fps.agent.v1.ScenarioSource`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct ScenarioSource {
    /// `<module>/<scenario>`.
    pub r#ref: String,
    /// The module that exports the scenario.
    pub module: String,
    /// The scenario file in YAML.
    pub source: String,
    /// `sha256:<hex>` over the UTF-8 bytes of the source.
    pub digest: String,
}

/// Mirrors `fps.agent.v1.MixEntry`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct MixEntry {
    /// The entry name, which worker rates refer to.
    pub id: String,
    /// The ref of a scenario source.
    pub scenario: String,
    /// The normalized weight.
    pub weight: f64,
    /// The effective scenario variables, a `google.protobuf.Struct`.
    pub params: BTreeMap<String, Json>,
    /// The client host pool, or empty for the default.
    pub client_pool: String,
    /// The server host pool, or empty for the default.
    pub server_pool: String,
}

/// Mirrors `fps.agent.v1.PortBinding`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct PortBinding {
    /// The port name of the test configuration.
    pub name: String,
    /// The local interface.
    pub interface_name: String,
    /// The MTU, or zero to keep the current value.
    pub mtu: u32,
}

/// Mirrors `fps.agent.v1.HostRole`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub enum HostRole {
    /// Invalid in a plan.
    #[default]
    #[serde(rename = "HOST_ROLE_UNSPECIFIED")]
    Unspecified,
    /// Client hosts.
    #[serde(rename = "HOST_ROLE_CLIENT")]
    Client,
    /// Server hosts.
    #[serde(rename = "HOST_ROLE_SERVER")]
    Server,
}

/// Mirrors `fps.agent.v1.HostPoolSlice`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct HostPoolSlice {
    /// The pool name.
    pub name: String,
    /// The port that the pool sits on.
    pub port: String,
    /// The role of the pool.
    pub role: HostRole,
    /// The IPv4 prefix of the whole pool.
    pub ipv4_prefix: String,
    /// The IPv6 prefix of the whole pool.
    pub ipv6_prefix: String,
    /// The number of hosts of the whole pool.
    pub pool_count: u32,
    /// The pool index of the first host of the slice.
    pub offset: u32,
    /// The number of hosts of the slice.
    pub count: u32,
    /// The gateway address, or empty.
    pub gateway: String,
    /// The VLAN id, or zero.
    pub vlan: u32,
    /// The MAC address of pool index 0.
    pub mac_base: String,
    /// The host OS profile.
    pub os_profile: String,
}

/// Mirrors `fps.agent.v1.ExternalServer`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct ExternalServer {
    /// The server name.
    pub name: String,
    /// A host name or an IP address.
    pub address: String,
    /// The server port, or zero for the scenario default.
    pub port: u32,
}

/// Mirrors `fps.agent.v1.WorkerPlan`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct WorkerPlan {
    /// Unique within the plan.
    pub worker_id: u32,
    /// The CPU that the worker pins to.
    pub core: u32,
    /// The host ranges that the worker owns.
    pub hosts: Vec<HostRangePlan>,
    /// The steady state target of each mix entry.
    pub rates: Vec<WorkerRate>,
}

/// Mirrors `fps.agent.v1.HostRange`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct HostRangePlan {
    /// The host pool.
    pub pool: String,
    /// The pool index of the first host.
    pub offset: u32,
    /// The number of hosts.
    pub count: u32,
}

/// Mirrors `fps.agent.v1.WorkerRate`, whose `target` oneof becomes two
/// options of which at most one may be set.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct WorkerRate {
    /// The mix entry.
    pub mix_entry: String,
    /// The session open rate.
    pub sessions_per_second: Option<f64>,
    /// The number of sessions kept open.
    pub concurrent_sessions: Option<u32>,
}

/// Mirrors `fps.agent.v1.LoadObjective`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub enum LoadObjective {
    /// Invalid in a plan.
    #[default]
    #[serde(rename = "LOAD_OBJECTIVE_UNSPECIFIED")]
    Unspecified,
    /// Open sessions.
    #[serde(rename = "LOAD_OBJECTIVE_CONCURRENT_SESSIONS")]
    ConcurrentSessions,
    /// Session open rate.
    #[serde(rename = "LOAD_OBJECTIVE_SESSIONS_PER_SECOND")]
    SessionsPerSecond,
    /// Bits per second, compiled to a session rate.
    #[serde(rename = "LOAD_OBJECTIVE_THROUGHPUT_BPS")]
    ThroughputBps,
    /// Simulated users, compiled to open sessions.
    #[serde(rename = "LOAD_OBJECTIVE_SIMULATED_USERS")]
    SimulatedUsers,
}

/// Mirrors `fps.agent.v1.LoadPlan`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct LoadPlan {
    /// The objective type.
    pub objective: LoadObjective,
    /// The share of the objective value for this agent.
    pub objective_value: f64,
    /// The phases in order.
    pub phases: Vec<LoadPhase>,
}

/// Mirrors `fps.agent.v1.PhaseShape`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub enum PhaseShape {
    /// Invalid in a plan.
    #[default]
    #[serde(rename = "PHASE_SHAPE_UNSPECIFIED")]
    Unspecified,
    /// One fraction for the whole phase.
    #[serde(rename = "PHASE_SHAPE_HOLD")]
    Hold,
    /// A linear move between the fractions.
    #[serde(rename = "PHASE_SHAPE_LINEAR")]
    Linear,
    /// Equal steps between the fractions.
    #[serde(rename = "PHASE_SHAPE_STEP")]
    Step,
}

/// Mirrors `fps.agent.v1.LoadPhase`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct LoadPhase {
    /// The phase name.
    pub name: String,
    /// The length of the phase.
    #[serde(deserialize_with = "duration")]
    pub duration: Duration,
    /// How the fraction moves.
    pub shape: PhaseShape,
    /// The fraction at the phase start.
    pub start_fraction: f64,
    /// The fraction at the phase end.
    pub end_fraction: f64,
    /// The number of steps of a step phase.
    pub steps: u32,
}

/// Mirrors `fps.agent.v1.CriterionOp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
pub enum CriterionOp {
    /// Invalid in a plan.
    #[default]
    #[serde(rename = "CRITERION_OP_UNSPECIFIED")]
    Unspecified,
    /// Less than.
    #[serde(rename = "CRITERION_OP_LT")]
    Lt,
    /// Less than or equal.
    #[serde(rename = "CRITERION_OP_LE")]
    Le,
    /// Greater than.
    #[serde(rename = "CRITERION_OP_GT")]
    Gt,
    /// Greater than or equal.
    #[serde(rename = "CRITERION_OP_GE")]
    Ge,
    /// Equal.
    #[serde(rename = "CRITERION_OP_EQ")]
    Eq,
    /// Not equal.
    #[serde(rename = "CRITERION_OP_NE")]
    Ne,
}

impl CriterionOp {
    /// Returns the comparison symbol, for messages.
    pub fn symbol(self) -> &'static str {
        match self {
            CriterionOp::Unspecified => "?",
            CriterionOp::Lt => "<",
            CriterionOp::Le => "<=",
            CriterionOp::Gt => ">",
            CriterionOp::Ge => ">=",
            CriterionOp::Eq => "==",
            CriterionOp::Ne => "!=",
        }
    }

    /// Applies the comparison. Every comparison with NaN fails.
    pub fn holds(self, value: f64, threshold: f64) -> bool {
        match self {
            CriterionOp::Unspecified => false,
            CriterionOp::Lt => value < threshold,
            CriterionOp::Le => value <= threshold,
            CriterionOp::Gt => value > threshold,
            CriterionOp::Ge => value >= threshold,
            #[allow(clippy::float_cmp)]
            CriterionOp::Eq => value == threshold,
            #[allow(clippy::float_cmp)]
            CriterionOp::Ne => !value.is_nan() && value != threshold,
        }
    }
}

/// Mirrors `fps.agent.v1.Criterion`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Criterion {
    /// A catalog metric name, with a statistic for a histogram.
    pub metric: String,
    /// The comparison.
    pub op: CriterionOp,
    /// The threshold.
    pub value: f64,
}

/// Errors of reading a plan file.
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    /// The file could not be read.
    #[error("read {path}")]
    Read {
        /// The plan file.
        path: String,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
    /// The file is not a plan in the canonical JSON mapping.
    #[error("parse {path}: {source}")]
    Parse {
        /// The plan file.
        path: String,
        /// The cause.
        #[source]
        source: serde_json::Error,
    },
    /// The plan breaks a rule of the plan contract.
    #[error("plan rule: {0}")]
    Rule(String),
}

impl Plan {
    /// Reads and checks a plan file.
    pub fn load(path: &Path) -> Result<Self, PlanError> {
        let text = std::fs::read_to_string(path).map_err(|source| PlanError::Read {
            path: path.display().to_string(),
            source,
        })?;
        let plan = Self::parse(&text).map_err(|e| match e {
            PlanError::Parse { source, .. } => PlanError::Parse {
                path: path.display().to_string(),
                source,
            },
            other => other,
        })?;
        Ok(plan)
    }

    /// Parses and checks plan text.
    pub fn parse(text: &str) -> Result<Self, PlanError> {
        let plan: Plan = serde_json::from_str(text).map_err(|source| PlanError::Parse {
            path: "<text>".to_owned(),
            source,
        })?;
        plan.check()?;
        Ok(plan)
    }

    /// Applies the reference and range rules of the plan contract that the
    /// standalone mode relies on.
    pub fn check(&self) -> Result<(), PlanError> {
        let rule = |m: String| Err(PlanError::Rule(m));
        if self.plan_id.is_empty() {
            return rule("planId is required".into());
        }
        let mut modules = HashSet::new();
        for m in &self.modules {
            if !modules.insert(m.name.as_str()) {
                return rule(format!("duplicate module '{}'", m.name));
            }
        }
        let mut scenarios = HashSet::new();
        for s in &self.scenarios {
            if !scenarios.insert(s.r#ref.as_str()) {
                return rule(format!("duplicate scenario '{}'", s.r#ref));
            }
            if !modules.contains(s.module.as_str()) {
                return rule(format!(
                    "scenario '{}' names module '{}', which is not in modules",
                    s.r#ref, s.module
                ));
            }
        }
        let pools: HashMap<&str, &HostPoolSlice> = self
            .host_pools
            .iter()
            .map(|p| (p.name.as_str(), p))
            .collect();
        if pools.len() != self.host_pools.len() {
            return rule("duplicate host pool".into());
        }
        let mut mix = HashSet::new();
        for e in &self.mix {
            if !mix.insert(e.id.as_str()) {
                return rule(format!("duplicate mix entry '{}'", e.id));
            }
            if !scenarios.contains(e.scenario.as_str()) {
                return rule(format!(
                    "mix entry '{}' names scenario '{}', which is not in scenarios",
                    e.id, e.scenario
                ));
            }
            for (pool, role) in [
                (&e.client_pool, HostRole::Client),
                (&e.server_pool, HostRole::Server),
            ] {
                if pool.is_empty() {
                    continue;
                }
                match pools.get(pool.as_str()) {
                    None => return rule(format!("mix entry '{}': unknown pool '{pool}'", e.id)),
                    Some(p) if p.role != role => {
                        return rule(format!(
                            "mix entry '{}': pool '{pool}' has role {:?}",
                            e.id, p.role
                        ))
                    }
                    Some(_) => {}
                }
            }
        }
        let mut workers = HashSet::new();
        for w in &self.workers {
            if !workers.insert(w.worker_id) {
                return rule(format!("duplicate worker {}", w.worker_id));
            }
            for h in &w.hosts {
                let Some(p) = pools.get(h.pool.as_str()) else {
                    return rule(format!(
                        "worker {}: unknown host pool '{}'",
                        w.worker_id, h.pool
                    ));
                };
                let end = u64::from(h.offset) + u64::from(h.count);
                if h.count == 0
                    || h.offset < p.offset
                    || end > u64::from(p.offset) + u64::from(p.count)
                {
                    return rule(format!(
                        "worker {}: host range of '{}' lies outside the slice",
                        w.worker_id, h.pool
                    ));
                }
            }
            for r in &w.rates {
                if !mix.contains(r.mix_entry.as_str()) {
                    return rule(format!(
                        "worker {}: unknown mix entry '{}'",
                        w.worker_id, r.mix_entry
                    ));
                }
                if r.sessions_per_second.is_some() == r.concurrent_sessions.is_some() {
                    return rule(format!(
                        "worker {}: rate of '{}' needs exactly one target",
                        w.worker_id, r.mix_entry
                    ));
                }
            }
        }
        if self.workers.is_empty() {
            return rule("a plan needs at least one worker".into());
        }
        if self.load.phases.is_empty() {
            return rule("a plan needs at least one load phase".into());
        }
        Ok(())
    }

    /// Returns the total length of the load phases.
    pub fn load_duration(&self) -> Duration {
        self.load.phases.iter().map(|p| p.duration).sum()
    }

    /// Returns the pool that `name` names, or the first pool of `role` when
    /// `name` is empty.
    pub fn pool(&self, name: &str, role: HostRole) -> Result<&HostPoolSlice, RunError> {
        let found = if name.is_empty() {
            self.host_pools.iter().find(|p| p.role == role)
        } else {
            self.host_pools.iter().find(|p| p.name == name)
        };
        found.ok_or_else(|| {
            RunError::Plan(PlanError::Rule(format!(
                "no host pool '{name}' with role {role:?}"
            )))
        })
    }
}

/// Reads a `bytes` field, which the canonical mapping writes as base64. A
/// proto3 JSON parser accepts the standard and the URL safe alphabet, with or
/// without padding.
fn base64_bytes<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    use base64::Engine as _;
    let text = String::deserialize(d)?;
    let text = text.trim_end_matches('=');
    let standard = base64::engine::general_purpose::STANDARD_NO_PAD;
    let url = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    standard
        .decode(text)
        .or_else(|_| url.decode(text))
        .map_err(|e| de::Error::custom(format!("invalid base64: {e}")))
}

/// Reads a `uint64`, which the canonical mapping writes as a string and
/// which a proto3 JSON parser also accepts as a number.
fn uint64<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    struct V;
    impl de::Visitor<'_> for V {
        type Value = u64;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a uint64 as a decimal string or a number")
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<u64, E> {
            Ok(v)
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<u64, E> {
            v.parse()
                .map_err(|_| E::custom(format!("invalid uint64 {v:?}")))
        }
    }
    d.deserialize_any(V)
}

/// Reads a `google.protobuf.Duration` string such as `"30s"` or `"0.250s"`.
fn duration<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
    let s = String::deserialize(d)?;
    parse_duration(&s).ok_or_else(|| de::Error::custom(format!("invalid duration {s:?}")))
}

/// Parses the proto3 JSON form of a non negative duration.
pub fn parse_duration(s: &str) -> Option<Duration> {
    let num = s.strip_suffix('s')?;
    let (whole, frac) = num.split_once('.').unwrap_or((num, ""));
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if frac.len() > 9 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let secs: u64 = whole.parse().ok()?;
    let nanos: u32 = if frac.is_empty() {
        0
    } else {
        format!("{frac:0<9}").parse().ok()?
    };
    Some(Duration::new(secs, nanos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_follow_the_proto3_json_form() {
        assert_eq!(parse_duration("30s"), Some(Duration::from_secs(30)));
        assert_eq!(parse_duration("0.250s"), Some(Duration::from_millis(250)));
        assert_eq!(parse_duration("1.000000001s"), Some(Duration::new(1, 1)));
        for bad in ["30", "s", "-1s", "1.0000000001s", "1.s5", "1e3s"] {
            assert_eq!(parse_duration(bad), None, "{bad}");
        }
    }

    #[test]
    fn unknown_fields_and_missing_targets_are_rejected() {
        let err = Plan::parse(r#"{"planId":"p","bogus":1}"#).unwrap_err();
        assert!(err.to_string().contains("bogus"), "{err}");
        let text = r#"{"planId":"p","modules":[{"name":"m"}],
            "scenarios":[{"ref":"m/s","module":"m"}],
            "mix":[{"id":"e","scenario":"m/s","weight":1}],
            "workers":[{"rates":[{"mixEntry":"e"}]}],
            "load":{"phases":[{"name":"a","duration":"1s"}]}}"#;
        let err = Plan::parse(text).unwrap_err();
        assert!(err.to_string().contains("exactly one target"), "{err}");
    }

    #[test]
    fn seeds_read_as_strings_or_numbers() {
        let base =
            r#""planId":"p","workers":[{}],"load":{"phases":[{"name":"a","duration":"1s"}]}"#;
        let p = Plan::parse(&format!(r#"{{{base},"seed":"18446744073709551615"}}"#)).unwrap();
        assert_eq!(p.seed, u64::MAX);
        let p = Plan::parse(&format!(r#"{{{base},"seed":7}}"#)).unwrap();
        assert_eq!(p.seed, 7);
    }

    #[test]
    fn criterion_ops_fail_on_nan() {
        for op in [
            CriterionOp::Lt,
            CriterionOp::Le,
            CriterionOp::Gt,
            CriterionOp::Ge,
            CriterionOp::Eq,
            CriterionOp::Ne,
        ] {
            assert!(!op.holds(f64::NAN, 1.0), "{op:?}");
        }
        assert!(CriterionOp::Lt.holds(0.0, 0.001));
        assert!(!CriterionOp::Lt.holds(0.001, 0.001));
        assert!(CriterionOp::Ne.holds(2.0, 1.0));
    }
}
