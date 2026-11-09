// SPDX-License-Identifier: GPL-3.0-only
//! Worker threads for UDP flows.
//!
//! The workers of `fps-runtime` run TCP flows over kernel streams. The
//! standalone mode runs UDP flows on the same model with the datagram
//! openers of `fps-backend-kernel`. Each worker owns one thread with a
//! single threaded Tokio runtime, its schedule or its listeners, and its own
//! statistics shard, and it sends a cumulative [`WorkerSnapshot`] once per
//! snapshot interval and a final one when it ends. A client worker opens each
//! session with [`Connector::connect_datagram`] from a source address of its
//! host range. A server worker binds one [`KernelDatagramListener`] per
//! address, and every first datagram of a new peer opens a server session.
//! The session ids, generators, and metric series equal those of the TCP
//! workers.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use fps_backend_kernel::{Connector, KernelDatagram, KernelDatagramListener};
use fps_capture::{Side as TapSide, TappedDatagram};
use fps_flow::program::Program;
use fps_flow::template::Filler;
use fps_flow::TemplateFn;
use fps_interp::{
    session_rng, DatagramOpener, Failure, OpenKind, OpenRequest, Reason, Session, SessionCtx, Side,
    SideStats,
};
use fps_runtime::{
    global_session_id, ClientConfig, Collector, FlowRef, Role, ServerConfig, SessionKey,
    SourceHosts, TokioClock, WorkerConfig, WorkerSnapshot,
};
use fps_sched::{RateControl, Schedule};
use fps_stats::Side as StatsSide;
use fps_stats::{FailureReason, SessionCounters, Shard, StatsError};
use tokio::sync::{watch, Notify};
use tokio::task::LocalSet;
use tokio::time::{interval_at, sleep_until, Instant, MissedTickBehavior};

use crate::control::CaptureCell;

/// The longest a client worker sleeps, so that it sees a new target within
/// this time also while its schedule is paused or at its cap, as the workers
/// of `fps-runtime` do.
const MAX_IDLE: Duration = Duration::from_millis(100);

/// What the owner of a [`UdpRuntime`] hands to its workers besides their
/// configuration.
#[derive(Debug, Clone, Default)]
pub struct UdpHooks {
    /// The capture that sessions are tapped into when they begin.
    pub capture: CaptureCell,
    /// Whether server sessions are tapped. A client session already holds both
    /// directions, so an agent that runs clients taps those only, and a
    /// process with server workers only taps the server side.
    pub tap_servers: bool,
}

/// Errors of a UDP worker.
#[derive(Debug, thiserror::Error)]
pub enum UdpError {
    /// The worker configuration is invalid.
    #[error("udp worker {worker}: {detail}")]
    Config {
        /// The worker id.
        worker: u32,
        /// What is wrong.
        detail: String,
    },
    /// A server worker could not bind a listener.
    #[error("udp worker {worker}: listen")]
    Listen {
        /// The worker id.
        worker: u32,
        /// The backend error.
        #[source]
        source: fps_backend_kernel::BackendError,
    },
    /// The worker could not register its statistics series.
    #[error("udp worker {worker}: statistics")]
    Stats {
        /// The worker id.
        worker: u32,
        /// The statistics error.
        #[source]
        source: StatsError,
    },
    /// The worker thread or its Tokio runtime could not start.
    #[error("udp worker {worker}: start thread")]
    Thread {
        /// The worker id.
        worker: u32,
        /// The operating system error.
        #[source]
        source: io::Error,
    },
    /// A worker thread panicked or ended before it reported.
    #[error("udp worker {worker}: thread ended unexpectedly")]
    Lost {
        /// The worker id.
        worker: u32,
    },
}

/// The run state that every UDP worker follows, as in `fps-runtime`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Run,
    Drain(Duration),
    Abort,
}

