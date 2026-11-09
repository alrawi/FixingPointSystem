// SPDX-License-Identifier: GPL-3.0-only
//! Turning a plan into worker configurations.
//!
//! Each plan worker becomes several engine workers, because an engine worker
//! runs one side of one flow. For every rate of a plan worker the agent adds
//! a client worker, and for every mix entry whose server pool has a host
//! range on a plan worker the agent adds a server worker. The engine worker id
//! is the plan worker id shifted left by 8 bits, plus the mix index shifted
//! left by 1 bit, plus 1 for the server side, so ids stay unique and every
//! session id still names its plan worker. Every engine worker pins to the
//! core of its plan worker.
//!
//! A client worker takes its source addresses from the worker's range of the
//! client pool and sends to the worker's range of the server pool, so a
//! session stays on its worker. Its schedule draws from the stream of its
//! plan worker and mix index. A server worker listens on every address of the
//! worker's range of the server pool at the port of the scenario's `accept`.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use fps_backend_kernel::{ConnectConfig, ListenConfig};
use fps_flow::program::L4;
use fps_flow::Scenario;
use fps_hosts::{HostPool, HostRange, HostSelector, PoolSpec, Selection};
use fps_runtime::{ClientConfig, FlowRef, Role, ServerConfig, SourceHosts, WorkerConfig};
use fps_sched::{stream_id, LoadProfile, Objective, Phase, ScheduleConfig, Shape};

use crate::modules::{with_params, LoadedModules};
use crate::plan::{
    Backend, HostPoolSlice, HostRole, PhaseShape, Plan, Topology, WorkerPlan, WorkerRate,
};
use crate::run::RunError;

/// Which sides of the plan this process runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sides {
    /// Client and server workers, as the topology allows.
    Both,
    /// Only client workers.
    Client,
    /// Only server workers.
    Server,
}

impl Sides {
    /// Parses `both`, `client`, or `server`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "both" => Some(Sides::Both),
            "client" => Some(Sides::Client),
            "server" => Some(Sides::Server),
            _ => None,
        }
    }

    fn client(self) -> bool {
        self != Sides::Server
    }

    fn server(self) -> bool {
        self != Sides::Client
    }
}

/// The engine workers of one plan, split by transport and side.
#[derive(Debug, Default)]
pub struct Assembly {
    /// TCP client workers, for `fps-runtime`.
    pub tcp_clients: Vec<WorkerConfig>,
    /// TCP server workers, for `fps-runtime`.
    pub tcp_servers: Vec<WorkerConfig>,
    /// UDP client workers.
    pub udp_clients: Vec<WorkerConfig>,
    /// UDP server workers.
    pub udp_servers: Vec<WorkerConfig>,
}

impl Assembly {
    /// Returns true when the assembly has a client worker.
    pub fn has_clients(&self) -> bool {
        !(self.tcp_clients.is_empty() && self.udp_clients.is_empty())
    }

    /// Returns true when the assembly has a server worker.
    pub fn has_servers(&self) -> bool {
        !(self.tcp_servers.is_empty() && self.udp_servers.is_empty())
    }
}

/// Returns the engine worker id of one side of one mix entry on one plan
/// worker.
pub fn engine_worker_id(plan_worker: u32, mix_index: usize, server: bool) -> Option<u32> {
    if plan_worker >= 256 || mix_index >= 128 {
        return None;
    }
    // Both values are range checked above.
    let mix = u32::try_from(mix_index).ok()?;
    Some(plan_worker << 8 | mix << 1 | u32::from(server))
}

