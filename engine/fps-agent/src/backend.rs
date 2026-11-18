// SPDX-License-Identifier: GPL-3.0-only
//! Starting the transport backend of a run (AGT-24).
//!
//! The agent selects its backend at boot ([`crate::select`]) and the plan of a
//! run names a backend too. [`resolve`] makes the two agree: the backend that
//! a run starts is the one that the agent selected, and a plan that asks for
//! another one is refused. [`start`] then brings the backend up.
//!
//! * The kernel backend needs nothing before the workers open their sockets.
//! * The AF_XDP backend opens one AF_XDP queue per lane on the port of the
//!   client pool and on the port of the server pool, gives each pair to a
//!   lane of the user space datapath of TRN-27 ([`fps_datapath::Datapath`]),
//!   and starts the lane threads. The own TCP stack then answers ARP and
//!   serves the host pools of the plan on those queues. [`AfXdpBackend`]
//!   keeps the datapath alive and reports, for every port, the mode that the
//!   sockets really bound in ([`BoundPort`]), read back from the kernel and
//!   not taken from the driver table. [`AfXdpBackend::transport`] gives the
//!   workers the datapath as a `fps_runtime::transport::Transport`
//!   ([`crate::transport`]), which `Prepared::start_on` sets on every worker
//!   (AGT-25). The `os_profile` of each host pool becomes the OS profile of
//!   its side in the own stack, one profile for the client side and one for
//!   the server side.
//!
//! # What a plan has to look like on AF_XDP
//!
//! The datapath is one own stack per lane, and the stack wants two links, so
//! the plan needs both arms:
//!
//! * the topology is `TOPOLOGY_APP_SIM`, and the process runs both sides,
//! * the mix uses one client pool and one server pool (several entries may
//!   share them),
//! * the two pools sit on two different ports, and each port names an
//!   interface in `PortBinding`,
//! * both pools are IPv4, untagged or on the same VLAN, and use no DHCP.
//!
//! Anything else is [`BackendError::Unsupported`] with the package that adds
//! it (IPv6 is TRN-30, DHCP is HST-04 on the datapath). TCP and UDP flows both
//! run (TRN-29, AGT-25). The bound mode is available as soon
//! as [`start`] returns, and [`StartedBackend::report`] writes it into the
//! `xdp_driver_mode` of the matching [`PortInfo`] of a [`Capabilities`].

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fps_backend_afxdp::{
    open_port, AfXdpError, BackendKind, BoundMode, FrameIo, OpenedPort, PortConfig, QueueConfig,
    XdpMode as Attach,
};
use fps_datapath::{split_pools, Datapath, DatapathConfig, DatapathError, Lane, LaneReport};
use fps_hosts::HostRange;
use fps_tcp_own::{HostSlice, OsProfile, OwnConfig, Pools};

use crate::assemble::{pool_of, Sides};
use crate::plan::{Backend, HostPoolSlice, HostRole, Plan, Topology};
use crate::proto::{Capabilities, XdpMode};
use crate::run::RunError;
use crate::transport::DatapathTransport;

/// How long a bind retries while the kernel still releases the queue of a
/// socket that a previous run closed. The release runs in a work queue, so a
/// bind right after a close can meet `EBUSY` for a few milliseconds.
const BUSY_RETRY: Duration = Duration::from_secs(2);

/// Why a backend did not start.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// The plan names another backend than the one that the agent selected.
    #[error("the plan asks for backend {asked}, and this agent runs {selected}")]
    Mismatch {
        /// The backend of the plan.
        asked: &'static str,
        /// The backend that the agent selected.
        selected: &'static str,
    },
    /// The plan asks for something that the backend does not serve yet.
    #[error("backend af_xdp: {0}")]
    Unsupported(String),
    /// The plan breaks a rule that the backend needs.
    #[error("backend af_xdp: {0}")]
    Plan(String),
    /// A queue could not be opened or bound.
    #[error("backend af_xdp: open port {port} (interface {interface})")]
    Open {
        /// The port name of the plan.
        port: String,
        /// The interface of the port.
        interface: String,
        /// The failure of the backend crate.
        #[source]
        source: AfXdpError,
    },
    /// The datapath did not start.
    #[error("backend af_xdp: start the datapath")]
    Datapath(#[from] DatapathError),
    /// The thread that serves the listeners of the datapath did not start.
    #[error("backend af_xdp: start the listener thread of the datapath transport")]
    Transport(#[source] std::io::Error),
}

/// Returns the name of a plan backend for messages.
fn plan_backend_name(b: Backend) -> &'static str {
    match b {
        Backend::Unspecified => "unspecified",
        Backend::Kernel => "kernel",
        Backend::AfXdp => "af_xdp",
        Backend::Dpdk => "dpdk",
    }
}