impl Control {
    fn then(self, next: Control) -> Control {
        match (self, next) {
            (Control::Abort, _) | (_, Control::Abort) => Control::Abort,
            (Control::Drain(a), Control::Drain(b)) => Control::Drain(a.min(b)),
            (Control::Drain(a), Control::Run) => Control::Drain(a),
            (Control::Run, n) => n,
        }
    }
}

type Ready = (u32, Result<(), String>);

/// A running set of UDP workers.
#[derive(Debug)]
pub struct UdpRuntime {
    control: watch::Sender<Control>,
    threads: Vec<(u32, JoinHandle<Result<(), UdpError>>)>,
    snapshots: Receiver<WorkerSnapshot>,
    collector: Collector,
    rates: Arc<RateControl>,
}

impl UdpRuntime {
    /// Starts one thread per worker and waits until every worker is ready,
    /// which for a server worker means that its listeners are bound. An empty
    /// worker list gives a runtime that is finished at once.
    pub fn start(
        seed: u64,
        snapshot_interval: Duration,
        workers: Vec<WorkerConfig>,
    ) -> Result<Self, UdpError> {
        Self::start_with(seed, snapshot_interval, workers, UdpHooks::default())
    }

    /// Like [`UdpRuntime::start`], with the hooks that the agent session
    /// needs for captures.
    pub fn start_with(
        seed: u64,
        snapshot_interval: Duration,
        workers: Vec<WorkerConfig>,
        hooks: UdpHooks,
    ) -> Result<Self, UdpError> {
        let (control, control_rx) = watch::channel(Control::Run);
        let (snap_tx, snapshots) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel::<Ready>();
        let mut me = Self {
            control,
            threads: Vec::new(),
            snapshots,
            collector: Collector::new(),
            rates: Arc::new(RateControl::new()),
        };
        let mut failure = None;
        for w in workers {
            let id = w.id;
            let start = Start {
                config: w,
                seed,
                snapshot_interval,
                control: control_rx.clone(),
                snapshots: snap_tx.clone(),
                ready: ready_tx.clone(),
                rates: Arc::clone(&me.rates),
                hooks: hooks.clone(),
            };
            match std::thread::Builder::new()
                .name(format!("fps-udp-{id}"))
                .spawn(move || run_thread(start))
            {
                Ok(t) => me.threads.push((id, t)),
                Err(source) => {
                    failure = Some(UdpError::Thread { worker: id, source });
                    break;
                }
            }
        }
        drop(ready_tx);
        for _ in 0..me.threads.len() {
            match ready_rx.recv() {
                Ok((_, Ok(()))) => {}
                Ok((id, Err(detail))) => {
                    failure.get_or_insert(UdpError::Config { worker: id, detail });
                }
                Err(_) => {
                    failure.get_or_insert(UdpError::Lost { worker: u32::MAX });
                    break;
                }
            }
        }
        match failure {
            None => Ok(me),
            Some(e) => {
                me.abort();
                let _ = me.join();
                Err(e)
            }
        }
    }

    /// Returns the registry of the rate handles of the client workers by mix
    /// entry name, as `RuntimeHandle::rate_control` does for TCP. Every client
    /// worker registered its handle before [`UdpRuntime::start`] returned.
    pub fn rate_control(&self) -> Arc<RateControl> {
        Arc::clone(&self.rates)
    }

    /// Stops new sessions and resets the open ones after `ramp_down`.
    pub fn stop(&self, ramp_down: Duration) {
        self.control
            .send_modify(|c| *c = c.then(Control::Drain(ramp_down)));
    }

    /// Resets every open session at once and ends every worker.
    pub fn abort(&self) {
        self.control.send_modify(|c| *c = c.then(Control::Abort));
    }

    /// Takes the snapshots that arrived since the last call.
    pub fn collect(&mut self) -> &Collector {
        while let Ok(s) = self.snapshots.try_recv() {
            self.collector.absorb(s);
        }
        &self.collector
    }

    /// Returns true when every worker thread has ended.
    pub fn is_finished(&self) -> bool {
        self.threads.iter().all(|(_, t)| t.is_finished())
    }

