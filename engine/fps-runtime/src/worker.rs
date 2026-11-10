// SPDX-License-Identifier: GPL-3.0-only
//! One worker thread with its own single threaded Tokio runtime.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use fps_backend_kernel::{Connector, KernelListener, ListenConfig};
use fps_capture::CaptureCell;
use fps_flow::template::Filler;
use fps_interp::{session_rng, Session, SessionCtx, Side};
use fps_sched::{RateHandle, Schedule};
use fps_stats::Shard;
use tokio::sync::{watch, Notify};
use tokio::task::LocalSet;
use tokio::time::{interval_at, sleep_until, Instant, MissedTickBehavior};

use crate::clock::TokioClock;
use crate::collector::WorkerSnapshot;
use crate::config::{ClientConfig, FlowRef, Role, ServerConfig, WorkerConfig};
use crate::control::Control;
use crate::extension::close_reason;
use crate::instance::{self, Peer};
use crate::metrics::SessionMetrics;
use crate::opener::{global_session_id, Accepted, ClientOpener, Endpoints, TapOpener};
use crate::runtime::WorkerExit;
use crate::{RuntimeError, SessionKey, TlsProfiles, Transport};

/// What a worker thread receives from the runtime.
pub(crate) struct WorkerStart {
    pub(crate) config: WorkerConfig,
    pub(crate) seed: u64,
    pub(crate) snapshot_interval: Duration,
    pub(crate) control: watch::Receiver<Control>,
    pub(crate) snapshots: Sender<WorkerSnapshot>,
    pub(crate) ready: Sender<(u32, Result<Ready, RuntimeError>)>,
    pub(crate) capture: CaptureCell,
    pub(crate) tls: Option<Arc<TlsProfiles>>,
}

/// What a worker reports when it is ready.
pub(crate) struct Ready {
    /// The bound listen addresses of a server worker.
    pub(crate) addrs: Vec<SocketAddr>,
    /// The mix entry name and the rate handle of a client worker.
    pub(crate) rate: Option<(String, RateHandle)>,
}

/// The longest time that a client worker sleeps before it reads its rate
/// handle again. A worker with nothing due, because its rate is 0 or its
/// sessions sit at the cap, would otherwise never see a new target, and a
/// worker that waits for a distant start would keep the old spacing.
pub(crate) const RATE_POLL: Duration = Duration::from_millis(100);

/// Returns when a client worker wakes next: at the due time of its schedule,
/// but at most [`RATE_POLL`] after `now`.
pub(crate) fn next_wake(due: Option<Instant>, now: Instant) -> Instant {
    let poll = now + RATE_POLL;
    due.map_or(poll, |d| d.min(poll))
}

/// The body of one worker thread.
pub(crate) fn run_thread(start: WorkerStart) -> Result<WorkerExit, RuntimeError> {
    let worker = start.config.id;
    let pinned = start
        .config
        .core
        .map(|core| core_affinity::set_for_current(core_affinity::CoreId { id: core }));
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(source) => {
            let err = RuntimeError::Thread { worker, source };
            let _ = start.ready.send((worker, Err(clone_err(&err))));
            return Err(err);
        }
    };
    let local = LocalSet::new();
    let mut exit = local.block_on(&rt, run_worker(start))?;
    exit.pinned = pinned;
    Ok(exit)
}

/// The shared state of one worker, which only its own thread touches.
pub(crate) struct Worker {
    pub(crate) id: u32,
    pub(crate) seed: u64,
    side: Side,
    pub(crate) flow: FlowRef,
    pub(crate) filler: Filler,
    pub(crate) shard: RefCell<Shard>,
    metrics: SessionMetrics,
    /// The series of every flow of the scenario, by flow index, for a worker
    /// that runs scenario instances, and empty for a single flow scenario.
    /// Flows of one module share their series.
    flow_metrics: Vec<SessionMetrics>,
    /// The configuration of the listeners that a server instance binds data
    /// ports with.
    pub(crate) listen_config: ListenConfig,
    open: Cell<u32>,
    closed: Notify,
    sequence: Cell<u64>,
    snapshots: Sender<WorkerSnapshot>,
    abort: watch::Sender<bool>,
    pub(crate) capture: CaptureCell,
    tls: Option<Arc<TlsProfiles>>,
}