/// Decides which backend a run starts.
///
/// `selected` is the backend that the agent selected at boot, or `None` in
/// the standalone mode, where no selection ran and the plan names the
/// backend. With a selection the plan must name the same backend or leave it
/// unspecified. Without one the plan must name the kernel backend or AF_XDP.
///
/// # Errors
///
/// Returns [`RunError::Unimplemented`] for a backend that the agent cannot
/// start (DPDK, or no backend named in the standalone mode), and
/// [`BackendError::Mismatch`] when the plan and the selection disagree.
pub fn resolve(plan: Backend, selected: Option<BackendKind>) -> Result<BackendKind, RunError> {
    let named = match plan {
        Backend::Kernel => Some(BackendKind::Kernel),
        Backend::AfXdp => Some(BackendKind::AfXdp),
        Backend::Unspecified | Backend::Dpdk => None,
    };
    match (selected, named) {
        (Some(s), Some(n)) if s != n => Err(BackendError::Mismatch {
            asked: n.name(),
            selected: s.name(),
        }
        .into()),
        (Some(s), _) if plan != Backend::Dpdk => Ok(s),
        (None, Some(n)) => Ok(n),
        _ => Err(RunError::Unimplemented(format!(
            "backend {}; the agent starts BACKEND_KERNEL and BACKEND_AF_XDP",
            plan_backend_name(plan)
        ))),
    }
}

/// The mode of one bound port as the sockets report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundPort {
    /// The port name of the plan.
    pub name: String,
    /// The local interface.
    pub interface: String,
    /// The role of the pool that sits on the port.
    pub role: HostRole,
    /// How the XDP program is attached.
    pub attach: Attach,
    /// The mode of each queue, in the order of the lanes.
    pub queues: Vec<BoundMode>,
}

impl BoundPort {
    /// The contract value: generic when the program runs in the generic hook,
    /// else zero copy when every queue bound in zero copy, else native.
    #[must_use]
    pub fn xdp_mode(&self) -> XdpMode {
        mode_of(self.attach, &self.queues)
    }
}

/// Maps the attach mode of a port and the bind modes of its queues to the
/// contract enum. A port with queues in different modes reports the weakest.
#[must_use]
pub fn mode_of(attach: Attach, queues: &[BoundMode]) -> XdpMode {
    match attach {
        Attach::Generic => XdpMode::Generic,
        Attach::Native => {
            if !queues.is_empty() && queues.iter().all(|q| *q == BoundMode::ZeroCopy) {
                XdpMode::ZeroCopy
            } else {
                XdpMode::Native
            }
        }
    }
}

impl fmt::Display for BoundPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let attach = match self.attach {
            Attach::Native => "native",
            Attach::Generic => "generic",
        };
        let queues: Vec<&str> = self
            .queues
            .iter()
            .map(|q| match q {
                BoundMode::ZeroCopy => "zerocopy",
                BoundMode::Copy => "copy",
            })
            .collect();
        write!(
            f,
            "port {} interface {} attach {attach} queues {} xdp_driver_mode {}",
            self.name,
            self.interface,
            queues.join(","),
            self.xdp_mode().proto_name()
        )
    }
}