    /// Waits for every worker and returns the collector and the worker
    /// results.
    pub fn join(mut self) -> (Collector, BTreeMap<u32, Result<(), UdpError>>) {
        let mut out = BTreeMap::new();
        for (id, t) in std::mem::take(&mut self.threads) {
            out.insert(id, t.join().unwrap_or(Err(UdpError::Lost { worker: id })));
        }
        self.collect();
        (std::mem::take(&mut self.collector), out)
    }
}

impl Drop for UdpRuntime {
    fn drop(&mut self) {
        if !self.threads.is_empty() {
            self.abort();
        }
    }
}

struct Start {
    config: WorkerConfig,
    seed: u64,
    snapshot_interval: Duration,
    control: watch::Receiver<Control>,
    snapshots: Sender<WorkerSnapshot>,
    ready: Sender<Ready>,
    rates: Arc<RateControl>,
    hooks: UdpHooks,
}

fn run_thread(start: Start) -> Result<(), UdpError> {
    let worker = start.config.id;
    if let Some(core) = start.config.core {
        // A failed pin leaves the thread unpinned, as in fps-runtime.
        let _ = core_affinity::set_for_current(core_affinity::CoreId { id: core });
    }
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(source) => {
            let _ = start.ready.send((worker, Err(source.to_string())));
            return Err(UdpError::Thread { worker, source });
        }
    };
    LocalSet::new().block_on(&rt, run_worker(start))
}

/// The series that a worker records for every session.
///
/// They are the series of [`SessionCounters`], which `fps-runtime` registers
/// for its TCP workers too, so every series carries the `side` label of the
/// worker and the two sides of one UDP session never add up.
struct Metrics {
    counters: SessionCounters,
}

/// Maps the side of the interpreter to the `side` label value.
fn stats_side(side: Side) -> StatsSide {
    match side {
        Side::Client => StatsSide::Client,
        Side::Server => StatsSide::Server,
    }
}

/// Maps a failure reason of the interpreter to its `reason` label value.
fn stats_reason(reason: Reason) -> FailureReason {
    match reason {
        Reason::ConnectTimeout => FailureReason::ConnectTimeout,
        Reason::ConnectRefused => FailureReason::ConnectRefused,
        Reason::ExpectTimeout => FailureReason::ExpectTimeout,
        Reason::PatternMismatch => FailureReason::PatternMismatch,
        Reason::UnexpectedReset => FailureReason::UnexpectedReset,
        Reason::PeerClosed => FailureReason::PeerClosed,
        Reason::TlsFailure => FailureReason::TlsFailure,
        Reason::Aborted => FailureReason::Aborted,
        Reason::Internal => FailureReason::Internal,
        Reason::DatagramRejected => FailureReason::DatagramRejected,
        Reason::TemplateError => FailureReason::TemplateError,
        Reason::ExtensionError => FailureReason::ExtensionError,
    }
}

impl Metrics {
    fn register(
        shard: &mut Shard,
        application: &str,
        scenario: &str,
        side: Side,
    ) -> Result<Self, StatsError> {
        Ok(Self {
            counters: SessionCounters::register(shard, application, scenario, stats_side(side))?,
        })
    }

    fn started(&self, shard: &mut Shard) {
        self.counters.started(shard);
    }

    fn ended(&self, shard: &mut Shard, stats: SideStats, outcome: &Result<(), Failure>, us: u64) {
        if stats.established {
            self.counters.established(shard);
        }
        match outcome {
            Ok(()) => self.counters.succeeded(shard),
            Err(f) => self.counters.failed(shard, stats_reason(f.reason)),
        }
        self.counters
            .add_bytes(shard, stats.bytes_tx, stats.bytes_rx);
        self.counters.ended(shard, us);
    }
}

struct Worker {
    id: u32,
    seed: u64,
    side: Side,
    flow: FlowRef,
    filler: Filler,
    shard: RefCell<Shard>,
    metrics: Metrics,
    open: Cell<u32>,
    closed: Notify,
    sequence: Cell<u64>,
    snapshots: Sender<WorkerSnapshot>,
    abort: watch::Sender<bool>,
    capture: CaptureCell,
    tap_servers: bool,
}

