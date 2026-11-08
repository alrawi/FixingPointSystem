// SPDX-License-Identifier: GPL-3.0-only
//! The configuration of a runtime and its workers.

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use fps_backend_kernel::{ConnectConfig, ListenConfig};
use fps_flow::function::NoFunctions;
use fps_flow::program::Flow;
use fps_flow::{Scenario, TemplateFn};
use fps_hosts::{HostPool, HostSelector};
use fps_interp::Limits;
use fps_sched::ScheduleConfig;

use crate::RuntimeError;

/// Builds the module functions of one session.
///
/// Every session gets its own value, so module state never crosses sessions
/// or workers.
pub type FunctionFactory = Arc<dyn Fn() -> Box<dyn TemplateFn> + Send + Sync>;

/// One flow of a compiled scenario, with what its sessions need besides the
/// transport.
#[derive(Clone)]
pub struct FlowRef {
    /// The compiled scenario after overrides and constant folding. Workers
    /// only read it.
    pub scenario: Arc<Scenario>,
    /// The index of the flow in `scenario.flows`.
    pub flow: usize,
    /// The module functions of each session.
    pub functions: FunctionFactory,
    /// The interpreter limits of each session.
    pub limits: Limits,
    /// The name of the mix entry that the flow serves, under which the
    /// runtime registers the rate handles of its client workers in
    /// [`crate::RuntimeHandle::rate_control`]. `None` uses the scenario name.
    /// Set it when two mix entries run the same scenario.
    pub entry: Option<String>,
}

impl FlowRef {
    /// Returns a reference to flow `flow` of `scenario` without module
    /// functions and with default limits.
    pub fn new(scenario: Arc<Scenario>, flow: usize) -> Self {
        Self {
            scenario,
            flow,
            functions: Arc::new(|| Box::new(NoFunctions)),
            limits: Limits::default(),
            entry: None,
        }
    }

    /// Returns the flow, or an error when the index is out of range.
    pub(crate) fn get(&self) -> Result<&Flow, RuntimeError> {
        self.scenario
            .flows
            .get(self.flow)
            .ok_or_else(|| RuntimeError::Config {
                detail: format!(
                    "scenario {} has no flow at index {}",
                    self.scenario.name, self.flow
                ),
            })
    }
}

impl fmt::Debug for FlowRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FlowRef")
            .field("scenario", &self.scenario.name)
            .field("entry", &self.entry)
            .field("flow", &self.flow)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

/// The source hosts of a client worker, which is one worker's range of a host
/// pool and the selector over it.
#[derive(Debug, Clone)]
pub struct SourceHosts {
    /// The host pool, which maps host indexes to addresses.
    pub pool: HostPool,
    /// The selector, which maps a session id to a host index of the worker's
    /// range.
    pub selector: HostSelector,
    /// True to take IPv6 source addresses, false for IPv4.
    pub ipv6: bool,
}

/// The configuration of a client worker.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// The flow whose client side the worker runs.
    pub flow: FlowRef,
    /// The session schedule of the worker. Its stream should be unique per
    /// worker, see [`fps_sched::stream_id`].
    pub schedule: ScheduleConfig,
    /// The server addresses for a `connect` without `host`. Session `s` uses
    /// address `s mod len`.
    pub servers: Vec<IpAddr>,
    /// A port that replaces the rendered `connect` port, or `None` to use the
    /// rendered one.
    pub port: Option<u16>,
    /// The source hosts.
    pub sources: SourceHosts,
    /// The kernel connector configuration.
    pub connect: ConnectConfig,
}

/// The configuration of a server worker.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// The flow whose server side the worker runs.
    pub flow: FlowRef,
    /// The addresses to listen on. Port 0 lets the kernel pick a port, which
    /// [`crate::RuntimeHandle::listen_addrs`] reports.
    pub listen: Vec<SocketAddr>,
    /// The kernel listener configuration.
    pub config: ListenConfig,
}

/// What a worker does.
#[derive(Debug, Clone)]
pub enum Role {
    /// The worker opens sessions on a schedule.
    Client(Box<ClientConfig>),
    /// The worker accepts sessions.
    Server(ServerConfig),
}

/// The configuration of one worker thread.
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// The worker id, unique within the runtime. It names the statistics
    /// shard and sits in the high bits of every session id.
    pub id: u32,
    /// The core to pin the thread to, or `None` to leave it unpinned.
    pub core: Option<usize>,
    /// What the worker does.
    pub role: Role,
}

/// The configuration of a runtime.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// The run seed, from which every session generator derives.
    pub seed: u64,
    /// The interval between two snapshots of a worker.
    pub snapshot_interval: Duration,
    /// The workers.
    pub workers: Vec<WorkerConfig>,
}

impl RuntimeConfig {
    /// Returns a configuration with a one second snapshot interval.
    pub fn new(seed: u64, workers: Vec<WorkerConfig>) -> Self {
        Self {
            seed,
            snapshot_interval: Duration::from_secs(1),
            workers,
        }
    }

    /// Checks worker ids, flows, and addresses before any thread starts.
    pub(crate) fn validate(&self) -> Result<(), RuntimeError> {
        if self.workers.is_empty() {
            return Err(config("the runtime has no workers"));
        }
        if self.snapshot_interval.is_zero() {
            return Err(config("the snapshot interval must be positive"));
        }
        let mut ids: Vec<u32> = self.workers.iter().map(|w| w.id).collect();
        ids.sort_unstable();
        if let Some(w) = ids.windows(2).find(|w| w[0] == w[1]) {
            return Err(config(&format!("worker id {} appears twice", w[0])));
        }
        for w in &self.workers {
            if w.id >= 1 << crate::opener::WORKER_ID_BITS {
                return Err(config(&format!(
                    "worker id {} does not fit in {} bits",
                    w.id,
                    crate::opener::WORKER_ID_BITS
                )));
            }
            match &w.role {
                Role::Client(c) => {
                    c.flow.get()?;
                    if c.servers.is_empty() && c.flow.get()?.client.host.is_none() {
                        return Err(config(&format!(
                            "client worker {} has no server address and its connect has no host",
                            w.id
                        )));
                    }
                }
                Role::Server(s) => {
                    if s.flow.get()?.server.is_none() {
                        return Err(config(&format!(
                            "server worker {}: flow {} has no server side",
                            w.id,
                            s.flow.get()?.id
                        )));
                    }
                    if s.listen.is_empty() {
                        return Err(config(&format!(
                            "server worker {} has no listen address",
                            w.id
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

fn config(detail: &str) -> RuntimeError {
    RuntimeError::Config {
        detail: detail.to_owned(),
    }
}