/// The settings of the AF_XDP backend.
#[derive(Debug, Clone)]
pub struct AfXdpSettings {
    /// The lanes of the datapath. Lane `k` uses queue `k` of each port.
    pub lanes: usize,
    /// The queue settings, which also choose the bind mode (the default asks
    /// for zero copy and falls back to copy mode).
    pub queue: QueueConfig,
}

impl Default for AfXdpSettings {
    /// One lane and the default queue settings.
    fn default() -> Self {
        Self {
            lanes: 1,
            queue: QueueConfig::default(),
        }
    }
}

/// The AF_XDP backend: the datapath on the queues of the plan ports.
#[derive(Debug)]
pub struct AfXdpBackend {
    datapath: Arc<Datapath>,
    transport: DatapathTransport,
    ports: Vec<BoundPort>,
}

impl AfXdpBackend {
    /// The datapath, whose [`Datapath::connector`] and [`Datapath::listen`]
    /// open the sessions of the plan hosts. A caller that listens through it
    /// directly and also through [`AfXdpBackend::transport`] cannot use a port
    /// twice, because the datapath serves a port once.
    #[must_use]
    pub fn datapath(&self) -> &Datapath {
        &self.datapath
    }

    /// The datapath as the [`fps_runtime::transport::Transport`] that the
    /// workers of a run open their sessions with (AGT-25). Every call returns
    /// a handle on the same transport.
    #[must_use]
    pub fn transport(&self) -> DatapathTransport {
        self.transport.clone()
    }

    /// The ports with the modes that their sockets bound in.
    #[must_use]
    pub fn ports(&self) -> &[BoundPort] {
        &self.ports
    }

    /// Stops the listeners of the transport and the lane threads, releases the
    /// queues, and returns what each lane did. The workers that opened
    /// sessions through the transport must have ended, or their sessions stop
    /// at once.
    #[must_use]
    pub fn stop(self) -> Vec<LaneReport> {
        self.transport.shutdown();
        match Arc::try_unwrap(self.datapath) {
            Ok(datapath) => datapath.stop(),
            // Nobody else holds the datapath after the shutdown. The drop of
            // the last handle stops the lanes all the same.
            Err(_) => Vec::new(),
        }
    }
}

/// The backend of a run, started.
#[derive(Debug)]
pub enum StartedBackend {
    /// The kernel backend. The workers open kernel sockets themselves.
    Kernel,
    /// The AF_XDP backend with its datapath.
    AfXdp(Box<AfXdpBackend>),
}

impl StartedBackend {
    /// The kind of backend.
    #[must_use]
    pub fn kind(&self) -> BackendKind {
        match self {
            Self::Kernel => BackendKind::Kernel,
            Self::AfXdp(_) => BackendKind::AfXdp,
        }
    }

    /// The bound ports, empty for the kernel backend.
    #[must_use]
    pub fn ports(&self) -> &[BoundPort] {
        match self {
            Self::Kernel => &[],
            Self::AfXdp(a) => a.ports(),
        }
    }

    /// Stops the backend and returns what each lane of the datapath did: no
    /// lane for the kernel backend.
    #[must_use]
    pub fn stop(self) -> Vec<LaneReport> {
        match self {
            Self::Kernel => Vec::new(),
            Self::AfXdp(a) => a.stop(),
        }
    }

    /// Writes the bound mode of every port into `caps`: the `PortInfo` whose
    /// name is the interface of a bound port gets that port's
    /// `xdp_driver_mode`, which replaces the driver table hint. A backend
    /// other than AF_XDP changes nothing, and the contract says that such a
    /// report carries no mode.
    pub fn report(&self, caps: &mut Capabilities) {
        for bound in self.ports() {
            let mode = bound.xdp_mode();
            for info in caps.ports.iter_mut().filter(|p| p.name == bound.interface) {
                info.xdp_driver_mode = mode;
            }
        }
    }
}

/// What a run needs from a plan to start the datapath.
struct Layout<'p> {
    client: &'p HostPoolSlice,
    server: &'p HostPoolSlice,
    client_interface: &'p str,
    server_interface: &'p str,
    vlan: Option<u16>,
}

fn plan_error(text: impl Into<String>) -> BackendError {
    BackendError::Plan(text.into())
}

