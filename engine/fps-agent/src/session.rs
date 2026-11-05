// SPDX-License-Identifier: GPL-3.0-only
//! The controller session, `fps-agent connect`.
//!
//! The agent dials the controller, as `contracts/README.md` requires, and
//! serves no gRPC service. [`serve`] calls `Register`, opens the `Session`
//! stream, and then serves the commands of the controller until the shutdown
//! future completes. A lost connection leads to a new `Register` call after
//! an exponential backoff, and a run continues across the gap as long as
//! the controller returns within the heartbeat timeout.
//!
//! The agent state machine lives in one [`Core`] behind a mutex, which the
//! session task and the run driver thread share. The mutex is never held
//! across an `.await`.
//!
//! | Command | Allowed in | Effect |
//! |---|---|---|
//! | `prepare` | `IDLE`, `STOPPED`, `FAILED` | `PREPARING`, then `PREPARED` on a dedicated thread |
//! | `start` | `PREPARED` | `RUNNING`, the run driver thread starts the workers at once or at `start_at` |
//! | `stop` | `PREPARED` | Releases the plan and moves to `IDLE` |
//! | `stop` | `RUNNING`, `STOPPING` | Ramps down through `STOPPING`, or aborts, and answers with the final snapshot once the workers end |
//! | `stop` | `STOPPED` | Answers with the final snapshot of the run |
//!
//! Any other command state pair fails with `ERROR_CODE_FAILED_PRECONDITION`,
//! and a `plan_id` that does not name the loaded plan fails with
//! `ERROR_CODE_NOT_FOUND`. A failed command leaves the state unchanged,
//! except for `ERROR_CODE_INTERNAL`, which moves the agent to `FAILED`.
//!
//! The run driver thread takes one aggregate snapshot per counter interval
//! and queues it for the session. Snapshots queued while the connection is
//! down go out on the next session, with sequence numbers that continue, so
//! the controller drops duplicates and loses nothing. Command results belong
//! to the session that received the command and are dropped after it ends.

use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use fps_stats::{Snapshot, SnapshotMeta, Timestamp};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Endpoint;

use crate::assemble::Sides;
use crate::convert::{self, plan_from_pb, snapshot_to_pb};
use crate::modules::ModuleError;
use crate::pb::{self, AgentState, ErrorCode};
use crate::run::{self, Prepared, RunError, SERVER_RAMP_DOWN};

/// The heartbeat interval when `RegisterResponse` carries none.
pub const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
/// The heartbeat timeout when `RegisterResponse` carries none.
pub const DEFAULT_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(5);
/// The counter interval when `PrepareCommand` carries none.
pub const DEFAULT_COUNTER_INTERVAL: Duration = Duration::from_secs(1);
/// The drain timeout when `StopCommand` carries none.
pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the watchdog compares the controller silence with the timeout.
const WATCHDOG_TICK: Duration = Duration::from_millis(100);
/// How long a dial may take before it counts as failed.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The exponential backoff between two dials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    /// The wait after the first failure.
    pub initial: Duration,
    /// The longest wait.
    pub max: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(200),
            max: Duration::from_secs(10),
        }
    }
}

impl Backoff {
    /// Returns the wait that follows `current`, which doubles up to the
    /// maximum.
    pub fn next(&self, current: Duration) -> Duration {
        current.saturating_mul(2).min(self.max).max(self.initial)
    }
}

/// The default limit on redialing, five minutes of failed attempts
/// (decision D12 item 11).
pub const DEFAULT_REDIAL_LIMIT: Duration = Duration::from_secs(300);

/// What the agent does after a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Redial {
    /// Dial again after this wait, which already carries the jitter.
    After(Duration),
    /// The failed attempts lasted for the limit, so the agent gives up.
    GiveUp {
        /// How long the attempts have failed.
        failing_for: Duration,
    },
}

/// The redial policy of [`serve`]: an exponential [`Backoff`] with jitter that
/// restarts at its initial wait after an abort, and a limit on the time of
/// failed attempts.
///
/// A session that opened and ended without an abort clears the failure
/// window. A session that failed to open, or that ended with `ABORTED`,
/// starts the window when none runs and keeps it otherwise, so two agents of
/// one identity that replace each other forever still reach the limit. The
/// policy takes the time as an argument and draws its jitter from a seeded
/// generator, so tests replay it exactly.
#[derive(Debug, Clone)]
pub struct RedialPolicy {
    backoff: Backoff,
    limit: Option<Duration>,
    wait: Duration,
    failing_since: Option<Instant>,
    rng: u64,
}