impl Worker {
    fn new(
        start: &WorkerStart,
        side: Side,
        flow: &FlowRef,
        listen_config: ListenConfig,
    ) -> Result<Self, RuntimeError> {
        let id = start.config.id;
        let f = flow.get()?;
        let mut shard = Shard::new(id);
        let stats = |source| RuntimeError::Stats { worker: id, source };
        let metrics = SessionMetrics::register(&mut shard, &f.module, &flow.scenario.name, side)
            .map_err(stats)?;
        let mut flow_metrics = Vec::new();
        if flow.scenario.flows.len() > 1 {
            let mut by_module: HashMap<&str, SessionMetrics> = HashMap::new();
            for f in &flow.scenario.flows {
                let m = match by_module.get(f.module.as_str()) {
                    Some(m) => m.clone(),
                    None => {
                        let m =
                            SessionMetrics::register(&mut shard, &f.module, &flow.scenario.name, side)
                                .map_err(stats)?;
                        by_module.insert(f.module.as_str(), m.clone());
                        m
                    }
                };
                flow_metrics.push(m);
            }
        }
        Ok(Self {
            id,
            seed: start.seed,
            side,
            flow: flow.clone(),
            filler: Filler::new(start.seed),
            shard: RefCell::new(shard),
            metrics,
            flow_metrics,
            listen_config,
            open: Cell::new(0),
            closed: Notify::new(),
            sequence: Cell::new(0),
            snapshots: start.snapshots.clone(),
            abort: watch::channel(false).0,
            capture: start.capture.clone(),
            tls: start.tls.clone(),
        })
    }

    /// Returns true when the scenario has several flows, so a session start
    /// runs a scenario instance.
    fn is_instance(&self) -> bool {
        !self.flow_metrics.is_empty()
    }

    /// Returns the series of flow `index`.
    pub(crate) fn metrics_of(&self, index: usize) -> &SessionMetrics {
        self.flow_metrics.get(index).unwrap_or(&self.metrics)
    }

    /// Sends the cumulative snapshot of the shard to the collector.
    fn send_snapshot(&self, is_final: bool) {
        let seq = self.sequence.get() + 1;
        self.sequence.set(seq);
        let snapshot = self.shard.borrow().snapshot();
        // A dropped collector only means that nobody reads the snapshots.
        let _ = self.snapshots.send(WorkerSnapshot {
            worker: self.id,
            side: self.side,
            sequence: seq,
            is_final,
            snapshot,
        });
    }