fn unsupported(text: impl Into<String>) -> BackendError {
    BackendError::Unsupported(text.into())
}

/// Finds the one pool of `role` that the mix uses.
fn mix_pool(plan: &Plan, role: HostRole) -> Result<&HostPoolSlice, BackendError> {
    let names: BTreeSet<&str> = plan
        .mix
        .iter()
        .map(|e| match role {
            HostRole::Server => e.server_pool.as_str(),
            _ => e.client_pool.as_str(),
        })
        .collect();
    let mut found: BTreeSet<&str> = BTreeSet::new();
    let mut slice = None;
    for name in names {
        let s = plan
            .pool(name, role)
            .map_err(|e| plan_error(e.to_string()))?;
        if found.insert(s.name.as_str()) {
            slice = Some(s);
        }
    }
    match (found.len(), slice) {
        (1, Some(s)) => Ok(s),
        (0, _) => Err(plan_error(format!("the mix uses no {role:?} host pool"))),
        (n, _) => Err(unsupported(format!(
            "the mix uses {n} {role:?} host pools, and the datapath serves one per side"
        ))),
    }
}

fn interface_of<'p>(plan: &'p Plan, slice: &HostPoolSlice) -> Result<&'p str, BackendError> {
    plan.ports
        .iter()
        .find(|p| p.name == slice.port)
        .map(|p| p.interface_name.as_str())
        .filter(|i| !i.is_empty())
        .ok_or_else(|| {
            plan_error(format!(
                "host pool {} sits on port '{}', which names no interface",
                slice.name, slice.port
            ))
        })
}

fn layout(plan: &Plan, sides: Sides) -> Result<Layout<'_>, BackendError> {
    if plan.topology != Topology::AppSim {
        return Err(unsupported(format!(
            "topology {:?}; the datapath needs a client arm and a server arm, so it runs TOPOLOGY_APP_SIM",
            plan.topology
        )));
    }
    if sides != Sides::Both {
        return Err(unsupported(format!(
            "sides {sides:?}; one process runs both arms of the datapath"
        )));
    }
    let client = mix_pool(plan, HostRole::Client)?;
    let server = mix_pool(plan, HostRole::Server)?;
    let client_interface = interface_of(plan, client)?;
    let server_interface = interface_of(plan, server)?;
    if client_interface == server_interface {
        return Err(plan_error(format!(
            "host pools {} and {} sit on the same interface {client_interface}, and each arm needs a port of its own",
            client.name, server.name
        )));
    }
    for s in [client, server] {
        if s.dhcp4.is_some() {
            return Err(unsupported(format!(
                "host pool {} leases its address through DHCP (HST-04 on the datapath)",
                s.name
            )));
        }
        if s.ipv4_prefix.is_empty() {
            return Err(unsupported(format!(
                "host pool {} has no IPv4 prefix; IPv6 is TRN-30",
                s.name
            )));
        }
    }
    if client.vlan != server.vlan {
        return Err(plan_error(format!(
            "host pools {} and {} name VLAN {} and {}, and the lane has one",
            client.name, server.name, client.vlan, server.vlan
        )));
    }
    let vlan = match client.vlan {
        0 => None,
        v => Some(u16::try_from(v).map_err(|_| plan_error(format!("VLAN {v} is not valid")))?),
    };
    Ok(Layout {
        client,
        server,
        client_interface,
        server_interface,
        vlan,
    })
}

fn slice_of(slice: &HostPoolSlice) -> Result<HostSlice, BackendError> {
    let pool = pool_of(slice).map_err(|e| plan_error(e.to_string()))?;
    Ok(HostSlice {
        pool,
        range: HostRange::new(slice.offset, slice.count),
    })
}

/// The operating system profile that a host pool names, for the own stack.
///
/// An empty name leaves the side without a profile, which keeps the defaults
/// of the stack. A name that the stack does not know is a plan error, so a
/// typo does not run with the wrong SYN options unseen.
fn profile_of(slice: &HostPoolSlice) -> Result<Option<OsProfile>, BackendError> {
    if slice.os_profile.is_empty() {
        return Ok(None);
    }
    OsProfile::builtin(&slice.os_profile)
        .map(Some)
        .map_err(|e| {
            plan_error(format!(
                "host pool {}: os_profile '{}': {e}",
                slice.name, slice.os_profile
            ))
        })
}