impl Worker {
    fn new(start: &Start, side: Side, flow: &FlowRef) -> Result<Self, UdpError> {
        let id = start.config.id;
        let f = flow
            .scenario
            .flows
            .get(flow.flow)
            .ok_or_else(|| UdpError::Config {
                worker: id,
                detail: format!("scenario {} has no flow {}", flow.scenario.name, flow.flow),
            })?;
        let mut shard = Shard::new(id);
        let metrics = Metrics::register(&mut shard, &f.module, &flow.scenario.name, side)
            .map_err(|source| UdpError::Stats { worker: id, source })?;
        Ok(Self {
            id,
            seed: start.seed,
            side,
            flow: flow.clone(),
            filler: Filler::new(start.seed),
            shard: RefCell::new(shard),
            metrics,
            open: Cell::new(0),
            closed: Notify::new(),
            sequence: Cell::new(0),
            snapshots: start.snapshots.clone(),
            abort: watch::channel(false).0,
            capture: start.hooks.capture.clone(),
            tap_servers: start.hooks.tap_servers,
        })
    }

    fn program(&self) -> Option<&Program> {
        let f = self.flow.scenario.flows.get(self.flow.flow)?;
        match self.side {
            Side::Client => Some(&f.client),
            Side::Server => f.server.as_ref(),
        }
    }

    fn send_snapshot(&self, is_final: bool) {
        let seq = self.sequence.get() + 1;
        self.sequence.set(seq);
        let snapshot = self.shard.borrow().snapshot();
        let _ = self.snapshots.send(WorkerSnapshot {
            worker: self.id,
            side: self.side,
            sequence: seq,
            is_final,
            snapshot,
        });
    }

    fn aborted(&self) -> impl Future<Output = ()> + 'static {
        let mut rx = self.abort.subscribe();
        async move {
            let _ = rx.wait_for(|a| *a).await;
        }
    }

    async fn session<O: DatagramOpener>(self: Rc<Self>, local_id: u64, mut opener: O) {
        let started = Instant::now();
        self.metrics.started(&mut self.shard.borrow_mut());
        let stop = self.aborted();
        let Some(program) = self.program() else {
            return;
        };
        let flow_id = self
            .flow
            .scenario
            .flows
            .get(self.flow.flow)
            .map_or("", |f| f.id.as_str());
        let native = (self.flow.functions)();
        let id = global_session_id(self.id, local_id);
        // A flow without extensions takes the native functions as they are.
        // Otherwise the wrapper routes the modules of the set to the
        // instances that this session owns, as the TCP worker does.
        let mut wrapped = None;
        let mut plain = None;
        let functions: &mut dyn TemplateFn = match &self.flow.extensions {
            Some(set) => wrapped.insert(set.session_functions(
                SessionKey {
                    seed: self.seed,
                    session_id: id,
                    side: self.side,
                    scenario: Arc::clone(&self.flow.scenario),
                    flow: self.flow.flow,
                    transport: fps_runtime::Transport::Udp,
                },
                native,
            )),
            None => plain.insert(native).as_mut(),
        };
        let mut session = Session::new(SessionCtx {
            vars: &self.flow.scenario.vars,
            filler: &self.filler,
            flow: flow_id,
            side: self.side,
            session_id: id,
            rng: session_rng(self.seed, id, self.side),
            functions,
            limits: self.flow.limits,
        });
        let outcome = session
            .run_datagram_program(&mut opener, program, stop)
            .await;
        let stats = session.stats();
        drop(session);
        if let Some(w) = wrapped {
            // The outcome is recorded below, so a failing on-close hook
            // cannot change it.
            let _ = w.finish(fps_runtime::close_reason(&outcome));
        }
        let us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.metrics
            .ended(&mut self.shard.borrow_mut(), stats, &outcome, us);
        self.open.set(self.open.get().saturating_sub(1));
        self.closed.notify_one();
    }
}