impl RedialPolicy {
    /// Returns a policy with `backoff`, an optional `limit` on failed
    /// attempts, and a jitter stream that `seed` selects. A `limit` of `None`
    /// redials forever.
    pub fn new(backoff: Backoff, limit: Option<Duration>, seed: u64) -> Self {
        Self {
            wait: backoff.initial,
            backoff,
            limit,
            failing_since: None,
            rng: seed,
        }
    }

    /// Records that a session ended at `now` and returns what the agent does
    /// next. `opened` tells whether the session got past `Register` and the
    /// stream setup, and `aborted` whether it ended with `ABORTED`.
    pub fn session_ended(&mut self, now: Instant, opened: bool, aborted: bool) -> Redial {
        if aborted {
            // Another session of this identity replaced ours. The backoff
            // restarts, and the failure window keeps running.
            self.wait = self.backoff.initial;
            self.failing_since.get_or_insert(now);
        } else if opened {
            self.wait = self.backoff.initial;
            self.failing_since = None;
        } else {
            self.failing_since.get_or_insert(now);
        }
        let failing_for = self
            .failing_since
            .map_or(Duration::ZERO, |since| now.saturating_duration_since(since));
        if self.limit.is_some_and(|limit| failing_for >= limit) {
            return Redial::GiveUp { failing_for };
        }
        let wait = self.wait;
        self.wait = self.backoff.next(self.wait);
        Redial::After(self.jittered(wait))
    }

    /// Returns a duration drawn uniformly from the upper half of `wait`, so
    /// agents that lost the controller together do not dial together.
    fn jittered(&mut self, wait: Duration) -> Duration {
        let draw = splitmix64(&mut self.rng);
        // The top 53 bits give a fraction in [0, 1).
        let fraction = (draw >> 11) as f64 / (1u64 << 53) as f64;
        wait.mul_f64(0.5 + 0.5 * fraction)
    }
}

/// SplitMix64, which turns any seed, zero included, into a good stream.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The configuration of [`serve`].
#[derive(Debug, Clone)]
pub struct Config {
    /// The controller address as a URI, such as `http://10.0.0.1:7443`.
    pub controller: String,
    /// The agent identity, `RegisterRequest.agent_id`.
    pub agent_id: String,
    /// The directory that holds one directory per module.
    pub modules: PathBuf,
    /// The sides that this agent runs.
    pub sides: Sides,
    /// The capabilities that `Register` reports.
    pub capabilities: pb::Capabilities,
    /// The backoff between dials.
    pub backoff: Backoff,
    /// How long failed attempts may last before the agent gives up, or
    /// `None` to redial forever.
    pub redial_limit: Option<Duration>,
    /// The seed of the backoff jitter. The binary takes it from the clock,
    /// and tests fix it.
    pub jitter_seed: u64,
    /// How long a run with server workers only keeps them after the load
    /// phases end.
    pub linger: Duration,
}

impl Config {
    /// Returns a configuration with both sides, the default backoff, and a
    /// five second linger.
    pub fn new(controller: String, agent_id: String, modules: PathBuf) -> Self {
        Self {
            controller,
            agent_id,
            modules,
            sides: Sides::Both,
            capabilities: pb::Capabilities::default(),
            backoff: Backoff::default(),
            redial_limit: Some(DEFAULT_REDIAL_LIMIT),
            jitter_seed: 0,
            linger: Duration::from_secs(5),
        }
    }
}

/// Errors that end [`serve`]. Every other failure leads to a new dial.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// The controller address is not a valid URI.
    #[error("invalid controller address {address:?}")]
    Address {
        /// The address.
        address: String,
        /// The cause.
        #[source]
        source: tonic::transport::Error,
    },
    /// The controller stayed unreachable for the redial limit.
    #[error("gave up redialing the controller after {failing_for:?} of failed attempts, last error: {last}")]
    RedialLimit {
        /// How long the attempts failed, which is at least the limit.
        failing_for: Duration,
        /// Why the last session ended.
        last: String,
    },
}

/// Why one session ended.
#[derive(Debug)]
enum Lost {
    /// The dial or the TCP connection failed.
    Transport(tonic::transport::Error),
    /// A call ended with a gRPC status.
    Status(tonic::Status),
    /// The controller closed the stream.
    Closed,
    /// The controller stayed silent for the heartbeat timeout.
    Silent,
}

impl std::fmt::Display for Lost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Lost::Transport(e) => write!(f, "transport: {e}"),
            Lost::Status(s) => write!(f, "status {:?}: {}", s.code(), s.message()),
            Lost::Closed => f.write_str("the controller closed the stream"),
            Lost::Silent => f.write_str("the controller stayed silent for the heartbeat timeout"),
        }
    }
}

/// Where a command result goes.
#[derive(Debug, Clone, Copy)]
struct Reply {
    session: u64,
    command_id: u64,
}