/// The settings of the own stack that the plan decides: one OS profile for
/// the client side and one for the server side, from the host pool of each
/// (TRN-28).
fn own_config(l: &Layout<'_>) -> Result<OwnConfig, BackendError> {
    Ok(OwnConfig {
        client_os_profile: profile_of(l.client)?,
        server_os_profile: profile_of(l.server)?,
        ..OwnConfig::default()
    })
}

/// Opens a port, and again for a short time while the kernel still holds the
/// queue of an earlier socket.
fn open_retrying(config: &PortConfig, port: &str) -> Result<OpenedPort, BackendError> {
    let deadline = Instant::now() + BUSY_RETRY;
    loop {
        match open_port(config) {
            Ok(p) => return Ok(p),
            Err(e) if e.raw_os_error() == Some(16) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(source) => {
                return Err(BackendError::Open {
                    port: port.to_owned(),
                    interface: config.interface.clone(),
                    source,
                })
            }
        }
    }
}

fn bound(name: &str, role: HostRole, p: &OpenedPort) -> BoundPort {
    BoundPort {
        name: name.to_owned(),
        interface: p.info.interface.clone(),
        role,
        attach: p.info.xdp_mode,
        queues: p.info.queues.iter().map(|q| q.bound).collect(),
    }
}

/// Starts the AF_XDP backend for `plan`: opens the queues, splits the host
/// pools into lanes, and starts the datapath.
///
/// # Errors
///
/// Returns [`BackendError::Unsupported`] or [`BackendError::Plan`] for a plan
/// that the datapath cannot serve, [`BackendError::Open`] when a queue does
/// not open (no privilege, no such interface, an XDP program already on the
/// device), and [`BackendError::Datapath`] when the lanes do not start.
pub fn start_af_xdp(
    plan: &Plan,
    sides: Sides,
    settings: &AfXdpSettings,
) -> Result<AfXdpBackend, BackendError> {
    let l = layout(plan, sides)?;
    // Refused here, before a queue binds, when a pool names an unknown profile.
    own_config(&l)?;
    let port_config = |interface: &str| PortConfig {
        interface: interface.to_owned(),
        queues: (0..u32::try_from(settings.lanes).unwrap_or(u32::MAX)).collect(),
        queue: settings.queue.clone(),
    };
    let client = open_retrying(&port_config(l.client_interface), &l.client.port)?;
    let server = open_retrying(&port_config(l.server_interface), &l.server.port)?;
    let ports = vec![
        bound(&l.client.port, HostRole::Client, &client),
        bound(&l.server.port, HostRole::Server, &server),
    ];
    let queues = client
        .queues
        .into_iter()
        .zip(server.queues)
        .map(|(c, s)| {
            (
                Box::new(c) as Box<dyn FrameIo>,
                Box::new(s) as Box<dyn FrameIo>,
            )
        })
        .collect();
    launch(&l, settings.lanes, queues, ports)
}

/// Starts the datapath of a plan on queues that the caller opened, one pair
/// of a client queue and a server queue for each lane.
///
/// [`start_af_xdp`] opens AF_XDP sockets and calls the same code. This
/// function takes any [`FrameIo`], so the wiring of a run can be tested on
/// in memory links (`fps_backend_afxdp::memory::frame_pair`) without root and
/// without a device. The backend that comes back reports no bound port,
/// because no socket was bound.
///
/// # Errors
///
/// As [`start_af_xdp`], except [`BackendError::Open`]. A list that holds no
/// pair gives [`BackendError::Plan`].
pub fn start_on_queues(
    plan: &Plan,
    sides: Sides,
    queues: Vec<(Box<dyn FrameIo>, Box<dyn FrameIo>)>,
) -> Result<AfXdpBackend, BackendError> {
    let l = layout(plan, sides)?;
    let lanes = queues.len();
    launch(&l, lanes, queues, Vec::new())
}