async fn run_worker(start: Start) -> Result<(), UdpError> {
    let id = start.config.id;
    let role = start.config.role.clone();
    let prepared = match &role {
        Role::Client(c) => Worker::new(&start, Side::Client, &c.flow).and_then(|w| {
            let sched = schedule(id, c, &start.rates)?;
            Ok((w, Vec::new(), Some(sched)))
        }),
        Role::Server(s) => Worker::new(&start, Side::Server, &s.flow).and_then(|w| {
            let listeners = bind(id, s)?;
            Ok((w, listeners, None))
        }),
    };
    let (worker, listeners, sched) = match prepared {
        Ok(p) => p,
        Err(e) => {
            let _ = start.ready.send((id, Err(error_chain(&e))));
            return Err(e);
        }
    };
    let _ = start.ready.send((id, Ok(())));
    let worker = Rc::new(worker);
    let mut control = start.control;
    let mut tick = interval_at(
        Instant::now() + start.snapshot_interval,
        start.snapshot_interval,
    );
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let stop = match (role, sched) {
        (Role::Client(c), Some(sched)) => {
            client_loop(&worker, *c, sched, &mut control, &mut tick).await
        }
        _ => server_loop(&worker, listeners, &mut control, &mut tick).await,
    };
    let mut deadline = match stop {
        Control::Run => None,
        Control::Drain(d) => Some(Instant::now() + d),
        Control::Abort => Some(Instant::now()),
    };
    while worker.open.get() > 0 {
        let aborted = *worker.abort.borrow();
        tokio::select! {
            biased;
            () = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() && !aborted => {
                let _ = worker.abort.send_replace(true);
            }
            r = control.changed(), if !aborted => {
                if r.is_err() || *control.borrow_and_update() == Control::Abort {
                    deadline = Some(Instant::now());
                }
            }
            () = worker.closed.notified() => {}
            _ = tick.tick() => worker.send_snapshot(false),
        }
    }
    worker.send_snapshot(true);
    Ok(())
}

/// Builds the schedule of a client worker and registers its rate handle under
/// the name of its mix entry, before the worker reports ready.
fn schedule(worker: u32, c: &ClientConfig, rates: &RateControl) -> Result<Schedule, UdpError> {
    let sched = Schedule::new(c.schedule.clone()).map_err(|e| UdpError::Config {
        worker,
        detail: format!("schedule: {e}"),
    })?;
    let entry = c.flow.entry.as_deref().unwrap_or(&c.flow.scenario.name);
    rates
        .register(entry, sched.rate_handle())
        .map_err(|e| UdpError::Config {
            worker,
            detail: format!("rate handle of {entry}: {e}"),
        })?;
    Ok(sched)
}

fn bind(worker: u32, s: &ServerConfig) -> Result<Vec<KernelDatagramListener>, UdpError> {
    s.listen
        .iter()
        .map(|a| {
            KernelDatagramListener::bind(*a, &s.config.socket)
                .map_err(|source| UdpError::Listen { worker, source })
        })
        .collect()
}

struct ClientCtx {
    connector: Connector,
    config: ClientConfig,
}