    /// Returns a future that completes when the worker aborts its sessions.
    pub(crate) fn aborted(&self) -> impl std::future::Future<Output = ()> + 'static {
        let mut rx = self.abort.subscribe();
        async move {
            // An error means that the worker dropped the sender, which ends
            // the session as well.
            let _ = rx.wait_for(|a| *a).await;
        }
    }

    /// Runs one session side and records it in the shard.
    async fn session<O: fps_interp::Opener>(
        self: Rc<Self>,
        local_id: u64,
        program: &fps_flow::program::Program,
        opener: O,
    ) where
        O::Io: Endpoints,
    {
        let started = Instant::now();
        self.metrics.started(&mut self.shard.borrow_mut());
        let stop = self.aborted();
        let (outcome, stats) = match self.flow.get() {
            Ok(flow) => {
                let native = (self.flow.functions)();
                let id = global_session_id(self.id, local_id);
                let mut opener = TapOpener::new(opener, &self.capture, id, self.side);
                // A flow without extensions takes the native functions as they
                // are. Otherwise the wrapper routes the modules of the set to
                // the instances that this session owns.
                let mut wrapped = None;
                let mut plain = None;
                let functions: &mut dyn fps_flow::TemplateFn = match &self.flow.extensions {
                    Some(set) => wrapped.insert(set.session_functions(
                        SessionKey {
                            seed: self.seed,
                            session_id: id,
                            side: self.side,
                            scenario: Arc::clone(&self.flow.scenario),
                            flow: self.flow.flow,
                            // The worker runs stream sessions only.
                            transport: Transport::Tcp,
                        },
                        native,
                    )),
                    None => plain.insert(native).as_mut(),
                };
                let mut session = Session::new(SessionCtx {
                    vars: &self.flow.scenario.vars,
                    filler: &self.filler,
                    flow: &flow.id,
                    side: self.side,
                    session_id: id,
                    rng: session_rng(self.seed, id, self.side),
                    functions,
                    limits: self.flow.limits,
                });
                if let Some(profiles) = &self.tls {
                    session.set_tls_upgrader(&**profiles);
                }
                let outcome = session.run_program(&mut opener, program, stop).await;
                let stats = session.stats();
                drop(session);
                if let Some(w) = wrapped {
                    // The outcome is recorded below, so a failing on-close
                    // hook cannot change it.
                    let _ = w.finish(close_reason(&outcome));
                }
                (outcome, stats)
            }
            // The configuration check rules this out before any thread starts.
            Err(_) => return,
        };
        let us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.metrics
            .ended(&mut self.shard.borrow_mut(), stats, &outcome, us);
        self.open.set(self.open.get().saturating_sub(1));
        self.closed.notify_one();
    }
}

impl Worker {
    /// Runs the scenario instance of one session start and ends the session
    /// count of the worker, see [`instance`].
    async fn run_instance(self: Rc<Self>, local_id: u64, peer: Peer) {
        instance::run(Rc::clone(&self), local_id, peer).await;
        self.open.set(self.open.get().saturating_sub(1));
        self.closed.notify_one();
    }
}

async fn run_worker(start: WorkerStart) -> Result<WorkerExit, RuntimeError> {
    let id = start.config.id;
    let role = start.config.role.clone();
    let prepared = match &role {
        Role::Client(c) => Worker::new(&start, Side::Client, &c.flow, ListenConfig::default()).and_then(|w| {
            let sched = Schedule::new(c.schedule.clone())
                .map_err(|source| RuntimeError::Schedule { worker: id, source })?;
            Ok((w, Vec::new(), Some(sched)))
        }),
        Role::Server(s) => Worker::new(&start, Side::Server, &s.flow, s.config.clone()).and_then(|w| {
            let listeners = bind(id, s)?;
            Ok((w, listeners, None))
        }),
    };
    let (worker, listeners, sched) = match prepared {
        Ok(p) => p,
        Err(e) => {
            let _ = start.ready.send((id, Err(clone_err(&e))));
            return Err(e);
        }
    };
    let addrs = listeners.iter().map(KernelListener::local_addr).collect();
    let rate = match (&role, &sched) {
        (Role::Client(c), Some(sched)) => Some((
            c.flow
                .entry
                .clone()
                .unwrap_or_else(|| c.flow.scenario.name.clone()),
            sched.rate_handle(),
        )),
        _ => None,
    };
    let _ = start.ready.send((id, Ok(Ready { addrs, rate })));
    let worker = Rc::new(worker);
    let mut control = start.control;
    let mut tick = interval_at(
        Instant::now() + start.snapshot_interval,
        start.snapshot_interval,
    );
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let stop = match role {
        Role::Client(c) => match sched {
            Some(sched) => client_loop(&worker, *c, sched, &mut control, &mut tick).await,
            None => Control::Run,
        },
        Role::Server(_) => server_loop(&worker, listeners, &mut control, &mut tick).await,
    };
    // Open sessions finish within the ramp down time and are reset after it.
    // An abort, or a lost handle, resets them at once.
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
    let shard = worker.shard.borrow();
    Ok(WorkerExit {
        worker: id,
        side: worker.side,
        pinned: None,
        sessions: shard.snapshot(),
    })
}