/// One message from the run driver or a command thread to the session.
#[derive(Debug)]
enum Event {
    /// A command result for the session that received the command.
    Result(u64, pb::CommandResult),
    /// A counter snapshot of the current run.
    Snapshot(pb::CounterSnapshot),
    /// The state changed, so a heartbeat goes out at once.
    StateChanged,
    /// The watchdog saw the heartbeat timeout pass.
    Silent,
}

/// The run driver thread of a started plan.
#[derive(Debug)]
struct Driver {
    control: std_mpsc::Sender<Duration>,
    thread: JoinHandle<()>,
}

/// The parameters of a prepared plan from its `PrepareCommand`.
#[derive(Debug, Clone, Copy)]
struct RunParams {
    interval: Duration,
}

/// The agent state that the session and the run driver share.
#[derive(Debug)]
struct Core {
    state: AgentState,
    plan_id: String,
    phase: String,
    message: String,
    prepared: Option<(Prepared, RunParams)>,
    driver: Option<Driver>,
    /// Increases on every prepare and every release, so a run driver whose
    /// plan was released leaves the state alone.
    generation: u64,
    last_final: Option<pb::CounterSnapshot>,
    stop_waiters: Vec<Reply>,
    start_waiter: Option<Reply>,
    last_heard: Option<Instant>,
    timeout: Duration,
}

struct Shared {
    cfg: Config,
    core: Mutex<Core>,
    events: mpsc::UnboundedSender<Event>,
}

fn state_name(s: AgentState) -> &'static str {
    s.as_str_name()
}

fn error(reply: Reply, code: ErrorCode, message: String) -> pb::CommandResult {
    pb::CommandResult {
        command_id: reply.command_id,
        error: Some(pb::CommandError {
            code: code as i32,
            message,
        }),
        result: None,
    }
}

fn success(reply: Reply, result: pb::command_result::Result) -> pb::CommandResult {
    pb::CommandResult {
        command_id: reply.command_id,
        error: None,
        result: Some(result),
    }
}

/// Maps a run error to the error code of the contract.
fn code_of(e: &RunError) -> ErrorCode {
    match e {
        RunError::Plan(_) => ErrorCode::InvalidArgument,
        RunError::Module(m) => match m {
            ModuleError::Digest { .. }
            | ModuleError::Mismatch { .. }
            | ModuleError::Read { .. }
            | ModuleError::Forbidden { .. }
            | ModuleError::Manifest { .. } => ErrorCode::FailedPrecondition,
            ModuleError::ScenarioDigest { .. }
            | ModuleError::Scenario { .. }
            | ModuleError::ScenarioName { .. }
            | ModuleError::Param { .. } => ErrorCode::InvalidArgument,
        },
        RunError::Unimplemented(_) => ErrorCode::Unimplemented,
        RunError::Runtime(_) | RunError::Udp(_) | RunError::Stats(_) | RunError::Output(_) => {
            ErrorCode::Internal
        }
    }
}