/// Cuts the pools of `l` into `lanes` slices, pairs each with its queues, and
/// starts the datapath and its transport.
fn launch(
    l: &Layout<'_>,
    lanes: usize,
    queues: Vec<(Box<dyn FrameIo>, Box<dyn FrameIo>)>,
    ports: Vec<BoundPort>,
) -> Result<AfXdpBackend, BackendError> {
    let pools = Pools {
        client: slice_of(l.client)?,
        server: slice_of(l.server)?,
        vlan: l.vlan,
    };
    let own = own_config(l)?;
    if queues.is_empty() {
        return Err(plan_error("a datapath needs at least one lane"));
    }
    let slices = split_pools(&pools, lanes)?;
    if slices.len() != queues.len() {
        return Err(plan_error("a port opened fewer queues than lanes"));
    }
    let built: Vec<Lane> = slices
        .into_iter()
        .zip(queues)
        .map(|(pools, (client, server))| Lane {
            client,
            server,
            pools,
        })
        .collect();
    let config = DatapathConfig {
        own,
        ..DatapathConfig::default()
    };
    let datapath = Arc::new(Datapath::start(built, config)?);
    let transport =
        DatapathTransport::new(Arc::clone(&datapath)).map_err(BackendError::Transport)?;
    Ok(AfXdpBackend {
        datapath,
        transport,
        ports,
    })
}

/// Starts `kind` for `plan`.
///
/// # Errors
///
/// Returns the [`BackendError`] of [`start_af_xdp`] for AF_XDP. The kernel
/// backend does not fail here.
pub fn start(
    kind: BackendKind,
    plan: &Plan,
    sides: Sides,
    settings: &AfXdpSettings,
) -> Result<StartedBackend, BackendError> {
    match kind {
        BackendKind::AfXdp => {
            start_af_xdp(plan, sides, settings).map(|b| StartedBackend::AfXdp(Box::new(b)))
        }
        _ => Ok(StartedBackend::Kernel),
    }
}