fn bind(worker: u32, s: &ServerConfig) -> Result<Vec<KernelListener>, RuntimeError> {
    s.listen
        .iter()
        .map(|a| {
            KernelListener::bind(*a, &s.config)
                .map_err(|source| RuntimeError::Listen { worker, source })
        })
        .collect()
}

/// Starts sessions on the schedule until it ends or a stop arrives, and
/// returns the stop, or [`Control::Run`] when the schedule ended.
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
                if w.is_instance() {
                    let local = start.session_id;
                    w.run_instance(local, Peer::Client(ctx)).await;
                    return;
                }
                let ctx = &*ctx;
                let opener = ClientOpener::new(
                    &ctx.connector,
                    &ctx.config.sources,
                    &ctx.config.servers,
                    ctx.config.port,
                    start.session_id,
                );
                let flow = match ctx.config.flow.get() {
                    Ok(f) => f,
                    Err(_) => return,
                };
                w.session(start.session_id, &flow.client, opener).await;
            });
        }
        if sched.is_finished(&clock) && worker.open.get() == 0 {
            return Control::Run;
        }
        let due = sched
            .next_due(&clock, worker.open.get())
            .map(|d| clock.instant_at(d));
        // Wake at least every RATE_POLL to read the rate handle, which also
        // ends the wait of a paused schedule, whose next_due is None.
        let wake = next_wake(due, Instant::now());
        tokio::select! {
            biased;
            r = control.changed() => {
                if r.is_err() {
                    return Control::Abort;
                }
            }
            () = sleep_until(wake) => {}
            () = worker.closed.notified() => {}
            _ = tick.tick() => worker.send_snapshot(false),
        }
    }
}

pub(crate) struct ClientCtx {
    pub(crate) connector: Connector,
    pub(crate) config: ClientConfig,
}

/// Accepts sessions until a stop arrives, and returns the stop.
async fn server_loop(
    worker: &Rc<Worker>,
    listeners: Vec<KernelListener>,
    control: &mut watch::Receiver<Control>,
    tick: &mut tokio::time::Interval,
) -> Control {
    let next_id = Rc::new(Cell::new(0u64));
    let mut accepts = Vec::with_capacity(listeners.len());
    for listener in listeners {
        let (w, next_id) = (Rc::clone(worker), Rc::clone(&next_id));
        accepts.push(tokio::task::spawn_local(async move {
            loop {
                let Ok((io, _peer)) = listener.accept().await else {
                    // One failed accept, such as EMFILE, leaves the listener usable.
                    tokio::task::yield_now().await;
                    continue;
                };
                let local_id = next_id.get();
                next_id.set(local_id + 1);
                w.open.set(w.open.get() + 1);
                let w = Rc::clone(&w);
                tokio::task::spawn_local(async move {
                    if w.is_instance() {
                        w.run_instance(local_id, Peer::Server(io)).await;
                        return;
                    }
                    let flow = match w.flow.get() {
                        Ok(f) => f,
                        Err(_) => return,
                    };
                    let Some(program) = flow.server.as_ref() else {
                        return;
                    };
                    let w2 = Rc::clone(&w);
                    w2.session(local_id, program, Accepted::new(io)).await;
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
    // Dropping the accept tasks closes the listeners, so no new session starts.
    for a in accepts {
        a.abort();
    }
    state
}

/// Copies an error for the ready report, which the thread result also holds.
fn clone_err(e: &RuntimeError) -> RuntimeError {
    RuntimeError::Config {
        detail: error_chain(e),
    }
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
    use super::*;

    #[test]
    fn a_wait_never_exceeds_the_poll_interval() {
        let now = Instant::now();
        assert_eq!(next_wake(None, now), now + RATE_POLL);
        let far = now + Duration::from_secs(5);
        assert_eq!(next_wake(Some(far), now), now + RATE_POLL);
        let near = now + Duration::from_millis(7);
        assert_eq!(next_wake(Some(near), now), near);
        // An overdue start wakes at once.
        assert_eq!(next_wake(Some(now), now), now);
    }
}