async fn client_loop(
    worker: &Rc<Worker>,
    c: ClientConfig,
    mut sched: Schedule,
    control: &mut watch::Receiver<Control>,
    tick: &mut tokio::time::Interval,
) -> Control {
    let ctx = Rc::new(ClientCtx {
        connector: Connector::new(c.connect.clone()),
        config: c,
    });
    let clock = TokioClock::start_now();
    loop {
        let state = *control.borrow_and_update();
        if state != Control::Run {
            return state;
        }
        let mut open = worker.open.get();
        for start in sched.drain(&clock, open) {
            open += 1;
            worker.open.set(open);
            let (w, ctx) = (Rc::clone(worker), Rc::clone(&ctx));
            tokio::task::spawn_local(async move {
                let ctx = &*ctx;
                let opener = ClientOpener::new(
                    &ctx.connector,
                    &ctx.config.sources,
                    &ctx.config.servers,
                    ctx.config.port,
                    start.session_id,
                    Tap::new(&w.capture, w.id, start.session_id, TapSide::Client),
                );
                w.session(start.session_id, opener).await;
            });
        }
        if sched.is_finished(&clock) && worker.open.get() == 0 {
            return Control::Run;
        }
        // Never sleep past MAX_IDLE, so a new target shows within that time.
        let wake = Instant::now() + MAX_IDLE;
        let due = sched
            .next_due(&clock, worker.open.get())
            .map_or(wake, |d| clock.instant_at(d).min(wake));
        tokio::select! {
            biased;
            r = control.changed() => {
                if r.is_err() {
                    return Control::Abort;
                }
            }
            () = sleep_until(due) => {}
            () = worker.closed.notified() => {}
            _ = tick.tick() => worker.send_snapshot(false),
        }
    }
}

async fn server_loop(
    worker: &Rc<Worker>,
    listeners: Vec<KernelDatagramListener>,
    control: &mut watch::Receiver<Control>,
    tick: &mut tokio::time::Interval,
) -> Control {
    let next_id = Rc::new(Cell::new(0u64));
    let mut accepts = Vec::with_capacity(listeners.len());
    for mut listener in listeners {
        let (w, next_id) = (Rc::clone(worker), Rc::clone(&next_id));
        accepts.push(tokio::task::spawn_local(async move {
            loop {
                let Ok((io, _peer)) = listener.accept().await else {
                    tokio::task::yield_now().await;
                    continue;
                };
                let local_id = next_id.get();
                next_id.set(local_id + 1);
                w.open.set(w.open.get() + 1);
                let w = Rc::clone(&w);
                tokio::task::spawn_local(async move {
                    let tap = w
                        .tap_servers
                        .then(|| Tap::new(&w.capture, w.id, local_id, TapSide::Server));
                    w.session(local_id, Accepted { io: Some(io), tap }).await;
                });
            }
        }));
    }
    let state = loop {
        let state = *control.borrow_and_update();
        if state != Control::Run {
            break state;
        }
        tokio::select! {
            biased;
            r = control.changed() => {
                if r.is_err() {
                    break Control::Abort;
                }
            }
            _ = tick.tick() => worker.send_snapshot(false),
        }
    };
    for a in accepts {
        a.abort();
    }
    state
}

/// Opens the client side of one UDP session from a source address of the
/// worker's host range.
struct ClientOpener<'a> {
    connector: &'a Connector,
    servers: &'a [IpAddr],
    port: Option<u16>,
    source: Result<IpAddr, String>,
    session: u64,
    tap: Tap,
}

/// What a session needs to join the current capture when it opens.
struct Tap {
    cell: CaptureCell,
    session_id: u64,
    side: TapSide,
}

impl Tap {
    fn new(cell: &CaptureCell, worker: u32, local_id: u64, side: TapSide) -> Self {
        Self {
            cell: cell.clone(),
            session_id: global_session_id(worker, local_id),
            side,
        }
    }

    /// Wraps the socket of the session. The capture is read here, when the
    /// session opens, so a capture that started a moment ago takes this one.
    fn wrap(&self, io: KernelDatagram) -> TappedDatagram<KernelDatagram> {
        let (local, peer) = (io.local_addr(), io.peer_addr());
        self.cell
            .tap_datagram(self.session_id, self.side, local, peer, io)
    }
}

impl<'a> ClientOpener<'a> {
    fn new(
        connector: &'a Connector,
        sources: &SourceHosts,
        servers: &'a [IpAddr],
        port: Option<u16>,
        session: u64,
        tap: Tap,
    ) -> Self {
        Self {
            connector,
            servers,
            port,
            source: source_addr(sources, session),
            session,
            tap,
        }
    }