/// Returns the load profile of the plan phases.
pub fn load_profile(plan: &Plan) -> Result<LoadProfile, RunError> {
    let phases = plan
        .load
        .phases
        .iter()
        .map(|p| {
            let shape = match p.shape {
                PhaseShape::Hold => Shape::Hold,
                PhaseShape::Linear => Shape::Linear,
                PhaseShape::Step => Shape::Step { steps: p.steps },
                PhaseShape::Unspecified => {
                    return Err(RunError::rule(format!("phase {} has no shape", p.name)))
                }
            };
            Ok(Phase::new(
                p.name.clone(),
                p.duration,
                shape,
                p.start_fraction,
                p.end_fraction,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    LoadProfile::new(phases).map_err(|e| RunError::rule(format!("load phases: {e}")))
}

fn pool_of(slice: &HostPoolSlice) -> Result<HostPool, RunError> {
    let opt = |s: &str| (!s.is_empty()).then(|| s.to_owned());
    let (v4, v6, gw, mac) = (
        opt(&slice.ipv4_prefix),
        opt(&slice.ipv6_prefix),
        opt(&slice.gateway),
        opt(&slice.mac_base),
    );
    HostPool::parse(&PoolSpec {
        name: &slice.name,
        ipv4: v4.as_deref(),
        ipv6: v6.as_deref(),
        count: slice.pool_count,
        gateway: gw.as_deref(),
        mac_base: mac.as_deref(),
    })
    .map_err(|e| RunError::rule(format!("host pool {}: {e}", slice.name)))
}

/// The parsed host pools of a plan with the slice of this agent.
struct Pools<'p> {
    plan: &'p Plan,
    parsed: HashMap<String, HostPool>,
}

impl<'p> Pools<'p> {
    fn new(plan: &'p Plan) -> Result<Self, RunError> {
        let mut parsed = HashMap::new();
        for p in &plan.host_pools {
            parsed.insert(p.name.clone(), pool_of(p)?);
        }
        Ok(Self { plan, parsed })
    }

    fn get(&self, name: &str, role: HostRole) -> Result<(&'p HostPoolSlice, &HostPool), RunError> {
        let slice = self.plan.pool(name, role)?;
        let pool = self
            .parsed
            .get(&slice.name)
            .ok_or_else(|| RunError::rule(format!("host pool {} is not parsed", slice.name)))?;
        Ok((slice, pool))
    }

    /// The range of `worker` in the pool, or `None` when it owns none.
    fn range(worker: &WorkerPlan, slice: &HostPoolSlice) -> Option<HostRange> {
        worker
            .hosts
            .iter()
            .find(|h| h.pool == slice.name)
            .map(|h| HostRange::new(h.offset, h.count))
    }

    /// The addresses of `range`, IPv4 when the pool has an IPv4 prefix.
    fn addresses(pool: &HostPool, range: HostRange) -> Result<Vec<IpAddr>, RunError> {
        range
            .indexes()
            .map(|i| {
                let a = if pool.ipv4().is_some() {
                    pool.ipv4_addr(i).map(|a| a.map(IpAddr::V4))
                } else {
                    pool.ipv6_addr(i).map(|a| a.map(IpAddr::V6))
                };
                match a {
                    Ok(Some(a)) => Ok(a),
                    Ok(None) => Err(RunError::rule(format!(
                        "pool {} has no prefix",
                        pool.name()
                    ))),
                    Err(e) => Err(RunError::rule(format!("pool {}: {e}", pool.name()))),
                }
            })
            .collect()
    }
}

/// Builds the engine workers of `plan` for `sides`.
pub fn assemble(plan: &Plan, loaded: &LoadedModules, sides: Sides) -> Result<Assembly, RunError> {
    if plan.backend != Backend::Kernel {
        return Err(RunError::Unimplemented(format!(
            "backend {:?}; the standalone mode runs BACKEND_KERNEL only",
            plan.backend
        )));
    }
    let (clients_allowed, servers_allowed) = match plan.topology {
        Topology::AppSim => (true, true),
        Topology::ClientSim => (true, false),
        Topology::ServerSim => (false, true),
        Topology::Unspecified => return Err(RunError::rule("topology is required".into())),
    };
    let profile = load_profile(plan)?;
    let pools = Pools::new(plan)?;
    let functions = loaded.functions.factory();

    // The compiled scenario of every mix entry, after its parameters.
    let mut flows: Vec<(Arc<Scenario>, L4)> = Vec::with_capacity(plan.mix.len());
    for e in &plan.mix {
        let base = loaded
            .scenarios
            .get(&e.scenario)
            .ok_or_else(|| RunError::rule(format!("scenario {} is not loaded", e.scenario)))?;
        let s = with_params(&e.id, base, &e.params)?;
        if s.flows.len() != 1 {
            return Err(RunError::Unimplemented(format!(
                "scenario {} has {} flows; the standalone mode runs scenarios with one flow",
                e.scenario,
                s.flows.len()
            )));
        }
        let l4 = s.flows[0].l4;
        flows.push((Arc::new(s), l4));
    }
    let flow_ref = |i: usize| {
        let mut f = FlowRef::new(Arc::clone(&flows[i].0), 0);
        f.functions = Arc::clone(&functions);
        // UpdateRates names mix entries, and two entries may share a scenario.
        f.entry = Some(plan.mix[i].id.clone());
        f
    };
    let external: Vec<IpAddr> = plan
        .external_servers
        .iter()
        .map(|s| {
            s.address.parse::<IpAddr>().map_err(|_| {
                RunError::Unimplemented(format!(
                    "external server {} has address {}; the standalone mode needs an IP address",
                    s.name, s.address
                ))
            })
        })
        .collect::<Result<_, _>>()?;
    let external_port = plan
        .external_servers
        .first()
        .and_then(|s| u16::try_from(s.port).ok())
        .filter(|p| *p != 0);

    let mut out = Assembly::default();
    if sides.client() && clients_allowed {
        for w in &plan.workers {
            for rate in &w.rates {
                let (mix_index, cfg) = client_worker(
                    plan,
                    &pools,
                    w,
                    rate,
                    &profile,
                    &external,
                    external_port,
                    &flow_ref,
                )?;
                match flows[mix_index].1 {
                    L4::Tcp => out.tcp_clients.push(cfg),
                    L4::Udp => out.udp_clients.push(cfg),
                }
            }
        }
    }
    if sides.server() && servers_allowed {
        let mut taken: HashMap<(bool, SocketAddr), String> = HashMap::new();
        for w in &plan.workers {
            for (i, e) in plan.mix.iter().enumerate() {
                let (slice, pool) = pools.get(&e.server_pool, HostRole::Server)?;
                let Some(range) = Pools::range(w, slice) else {
                    continue;
                };
                let (scenario, l4) = &flows[i];
                let Some(server) = scenario.flows[0].server.as_ref() else {
                    continue;
                };
                let port = server
                    .port
                    .as_static()
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .and_then(|s| s.trim().parse::<u16>().ok())
                    .ok_or_else(|| {
                        RunError::Unimplemented(format!(
                            "mix entry {}: the accept port is not a constant port",
                            e.id
                        ))
                    })?;
                let listen: Vec<SocketAddr> = Pools::addresses(pool, range)?
                    .into_iter()
                    .map(|ip| SocketAddr::new(ip, port))
                    .collect();
                let udp = *l4 == L4::Udp;
                for a in &listen {
                    if let Some(other) = taken.insert((udp, *a), e.id.clone()) {
                        return Err(RunError::rule(format!(
                            "mix entries {other} and {} both listen on {} {a}",
                            e.id,
                            if udp { "udp" } else { "tcp" }
                        )));
                    }
                }
                let id = engine_worker_id(w.worker_id, i, true).ok_or_else(too_many)?;
                let cfg = WorkerConfig {
                    id,
                    core: usize::try_from(w.core).ok(),
                    role: Role::Server(ServerConfig {
                        flow: flow_ref(i),
                        listen,
                        config: ListenConfig::default(),
                    }),
                };
                if udp {
                    out.udp_servers.push(cfg);
                } else {
                    out.tcp_servers.push(cfg);
                }
            }
        }
    }
    if !out.has_clients() && !out.has_servers() {
        return Err(RunError::rule(format!(
            "the plan gives this agent no worker for sides {sides:?} in topology {:?}",
            plan.topology
        )));
    }
    Ok(out)
}

fn too_many() -> RunError {
    RunError::Unimplemented(
        "the standalone mode supports worker ids below 256 and at most 128 mix entries".into(),
    )
}

#[allow(clippy::too_many_arguments)]
fn client_worker(
    plan: &Plan,
    pools: &Pools<'_>,
    w: &WorkerPlan,
    rate: &WorkerRate,
    profile: &LoadProfile,
    external: &[IpAddr],
    external_port: Option<u16>,
    flow_ref: &dyn Fn(usize) -> FlowRef,
) -> Result<(usize, WorkerConfig), RunError> {
    let (mix_index, entry) = plan
        .mix
        .iter()
        .enumerate()
        .find(|(_, e)| e.id == rate.mix_entry)
        .ok_or_else(|| RunError::rule(format!("unknown mix entry {}", rate.mix_entry)))?;
    let (cslice, cpool) = pools.get(&entry.client_pool, HostRole::Client)?;
    let crange = Pools::range(w, cslice).ok_or_else(|| {
        RunError::rule(format!(
            "worker {} has a rate for {} but no range of client pool {}",
            w.worker_id, entry.id, cslice.name
        ))
    })?;
    let (servers, port) = if plan.topology == Topology::ClientSim {
        (external.to_vec(), external_port)
    } else {
        let (sslice, spool) = pools.get(&entry.server_pool, HostRole::Server)?;
        let srange =
            Pools::range(w, sslice).unwrap_or_else(|| HostRange::new(sslice.offset, sslice.count));
        (Pools::addresses(spool, srange)?, None)
    };
    let objective = match (rate.sessions_per_second, rate.concurrent_sessions) {
        (Some(r), None) => Objective::SessionsPerSecond(r),
        (None, Some(n)) => Objective::ConcurrentSessions(n),
        _ => {
            return Err(RunError::rule(format!(
                "worker {}: rate of {} needs exactly one target",
                w.worker_id, entry.id
            )))
        }
    };
    let mut schedule = ScheduleConfig::new(objective, profile.clone(), plan.seed);
    let mix = u32::try_from(mix_index).map_err(|_| too_many())?;
    schedule.stream = stream_id(w.worker_id, mix);
    let id = engine_worker_id(w.worker_id, mix_index, false).ok_or_else(too_many)?;
    let selector = HostSelector::new(&cslice.name, crange, Selection::RoundRobin, plan.seed, id)
        .map_err(|e| RunError::rule(format!("worker {}: {e}", w.worker_id)))?;
    let cfg = WorkerConfig {
        id,
        core: usize::try_from(w.core).ok(),
        role: Role::Client(Box::new(ClientConfig {
            flow: flow_ref(mix_index),
            schedule,
            servers,
            port,
            sources: SourceHosts {
                pool: cpool.clone(),
                selector,
                ipv6: cpool.ipv4().is_none(),
            },
            connect: ConnectConfig::default(),
        })),
    };
    Ok((mix_index, cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_worker_ids_are_unique_and_bounded() {
        assert_eq!(engine_worker_id(0, 0, false), Some(0));
        assert_eq!(engine_worker_id(0, 0, true), Some(1));
        assert_eq!(engine_worker_id(1, 1, false), Some(258));
        assert_eq!(engine_worker_id(255, 127, true), Some(65_535));
        assert_eq!(engine_worker_id(256, 0, false), None);
        assert_eq!(engine_worker_id(0, 128, false), None);
    }
}