fn chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        s.push_str(": ");
        s.push_str(&c.to_string());
        cur = c.source();
    }
    s
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Core> {
        self.core.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn emit(&self, ev: Event) {
        // The receiver lives as long as serve, so a failed send means that
        // serve is returning and nobody needs the event.
        let _ = self.events.send(ev);
    }

    fn heartbeat(&self, sequence: u64) -> pb::SessionRequest {
        let core = self.lock();
        pb::SessionRequest {
            body: Some(pb::session_request::Body::Heartbeat(pb::Heartbeat {
                sequence,
                state: core.state as i32,
                plan_id: core.plan_id.clone(),
                phase: core.phase.clone(),
                agent_time: Some(convert::timestamp(SystemTime::now())),
                message: core.message.clone(),
            })),
        }
    }

    /// Serves one command and returns its result when it is known at once.
    fn command(
        self: &Arc<Self>,
        session: u64,
        resp: pb::SessionResponse,
    ) -> Option<pb::CommandResult> {
        use pb::session_response::Body;
        let reply = Reply {
            session,
            command_id: resp.command_id,
        };
        match resp.body {
            None => Some(error(
                reply,
                ErrorCode::InvalidArgument,
                "the command has no body".into(),
            )),
            Some(Body::HeartbeatAck(_)) => None,
            Some(Body::Prepare(cmd)) => self.prepare(reply, cmd),
            Some(Body::Start(cmd)) => self.start(reply, &cmd),
            Some(Body::Stop(cmd)) => self.stop(reply, &cmd),
        }
    }

    fn prepare(
        self: &Arc<Self>,
        reply: Reply,
        cmd: pb::PrepareCommand,
    ) -> Option<pb::CommandResult> {
        let mut core = self.lock();
        if !matches!(
            core.state,
            AgentState::Idle | AgentState::Stopped | AgentState::Failed
        ) {
            return Some(error(
                reply,
                ErrorCode::FailedPrecondition,
                format!("prepare is not allowed in {}", state_name(core.state)),
            ));
        }
        let Some(plan) = cmd.plan else {
            return Some(error(
                reply,
                ErrorCode::InvalidArgument,
                "prepare carries no plan".into(),
            ));
        };
        let interval = match cmd.counter_interval.as_ref() {
            None => DEFAULT_COUNTER_INTERVAL,
            Some(d) => match convert::duration("counter_interval", Some(d)) {
                Ok(d) if !d.is_zero() => d,
                _ => {
                    return Some(error(
                        reply,
                        ErrorCode::InvalidArgument,
                        "counter_interval must be positive".into(),
                    ))
                }
            },
        };
        if cmd.per_worker {
            tracing::warn!(
                plan_id = %plan.plan_id,
                "per worker snapshots are not implemented, the agent sends aggregate snapshots"
            );
        }
        let previous = (core.state, core.plan_id.clone(), core.message.clone());
        core.state = AgentState::Preparing;
        core.plan_id.clone_from(&plan.plan_id);
        core.message.clear();
        core.generation += 1;
        let generation = core.generation;
        let old = core.driver.take();
        drop(core);
        self.emit(Event::StateChanged);
        let shared = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("fps-agent-prepare".into())
            .spawn(move || {
                if let Some(d) = old {
                    let _ = d.thread.join();
                }
                let outcome = shared.load(&plan);
                let mut core = shared.lock();
                if core.generation != generation {
                    return;
                }
                let result = match outcome {
                    Ok(p) => {
                        tracing::info!(plan_id = %plan.plan_id, summary = %p.summary(), "plan prepared");
                        core.state = AgentState::Prepared;
                        core.prepared = Some((p, RunParams { interval }));
                        core.last_final = None;
                        success(
                            reply,
                            pb::command_result::Result::Prepare(pb::PrepareResult {
                                plan_id: plan.plan_id.clone(),
                                state: AgentState::Prepared as i32,
                            }),
                        )
                    }
                    Err((code, message)) => {
                        tracing::warn!(plan_id = %plan.plan_id, code = code.as_str_name(), message = %message, "prepare failed");
                        if code == ErrorCode::Internal {
                            core.state = AgentState::Failed;
                            core.message.clone_from(&message);
                        } else {
                            (core.state, core.plan_id, core.message) = previous;
                        }
                        error(reply, code, message)
                    }
                };
                drop(core);
                shared.emit(Event::Result(reply.session, result));
                shared.emit(Event::StateChanged);
            });
        if let Err(e) = spawned {
            let mut core = self.lock();
            core.state = AgentState::Failed;
            core.message = format!("spawn the prepare thread: {e}");
            let message = core.message.clone();
            drop(core);
            self.emit(Event::StateChanged);
            return Some(error(reply, ErrorCode::Internal, message));
        }
        None
    }

    /// Converts and checks the plan and loads its modules.
    fn load(&self, plan: &pb::Plan) -> Result<Prepared, (ErrorCode, String)> {
        let plan = plan_from_pb(plan).map_err(|e| (ErrorCode::InvalidArgument, chain(&e)))?;
        if !plan.agent_id.is_empty() && plan.agent_id != self.cfg.agent_id {
            return Err((
                ErrorCode::InvalidArgument,
                format!(
                    "the plan is for agent {}, not {}",
                    plan.agent_id, self.cfg.agent_id
                ),
            ));
        }
        let cores = self.cfg.capabilities.cores as usize;
        if cores > 0 && plan.workers.len() > cores {
            return Err((
                ErrorCode::ResourceExhausted,
                format!(
                    "the plan needs {} workers and the agent has {} cores",
                    plan.workers.len(),
                    cores
                ),
            ));
        }
        let prepared = run::prepare(plan, &self.cfg.modules, self.cfg.sides)
            .map_err(|e| (code_of(&e), chain(&e)))?;
        for note in prepared.notes() {
            tracing::info!(note = %note, "module note");
        }
        Ok(prepared)
    }

    fn start(self: &Arc<Self>, reply: Reply, cmd: &pb::StartCommand) -> Option<pb::CommandResult> {
        let mut core = self.lock();
        if core.state != AgentState::Prepared {
            return Some(error(
                reply,
                ErrorCode::FailedPrecondition,
                format!("start is not allowed in {}", state_name(core.state)),
            ));
        }
        if cmd.plan_id != core.plan_id {
            return Some(error(
                reply,
                ErrorCode::NotFound,
                format!("plan {} is not loaded", cmd.plan_id),
            ));
        }
        let Some((prepared, params)) = core.prepared.take() else {
            core.state = AgentState::Failed;
            core.message = "invariant: a prepared agent holds its plan".into();
            let message = core.message.clone();
            drop(core);
            self.emit(Event::StateChanged);
            return Some(error(reply, ErrorCode::Internal, message));
        };
        let start_at = cmd
            .start_at
            .as_ref()
            .map(convert::system_time)
            .filter(|t| *t > SystemTime::now());
        let (control, rx) = std_mpsc::channel();
        let generation = core.generation;
        let shared = Arc::clone(self);
        core.start_waiter = if start_at.is_none() {
            Some(reply)
        } else {
            None
        };
        let spawned = std::thread::Builder::new()
            .name("fps-agent-run".into())
            .spawn(move || shared.drive(generation, prepared, params, start_at, &rx));
        match spawned {
            Ok(thread) => {
                core.state = AgentState::Running;
                core.phase.clear();
                core.driver = Some(Driver { control, thread });
                drop(core);
                tracing::info!(plan_id = %cmd.plan_id, "run started");
                self.emit(Event::StateChanged);
                start_at.map(|t| {
                    success(
                        reply,
                        pb::command_result::Result::Start(pb::StartResult {
                            started_at: Some(convert::timestamp(t)),
                        }),
                    )
                })
            }
            Err(e) => {
                core.start_waiter = None;
                core.state = AgentState::Failed;
                core.message = format!("spawn the run thread: {e}");
                let message = core.message.clone();
                drop(core);
                self.emit(Event::StateChanged);
                Some(error(reply, ErrorCode::Internal, message))
            }
        }
    }

    fn stop(self: &Arc<Self>, reply: Reply, cmd: &pb::StopCommand) -> Option<pb::CommandResult> {
        let mut core = self.lock();
        let allowed = matches!(
            core.state,
            AgentState::Prepared | AgentState::Running | AgentState::Stopping | AgentState::Stopped
        );
        if !allowed {
            return Some(error(
                reply,
                ErrorCode::FailedPrecondition,
                format!("stop is not allowed in {}", state_name(core.state)),
            ));
        }
        if cmd.plan_id != core.plan_id {
            return Some(error(
                reply,
                ErrorCode::NotFound,
                format!("plan {} is not loaded", cmd.plan_id),
            ));
        }
        let abort = cmd.mode == pb::StopMode::Abort as i32;
        let drain = if abort {
            Duration::ZERO
        } else {
            match cmd.drain_timeout.as_ref() {
                None => DEFAULT_DRAIN_TIMEOUT,
                Some(d) => match convert::duration("drain_timeout", Some(d)) {
                    Ok(d) => d,
                    Err(e) => return Some(error(reply, ErrorCode::InvalidArgument, e.to_string())),
                },
            }
        };
        match core.state {
            AgentState::Prepared => {
                core.prepared = None;
                core.state = AgentState::Idle;
                core.plan_id.clear();
                drop(core);
                tracing::info!(plan_id = %cmd.plan_id, "plan released");
                self.emit(Event::StateChanged);
                Some(success(
                    reply,
                    pb::command_result::Result::Stop(pb::StopResult {
                        state: AgentState::Idle as i32,
                        final_snapshot: None,
                    }),
                ))
            }
            AgentState::Stopped => Some(success(
                reply,
                pb::command_result::Result::Stop(pb::StopResult {
                    state: AgentState::Stopped as i32,
                    final_snapshot: core.last_final.clone(),
                }),
            )),
            _ => {
                core.stop_waiters.push(reply);
                if let Some(d) = &core.driver {
                    // The driver ends only after it answered the waiters, so
                    // a failed send means that it is answering them now.
                    let _ = d.control.send(drain);
                }
                let changed = !abort && core.state == AgentState::Running;
                if changed {
                    core.state = AgentState::Stopping;
                }
                drop(core);
                tracing::info!(plan_id = %cmd.plan_id, abort, drain_ms = drain.as_millis() as u64, "run stopping");
                if changed {
                    self.emit(Event::StateChanged);
                }
                None
            }
        }
    }

    /// Runs a started plan on the run driver thread.
    fn drive(
        &self,
        generation: u64,
        prepared: Prepared,
        params: RunParams,
        start_at: Option<SystemTime>,
        control: &std_mpsc::Receiver<Duration>,
    ) {
        let plan_id = prepared.plan().plan_id.clone();
        let mut stop: Option<Duration> = None;
        let take_stops = |stop: &mut Option<Duration>| {
            let mut got = false;
            while let Ok(d) = control.try_recv() {
                *stop = Some(stop.map_or(d, |s| s.min(d)));
                got = true;
            }
            got
        };
        if let Some(t) = start_at {
            while stop.is_none() {
                take_stops(&mut stop);
                let Ok(wait) = t.duration_since(SystemTime::now()) else {
                    break;
                };
                std::thread::sleep(wait.min(Duration::from_millis(20)));
            }
            if stop.is_some() {
                let meta = self.meta(&plan_id, 1, String::new());
                self.finished(
                    generation,
                    snapshot_to_pb(&Snapshot::aggregate(), &meta).ok(),
                );
                return;
            }
        }
        let mut active = match prepared.start(params.interval) {
            Ok(a) => a,
            Err(e) => {
                self.failed(generation, format!("start the workers: {}", chain(&e)));
                return;
            }
        };
        if let Some(reply) = self.lock().start_waiter.take() {
            self.emit(Event::Result(
                reply.session,
                success(
                    reply,
                    pb::command_result::Result::Start(pb::StartResult {
                        started_at: Some(convert::timestamp(SystemTime::now())),
                    }),
                ),
            ));
        }
        let mut sequence = 0u64;
        let mut next = Instant::now() + params.interval;
        loop {
            if take_stops(&mut stop) {
                if let Some(d) = stop {
                    active.stop(d);
                }
            }
            if stop.is_some() {
                if active.clients_finished() {
                    break;
                }
            } else if active.is_done(self.cfg.linger) {
                break;
            }
            if let Some(message) = active.enforce_grace() {
                tracing::warn!(plan_id = %plan_id, "{message}");
            }
            let now = Instant::now();
            if now < next {
                std::thread::sleep((next - now).min(Duration::from_millis(20)));
                continue;
            }
            next += params.interval;
            let elapsed = active.elapsed();
            let phase = active.phase_at(elapsed);
            let snap = match active.sample() {
                Ok(s) => s,
                Err(e) => {
                    active.abort();
                    self.failed(generation, format!("take a snapshot: {}", chain(&e)));
                    return;
                }
            };
            sequence += 1;
            let meta = self.meta(&plan_id, sequence, phase.clone());
            match snapshot_to_pb(&snap, &meta) {
                Ok(s) => {
                    let mut core = self.lock();
                    if core.generation != generation {
                        continue;
                    }
                    let changed = core.phase != phase;
                    core.phase = phase;
                    drop(core);
                    self.emit(Event::Snapshot(s));
                    if changed {
                        self.emit(Event::StateChanged);
                    }
                }
                Err(e) => tracing::warn!(plan_id = %plan_id, error = %e, "snapshot dropped"),
            }
        }
        let server_ramp = stop.map_or(SERVER_RAMP_DOWN, |d| d.min(SERVER_RAMP_DOWN));
        let phase = active.phase_at(active.elapsed());
        match active.finish(server_ramp) {
            Ok(f) => {
                let meta = SnapshotMeta {
                    is_final: true,
                    ..self.meta(&plan_id, sequence + 1, phase)
                };
                match snapshot_to_pb(&f.shown, &meta) {
                    Ok(s) => self.finished(generation, Some(s)),
                    Err(e) => self.failed(generation, format!("final snapshot: {e}")),
                }
            }
            Err(e) => self.failed(generation, format!("run: {}", chain(&e))),
        }
    }

    fn meta(&self, plan_id: &str, sequence: u64, phase: String) -> SnapshotMeta {
        SnapshotMeta {
            plan_id: plan_id.to_owned(),
            sequence,
            time: Timestamp::from_system_time(SystemTime::now()),
            phase,
            is_final: false,
        }
    }

    /// Moves to `STOPPED`, sends the final snapshot, and answers the stop
    /// commands that wait for it.
    fn finished(&self, generation: u64, last: Option<pb::CounterSnapshot>) {
        let mut core = self.lock();
        if core.generation != generation {
            return;
        }
        core.state = AgentState::Stopped;
        core.phase.clear();
        core.last_final.clone_from(&last);
        let waiters = std::mem::take(&mut core.stop_waiters);
        let start = core.start_waiter.take();
        drop(core);
        tracing::info!(
            plan_id = last.as_ref().map_or("", |s| s.plan_id.as_str()),
            "run stopped"
        );
        if let Some(s) = &last {
            self.emit(Event::Snapshot(s.clone()));
        }
        if let Some(reply) = start {
            self.emit(Event::Result(
                reply.session,
                success(
                    reply,
                    pb::command_result::Result::Start(pb::StartResult {
                        started_at: Some(convert::timestamp(SystemTime::now())),
                    }),
                ),
            ));
        }
        for reply in waiters {
            self.emit(Event::Result(
                reply.session,
                success(
                    reply,
                    pb::command_result::Result::Stop(pb::StopResult {
                        state: AgentState::Stopped as i32,
                        final_snapshot: last.clone(),
                    }),
                ),
            ));
        }
        self.emit(Event::StateChanged);
    }

    /// Moves to `FAILED` with `message` and fails the commands that wait.
    fn failed(&self, generation: u64, message: String) {
        let mut core = self.lock();
        if core.generation != generation {
            return;
        }
        core.state = AgentState::Failed;
        core.phase.clear();
        core.message.clone_from(&message);
        let mut waiters = std::mem::take(&mut core.stop_waiters);
        waiters.extend(core.start_waiter.take());
        drop(core);
        tracing::error!(message = %message, "run failed");
        for reply in waiters {
            self.emit(Event::Result(
                reply.session,
                error(reply, ErrorCode::Internal, message.clone()),
            ));
        }
        self.emit(Event::StateChanged);
    }

    /// Aborts any run, releases the plan, and moves to `IDLE`, because the
    /// controller stayed silent for the heartbeat timeout.
    fn release(&self) {
        let mut core = self.lock();
        core.generation += 1;
        let changed = core.state != AgentState::Idle;
        core.state = AgentState::Idle;
        core.plan_id.clear();
        core.phase.clear();
        core.message.clear();
        core.prepared = None;
        core.stop_waiters.clear();
        core.start_waiter = None;
        if let Some(d) = &core.driver {
            let _ = d.control.send(Duration::ZERO);
        }
        drop(core);
        if changed {
            tracing::warn!("the controller stayed silent for the heartbeat timeout, the run is aborted and the plan released");
            self.emit(Event::StateChanged);
        }
    }

    /// Returns true when the controller sent nothing for the heartbeat
    /// timeout, or when the watchdog already fired for this silence.
    fn is_silent(&self) -> bool {
        let core = self.lock();
        core.last_heard.is_none_or(|t| t.elapsed() > core.timeout)
    }

    fn heard(&self) {
        self.lock().last_heard = Some(Instant::now());
    }
}