    fn destination(&self, request: &OpenRequest<'_>) -> io::Result<SocketAddr> {
        let port = self.port.unwrap_or(request.port);
        let ip = match request.host {
            Some(host) => std::str::from_utf8(host)
                .ok()
                .and_then(|h| h.parse::<IpAddr>().ok())
                .ok_or_else(|| {
                    invalid(format!(
                        "connect host {} is not an IP address",
                        String::from_utf8_lossy(host)
                    ))
                })?,
            None => {
                let n = self.servers.len() as u64;
                if n == 0 {
                    return Err(invalid(
                        "no server address for a connect without host".into(),
                    ));
                }
                let i = usize::try_from(self.session % n).unwrap_or(0);
                *self
                    .servers
                    .get(i)
                    .ok_or_else(|| invalid("invariant: server index in range".into()))?
            }
        };
        Ok(SocketAddr::new(ip, port))
    }
}

/// Returns the source address that the selector of `sources` picks for
/// `session`.
pub(crate) fn source_addr(sources: &SourceHosts, session: u64) -> Result<IpAddr, String> {
    let index = sources.selector.select(session);
    let addr = if sources.ipv6 {
        sources.pool.ipv6_addr(index).map(|a| a.map(IpAddr::V6))
    } else {
        sources.pool.ipv4_addr(index).map(|a| a.map(IpAddr::V4))
    };
    match addr {
        Ok(Some(a)) => Ok(a),
        Ok(None) => Err(format!(
            "pool {} has no {} prefix",
            sources.pool.name(),
            if sources.ipv6 { "IPv6" } else { "IPv4" }
        )),
        Err(e) => Err(e.to_string()),
    }
}

impl DatagramOpener for ClientOpener<'_> {
    type Io = TappedDatagram<KernelDatagram>;

    fn open(
        &mut self,
        request: OpenRequest<'_>,
    ) -> impl Future<Output = io::Result<TappedDatagram<KernelDatagram>>> {
        let opened = (|| {
            if request.kind != OpenKind::Connect {
                return Err(invalid("a client opener cannot accept".into()));
            }
            let source = self.source.clone().map_err(invalid)?;
            let destination = self.destination(&request)?;
            self.connector
                .connect_datagram(source, destination)
                .map(|io| self.tap.wrap(io))
                .map_err(|e| io::Error::new(e.io_kind().unwrap_or(io::ErrorKind::Other), e))
        })();
        std::future::ready(opened)
    }
}

/// Hands the interpreter one session that a listener already opened.
struct Accepted {
    io: Option<KernelDatagram>,
    tap: Option<Tap>,
}

impl DatagramOpener for Accepted {
    type Io = TappedDatagram<KernelDatagram>;

    fn open(
        &mut self,
        request: OpenRequest<'_>,
    ) -> impl Future<Output = io::Result<TappedDatagram<KernelDatagram>>> {
        let tap = self.tap.as_ref();
        let io = match request.kind {
            OpenKind::Accept => self
                .io
                .take()
                .map(|io| match tap {
                    Some(t) => t.wrap(io),
                    None => TappedDatagram::new(io, None),
                })
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "session used")),
            OpenKind::Connect => Err(invalid("a server opener cannot connect".into())),
        };
        std::future::ready(io)
    }
}

fn invalid(detail: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, detail)
}

fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        s.push_str(": ");
        s.push_str(&c.to_string());
        cur = c.source();
    }
    s
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fps_backend_kernel::{ConnectConfig, ListenConfig};
    use fps_flow::Scenario;
    use fps_hosts::{HostPool, HostSelector, PoolSpec, Selection};
    use fps_sched::{Arrival, LoadProfile, Objective, Phase, ScheduleConfig, Shape};
    use fps_stats::{metric, SeriesValue};

    use super::*;

    const LOOKUP: &str = "scenario: lookup\nflows:\n  - id: query\n    module: dns\n    l4: udp\n    client:\n      - connect: { port: 53 }\n      - send: \"{{ hex('1234') }}\"\n      - expect: { min_bytes: 2, timeout_ms: 2000 }\n      - close: fin\n    server:\n      - accept: { port: 53 }\n      - expect: { min_bytes: 2, timeout_ms: 2000 }\n      - send: \"{{ hex('12348180') }}\"\n      - close: fin\n";

    #[test]
    fn udp_client_and_server_workers_complete_sessions_on_loopback() {
        let mut s = Scenario::parse(LOOKUP).unwrap();
        s.fold_constants().unwrap();
        let s = Arc::new(s);
        // An unprivileged test cannot bind port 53, so the server takes a
        // kernel chosen port that the client then overrides.
        let probe = std::net::UdpSocket::bind("127.61.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let server = WorkerConfig {
            id: 1,
            core: None,
            role: Role::Server(ServerConfig {
                flow: FlowRef::new(Arc::clone(&s), 0),
                listen: vec![SocketAddr::new("127.61.0.1".parse().unwrap(), port)],
                config: ListenConfig::default(),
            }),
        };
        let servers = UdpRuntime::start(5, Duration::from_millis(200), vec![server]).unwrap();
        let pool = HostPool::parse(&PoolSpec {
            name: "c",
            ipv4: Some("127.62.0.0/24"),
            count: 4,
            ..PoolSpec::default()
        })
        .unwrap();
        let profile = LoadProfile::new(vec![Phase::new(
            "steady",
            Duration::from_secs(1),
            Shape::Hold,
            1.0,
            1.0,
        )])
        .unwrap();
        let mut schedule = ScheduleConfig::new(Objective::SessionsPerSecond(20.0), profile, 5);
        schedule.arrival = Arrival::Constant;
        let client = WorkerConfig {
            id: 0,
            core: None,
            role: Role::Client(Box::new(ClientConfig {
                flow: FlowRef::new(Arc::clone(&s), 0),
                schedule,
                servers: vec!["127.61.0.1".parse().unwrap()],
                port: Some(port),
                sources: SourceHosts {
                    selector: HostSelector::new("c", pool.range(), Selection::RoundRobin, 5, 0)
                        .unwrap(),
                    pool,
                    ipv6: false,
                },
                connect: ConnectConfig::default(),
            })),
        };
        let clients = UdpRuntime::start(5, Duration::from_millis(200), vec![client]).unwrap();
        let (c, results) = clients.join();
        assert!(results.values().all(Result::is_ok), "{results:?}");
        servers.stop(Duration::from_secs(1));
        let (srv, results) = servers.join();
        assert!(results.values().all(Result::is_ok), "{results:?}");
        let c = c.total(Side::Client).unwrap();
        let srv = srv.total(Side::Server).unwrap();
        assert_eq!(c.counter_total(metric::SESSIONS_ATTEMPTED), 20);
        assert_eq!(c.counter_total(metric::SESSIONS_SUCCEEDED), 20);
        assert_eq!(srv.counter_total(metric::SESSIONS_SUCCEEDED), 20);
        assert_eq!(c.counter_total(metric::BYTES_TX), 40);
        assert_eq!(c.counter_total(metric::BYTES_RX), 80);
        // Every series carries the side label of its worker, and the two
        // sides sit in different series.
        for (snapshot, side) in [(&c, "client"), (&srv, "server")] {
            assert!(!snapshot.is_empty());
            for (key, _) in snapshot.iter() {
                assert!(
                    key.labels().contains(&("side", side.to_owned())),
                    "{:?} lacks side {side}",
                    key.labels()
                );
            }
        }
        let mut both = fps_stats::Snapshot::aggregate();
        both.merge(&c).unwrap();
        both.merge(&srv).unwrap();
        assert_eq!(both.counter_total(metric::SESSIONS_SUCCEEDED), 40);
        let on = |side: &str| {
            both.iter()
                .filter(|(k, v)| {
                    k.metric() == metric::SESSIONS_SUCCEEDED
                        && k.labels().contains(&("side", side.to_owned()))
                        && matches!(v, SeriesValue::Count(20))
                })
                .count()
        };
        assert_eq!((on("client"), on("server")), (1, 1));
    }
}