/// Checks that `plan` has the shape that the AF_XDP backend serves, without
/// opening anything. [`crate::run::prepare_on`] calls it, so a plan that the
/// datapath cannot serve fails at `prepare` and not at `start`.
///
/// # Errors
///
/// Returns the [`BackendError`] that [`start_af_xdp`] would return before it
/// opens a queue.
pub fn check_af_xdp(plan: &Plan, sides: Sides) -> Result<(), BackendError> {
    let l = layout(plan, sides)?;
    slice_of(l.client)?;
    slice_of(l.server)?;
    own_config(&l)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{MixEntry, PortBinding};

    fn slice(name: &str, port: &str, role: HostRole, prefix: &str) -> HostPoolSlice {
        HostPoolSlice {
            name: name.into(),
            port: port.into(),
            role,
            ipv4_prefix: prefix.into(),
            pool_count: 8,
            count: 8,
            ..HostPoolSlice::default()
        }
    }

    fn plan() -> Plan {
        Plan {
            topology: Topology::AppSim,
            backend: Backend::AfXdp,
            ports: vec![
                PortBinding {
                    name: "c".into(),
                    interface_name: "dpa".into(),
                    mtu: 0,
                },
                PortBinding {
                    name: "s".into(),
                    interface_name: "dpb".into(),
                    mtu: 0,
                },
            ],
            host_pools: vec![
                slice("clients", "c", HostRole::Client, "10.1.0.0/24"),
                slice("servers", "s", HostRole::Server, "10.2.0.0/24"),
            ],
            mix: vec![MixEntry {
                id: "web".into(),
                client_pool: "clients".into(),
                server_pool: "servers".into(),
                ..MixEntry::default()
            }],
            ..Plan::default()
        }
    }

    #[test]
    fn the_run_starts_the_selected_backend_and_not_the_plan_backend() {
        use BackendKind::{AfXdp, Kernel};
        // With a selection the plan may leave the backend open.
        assert_eq!(resolve(Backend::Unspecified, Some(AfXdp)).unwrap(), AfXdp);
        assert_eq!(resolve(Backend::Unspecified, Some(Kernel)).unwrap(), Kernel);
        assert_eq!(resolve(Backend::Kernel, Some(Kernel)).unwrap(), Kernel);
        assert_eq!(resolve(Backend::AfXdp, Some(AfXdp)).unwrap(), AfXdp);
        // Without one the plan decides, as the standalone mode always did.
        assert_eq!(resolve(Backend::Kernel, None).unwrap(), Kernel);
        assert_eq!(resolve(Backend::AfXdp, None).unwrap(), AfXdp);
    }

    #[test]
    fn a_plan_for_another_backend_than_the_selected_one_is_refused() {
        let e = resolve(Backend::AfXdp, Some(BackendKind::Kernel)).unwrap_err();
        assert!(
            matches!(e, RunError::Backend(BackendError::Mismatch { .. })),
            "{e}"
        );
        assert_eq!(
            e.to_string(),
            "the plan asks for backend af_xdp, and this agent runs kernel"
        );
        let e = resolve(Backend::Kernel, Some(BackendKind::AfXdp)).unwrap_err();
        assert!(e.to_string().contains("asks for backend kernel"), "{e}");
    }

    #[test]
    fn dpdk_and_an_unnamed_backend_in_the_standalone_mode_are_unimplemented() {
        for (b, s) in [
            (Backend::Dpdk, None),
            (Backend::Dpdk, Some(BackendKind::Kernel)),
            (Backend::Unspecified, None),
        ] {
            let e = resolve(b, s).unwrap_err();
            assert!(matches!(e, RunError::Unimplemented(_)), "{b:?} {s:?}: {e}");
        }
    }

    #[test]
    fn a_plan_in_the_supported_shape_passes_the_check() {
        check_af_xdp(&plan(), Sides::Both).unwrap();
        let p = plan();
        let l = layout(&p, Sides::Both).unwrap();
        assert_eq!((l.client_interface, l.server_interface), ("dpa", "dpb"));
        assert_eq!(l.vlan, None);
    }

    #[test]
    fn the_datapath_refuses_the_shapes_it_does_not_serve_and_says_which_package_adds_them() {
        let err = |p: &Plan, sides| check_af_xdp(p, sides).unwrap_err().to_string();

        let mut p = plan();
        p.topology = Topology::ClientSim;
        assert!(err(&p, Sides::Both).contains("TOPOLOGY_APP_SIM"));

        assert!(err(&plan(), Sides::Client).contains("both arms"));

        let mut p = plan();
        p.host_pools[0].dhcp4 = Some(crate::plan::Dhcp4 {});
        p.host_pools[0].ipv4_prefix.clear();
        assert!(err(&p, Sides::Both).contains("DHCP"));

        let mut p = plan();
        p.host_pools[1].ipv4_prefix.clear();
        assert!(err(&p, Sides::Both).contains("TRN-30"));

        let mut p = plan();
        p.host_pools[1].port = "c".into();
        assert!(err(&p, Sides::Both).contains("same interface"));

        let mut p = plan();
        p.ports[1].interface_name.clear();
        assert!(err(&p, Sides::Both).contains("names no interface"));

        let mut p = plan();
        p.host_pools[0].vlan = 7;
        assert!(err(&p, Sides::Both).contains("VLAN 7 and 0"));

        let mut p = plan();
        p.host_pools
            .push(slice("more", "c", HostRole::Client, "10.3.0.0/24"));
        p.mix.push(MixEntry {
            id: "other".into(),
            client_pool: "more".into(),
            server_pool: "servers".into(),
            ..MixEntry::default()
        });
        assert!(err(&p, Sides::Both).contains("2 Client host pools"));
    }

    #[test]
    fn each_pool_names_the_os_profile_of_its_side() {
        let mut p = plan();
        let l = layout(&p, Sides::Both).unwrap();
        let none = own_config(&l).unwrap();
        assert!(none.client_os_profile.is_none() && none.server_os_profile.is_none());

        p.host_pools[0].os_profile = "windows11".into();
        p.host_pools[1].os_profile = "macos14".into();
        let l = layout(&p, Sides::Both).unwrap();
        let own = own_config(&l).unwrap();
        assert_eq!(own.client_os_profile.map(|o| o.name), Some("windows11"));
        assert_eq!(own.server_os_profile.map(|o| o.name), Some("macos14"));

        // One side alone leaves the other as it was.
        p.host_pools[0].os_profile.clear();
        let l = layout(&p, Sides::Both).unwrap();
        let own = own_config(&l).unwrap();
        assert!(own.client_os_profile.is_none());
        assert_eq!(own.server_os_profile.map(|o| o.name), Some("macos14"));
    }

    #[test]
    fn an_unknown_os_profile_is_a_plan_error_at_the_check() {
        let mut p = plan();
        p.host_pools[1].os_profile = "beos".into();
        let e = check_af_xdp(&p, Sides::Both).unwrap_err();
        assert!(matches!(e, BackendError::Plan(_)), "{e:?}");
        let text = e.to_string();
        assert!(text.contains("host pool servers"), "{text}");
        assert!(text.contains("os_profile 'beos'"), "{text}");
    }

    #[test]
    fn the_datapath_starts_on_queues_that_the_caller_opened() {
        use fps_backend_afxdp::memory::frame_pair;
        let p = plan();
        let (c, s) = frame_pair(64, 1600);
        let backend = start_on_queues(&p, Sides::Both, vec![(Box::new(c), Box::new(s))]).unwrap();
        assert!(backend.ports().is_empty(), "no socket was bound");
        assert_eq!(backend.datapath().lanes(), 1);
        let reports = backend.stop();
        assert_eq!(reports.len(), 1);
        assert!(!reports[0].panicked);

        let e = start_on_queues(&p, Sides::Both, Vec::new()).err().unwrap();
        assert!(e.to_string().contains("at least one lane"), "{e}");
    }

    #[test]
    fn the_bound_mode_follows_the_attach_mode_and_the_queues() {
        use BoundMode::{Copy, ZeroCopy};
        assert_eq!(mode_of(Attach::Generic, &[ZeroCopy]), XdpMode::Generic);
        assert_eq!(mode_of(Attach::Generic, &[Copy]), XdpMode::Generic);
        assert_eq!(mode_of(Attach::Native, &[Copy]), XdpMode::Native);
        assert_eq!(mode_of(Attach::Native, &[ZeroCopy]), XdpMode::ZeroCopy);
        assert_eq!(
            mode_of(Attach::Native, &[ZeroCopy, Copy]),
            XdpMode::Native,
            "the weakest queue decides"
        );
        assert_eq!(mode_of(Attach::Native, &[]), XdpMode::Native);
    }

    #[test]
    fn the_report_fills_the_ports_that_were_bound_and_no_other() {
        use crate::proto::PortInfo;
        let port = |name: &str| PortInfo {
            name: name.into(),
            xdp_driver_mode: XdpMode::Unspecified,
            ..PortInfo::default()
        };
        let mut caps = Capabilities {
            ports: vec![port("dpa"), port("dpb"), port("eth0")],
            ..Capabilities::default()
        };
        let started = StartedBackend::Kernel;
        started.report(&mut caps);
        assert!(caps
            .ports
            .iter()
            .all(|p| p.xdp_driver_mode == XdpMode::Unspecified));

        // The report of a hand built AF_XDP backend is covered by the root
        // test (tests/afxdp_veth.rs), which owns a real datapath. Here the
        // mapping of one bound port is enough.
        let b = BoundPort {
            name: "c".into(),
            interface: "dpa".into(),
            role: HostRole::Client,
            attach: Attach::Native,
            queues: vec![BoundMode::Copy],
        };
        assert_eq!(b.xdp_mode(), XdpMode::Native);
        assert_eq!(
            b.to_string(),
            "port c interface dpa attach native queues copy xdp_driver_mode XDP_MODE_NATIVE"
        );
    }
}