/// Fires once per silence of the controller.
async fn watchdog(shared: Arc<Shared>) {
    loop {
        tokio::time::sleep(WATCHDOG_TICK).await;
        let fired = {
            let mut core = shared.lock();
            match core.last_heard {
                Some(t) if t.elapsed() > core.timeout => {
                    core.last_heard = None;
                    true
                }
                _ => false,
            }
        };
        if fired {
            shared.release();
            shared.emit(Event::Silent);
        }
    }
}

/// Registers with the controller and serves one session until it ends.
/// Returns `Ok` when the shutdown future completed.
async fn session<F>(
    shared: &Arc<Shared>,
    events: &mut mpsc::UnboundedReceiver<Event>,
    number: u64,
    shutdown: &mut std::pin::Pin<&mut F>,
    opened: &mut bool,
) -> Result<(), Lost>
where
    F: std::future::Future<Output = ()>,
{
    let endpoint = Endpoint::from_shared(shared.cfg.controller.clone())
        .map_err(Lost::Transport)?
        .connect_timeout(CONNECT_TIMEOUT)
        .tcp_nodelay(true);
    let channel = tokio::select! {
        () = shutdown.as_mut() => return Ok(()),
        c = endpoint.connect() => c.map_err(Lost::Transport)?,
    };
    let mut client = pb::controller_service_client::ControllerServiceClient::new(channel);
    let (state, plan_id) = {
        let core = shared.lock();
        (core.state, core.plan_id.clone())
    };
    let reg = client
        .register(pb::RegisterRequest {
            agent_id: shared.cfg.agent_id.clone(),
            capabilities: Some(shared.cfg.capabilities.clone()),
            state: state as i32,
            plan_id,
        })
        .await
        .map_err(Lost::Status)?
        .into_inner();
    let interval = match convert::duration("heartbeat_interval", reg.heartbeat_interval.as_ref()) {
        Ok(d) if !d.is_zero() => d,
        _ => DEFAULT_HEARTBEAT_INTERVAL,
    };
    let timeout = match convert::duration("heartbeat_timeout", reg.heartbeat_timeout.as_ref()) {
        Ok(d) if d > interval => d,
        _ => DEFAULT_HEARTBEAT_TIMEOUT.max(interval * 2),
    };
    {
        let mut core = shared.lock();
        core.timeout = timeout;
        core.last_heard = Some(Instant::now());
    }
    let (tx, rx) = mpsc::channel::<pb::SessionRequest>(64);
    let send = |msg: pb::SessionRequest| {
        let tx = tx.clone();
        async move { tx.send(msg).await.map_err(|_| Lost::Closed) }
    };
    send(pb::SessionRequest {
        body: Some(pb::session_request::Body::Open(pb::SessionOpen {
            agent_id: shared.cfg.agent_id.clone(),
            session_token: reg.session_token.clone(),
        })),
    })
    .await?;
    let mut inbound = client
        .session(ReceiverStream::new(rx))
        .await
        .map_err(Lost::Status)?
        .into_inner();
    *opened = true;
    tracing::info!(controller_id = %reg.controller_id, session = number, interval_ms = interval.as_millis() as u64, timeout_ms = timeout.as_millis() as u64, "session open");
    let mut sequence = 0u64;
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = shutdown.as_mut() => return Ok(()),
            _ = ticker.tick() => {
                sequence += 1;
                send(shared.heartbeat(sequence)).await?;
            }
            msg = inbound.message() => match msg {
                Ok(Some(resp)) => {
                    shared.heard();
                    if let Some(result) = shared.command(number, resp) {
                        send(pb::SessionRequest { body: Some(pb::session_request::Body::Result(result)) }).await?;
                    }
                }
                Ok(None) => return Err(Lost::Closed),
                Err(status) => return Err(Lost::Status(status)),
            },
            ev = events.recv() => match ev {
                // serve holds a sender, so the channel never closes here.
                None => return Err(Lost::Closed),
                Some(Event::Result(s, result)) => {
                    if s == number {
                        send(pb::SessionRequest { body: Some(pb::session_request::Body::Result(result)) }).await?;
                    }
                }
                Some(Event::Snapshot(snap)) => {
                    send(pb::SessionRequest { body: Some(pb::session_request::Body::Snapshot(snap)) }).await?;
                }
                Some(Event::StateChanged) => {
                    sequence += 1;
                    send(shared.heartbeat(sequence)).await?;
                }
                Some(Event::Silent) => {
                    if shared.is_silent() {
                        return Err(Lost::Silent);
                    }
                }
            },
        }
    }
}

/// Dials the controller of `cfg` and serves its commands until `shutdown`
/// completes. Every lost connection leads to a new dial after the backoff of
/// [`RedialPolicy`]. When the dials fail for `cfg.redial_limit`, the agent
/// aborts any run, releases the plan, and returns
/// [`SessionError::RedialLimit`]. On shutdown the agent aborts any run and
/// waits for its workers.
///
/// # Errors
///
/// Fails when the controller address is invalid or when the redial limit
/// passes.
pub async fn serve<F>(cfg: Config, shutdown: F) -> Result<(), SessionError>
where
    F: std::future::Future<Output = ()>,
{
    if let Err(source) = Endpoint::from_shared(cfg.controller.clone()) {
        return Err(SessionError::Address {
            address: cfg.controller.clone(),
            source,
        });
    }
    let (tx, mut events) = mpsc::unbounded_channel();
    let shared = Arc::new(Shared {
        core: Mutex::new(Core {
            state: AgentState::Idle,
            plan_id: String::new(),
            phase: String::new(),
            message: String::new(),
            prepared: None,
            driver: None,
            generation: 0,
            last_final: None,
            stop_waiters: Vec::new(),
            start_waiter: None,
            last_heard: None,
            timeout: DEFAULT_HEARTBEAT_TIMEOUT,
        }),
        events: tx,
        cfg,
    });
    let dog = tokio::spawn(watchdog(Arc::clone(&shared)));
    tokio::pin!(shutdown);
    let mut policy = RedialPolicy::new(
        shared.cfg.backoff,
        shared.cfg.redial_limit,
        shared.cfg.jitter_seed,
    );
    let mut number = 0u64;
    let mut outcome = Ok(());
    loop {
        number += 1;
        let mut opened = false;
        let wait = match session(&shared, &mut events, number, &mut shutdown, &mut opened).await {
            Ok(()) => break,
            Err(lost) => {
                let aborted = matches!(&lost, Lost::Status(s) if s.code() == tonic::Code::Aborted);
                match policy.session_ended(Instant::now(), opened, aborted) {
                    Redial::After(wait) => {
                        tracing::warn!(session = number, opened, aborted, error = %lost, backoff_ms = wait.as_millis() as u64, "session lost");
                        wait
                    }
                    Redial::GiveUp { failing_for } => {
                        tracing::error!(session = number, opened, aborted, error = %lost, failing_for_ms = failing_for.as_millis() as u64, "redial limit reached, the agent exits");
                        outcome = Err(SessionError::RedialLimit {
                            failing_for,
                            last: lost.to_string(),
                        });
                        break;
                    }
                }
            }
        };
        tokio::select! {
            () = shutdown.as_mut() => break,
            () = tokio::time::sleep(wait) => {}
        }
    }
    dog.abort();
    let driver = {
        let mut core = shared.lock();
        core.generation += 1;
        if let Some(d) = &core.driver {
            let _ = d.control.send(Duration::ZERO);
        }
        core.driver.take()
    };
    if let Some(d) = driver {
        let _ = tokio::task::spawn_blocking(move || d.thread.join()).await;
    }
    tracing::info!("agent stopped");
    outcome
}
