// SPDX-License-Identifier: GPL-3.0-only
//! Scenario instances, which run every flow of one scenario together.
//!
//! A [`ScenarioInstance`] owns the shared scenario variables and the state
//! of every flow. It starts each flow that is not spawned once every flow of
//! its `after` list completed, and it starts the same side of a flow with
//! `spawned: true` each time a side runs `spawn`. A flow side reads the
//! shared variables as they stood when it started, and a spawned side also
//! reads every `set` of its parent side up to the `spawn`. When a side
//! completes, the variables that its `set` actions bound join the shared
//! variables, so later flows read them.
//!
//! The first failure of any side fails the instance. The instance then
//! aborts every running side with reason `aborted`, starts no further
//! flows, and marks the flows that never started as skipped. A failed
//! spawned flow also fails the flow that spawned it and every flow up its
//! spawn chain, so the error names the whole chain.
//!
//! The instance runs all sides on the task that awaits [`ScenarioInstance::run`],
//! so the future is not `Send`. The engine runs it on the current thread
//! runtime of a worker, as it runs single sessions.

use std::collections::HashMap;
use std::fmt;
use std::future::{poll_fn, Future};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::Poll;

use fps_flow::program::{Flow, Program, L4};
use fps_flow::template::Filler;
use fps_flow::Clock;
use fps_flow::{Scenario, TemplateFn, Vars};
use tokio::sync::watch;

use crate::failure::{Failure, FailureKind, Reason};
use crate::open::{DatagramOpener, Opener};
use crate::rng::{session_rng, Side};
use crate::session::{Limits, Session, SessionCtx, SideStats};
use crate::upgrade::TlsUpgrader;

/// A `spawn` that a session side ran.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnRequest {
    /// Flow id of the side that ran `spawn`.
    pub parent: String,
    /// The side that ran `spawn`, which is also the side that starts.
    pub side: Side,
    /// Flow id that `spawn` names.
    pub flow: String,
    /// The variables that the parent side saw at the `spawn`, its `set`
    /// bindings included.
    pub vars: Vars,
}

/// The queue through which session sides hand `spawn` actions to their
/// scenario instance. Clones share one queue.
#[derive(Clone, Debug, Default)]
pub struct SpawnQueue(Arc<Mutex<Vec<SpawnRequest>>>);

impl SpawnQueue {
    /// Creates an empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a request.
    pub fn push(&self, request: SpawnRequest) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
    }

    /// Removes and returns every queued request in the order of `push`.
    pub fn drain(&self) -> Vec<SpawnRequest> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// The sides of every flow that an instance runs on this agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sides {
    /// Only client sides, as a client simulator runs them.
    Client,
    /// Only server sides, as a server simulator runs them.
    Server,
    /// Both sides against each other, as a test or a single agent runs them.
    Both,
}

impl Sides {
    /// Returns true when the instance runs `side`.
    pub fn contains(self, side: Side) -> bool {
        matches!(
            (self, side),
            (Sides::Both, _) | (Sides::Client, Side::Client) | (Sides::Server, Side::Server)
        )
    }
}

/// Settings of one scenario instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstanceConfig {
    /// The sides that this instance runs.
    pub sides: Sides,
    /// The run seed, from which every side derives its generator.
    pub run_seed: u64,
    /// Session id of the first flow session. Each further flow session,
    /// including every spawn, takes the next id in start order, and both
    /// sides of one flow session share one id.
    pub first_session_id: u64,
    /// Limits and timeouts of every side.
    pub limits: Limits,
}

/// What a scenario instance needs from the engine for each flow side.
///
/// The instance asks for a fresh opener and fresh module functions for
/// every side that it starts, so sides run concurrently.
pub trait FlowEnv {
    /// Opener of TCP flow sides.
    type Stream: Opener;
    /// Opener of UDP flow sides.
    type Datagram: DatagramOpener;

    /// Returns the opener of one side of a TCP flow.
    fn stream(&mut self, flow: &Flow, side: Side) -> Self::Stream;

    /// Returns the opener of one side of a UDP flow.
    fn datagram(&mut self, flow: &Flow, side: Side) -> Self::Datagram;

    /// Returns the module functions of one flow side.
    fn functions(&mut self, flow: &Flow, side: Side) -> Box<dyn TemplateFn>;

    /// Returns the upgrader that serves the `tls_upgrade` actions of one TCP
    /// flow side, or `None` when the engine has no TLS profiles.
    ///
    /// The instance asks once for every side that it starts, right after
    /// [`FlowEnv::functions`], and the side owns the upgrader until it ends.
    /// A side without an upgrader fails at its first `tls_upgrade` with
    /// reason `internal`. The default returns `None`.
    fn tls_upgrader(&mut self, _flow: &Flow, _side: Side) -> Option<Box<dyn TlsUpgrader>> {
        None
    }
}

/// The state of one flow of an instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowState {
    /// The flow waits for its `after` list or for a `spawn`.
    Waiting,
    /// At least one side of the flow runs.
    Running,
    /// Every side that started completed.
    Done,
    /// A side failed, or a flow that this flow spawned failed.
    Failed,
    /// The instance aborted a running side after another flow failed or the
    /// caller stopped the instance.
    Aborted,
    /// The flow never started, because the instance failed first or no side
    /// spawned it.
    Skipped,
}

impl FlowState {
    /// Returns a lower case name for logs and tests.
    pub fn as_str(self) -> &'static str {
        match self {
            FlowState::Waiting => "waiting",
            FlowState::Running => "running",
            FlowState::Done => "done",
            FlowState::Failed => "failed",
            FlowState::Aborted => "aborted",
            FlowState::Skipped => "skipped",
        }
    }
}

impl fmt::Display for FlowState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One finished flow side, in completion order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SideRecord {
    /// Flow id.
    pub flow: String,
    /// The side.
    pub side: Side,
    /// The session id of the side.
    pub session_id: u64,
    /// Byte counters of the side.
    pub stats: SideStats,
    /// The catalog reason when the side failed.
    pub failed: Option<Reason>,
}

/// The failure that failed a scenario instance.
#[derive(Debug, thiserror::Error)]
#[error("scenario {scenario} failed in flow {flow}{}: {failure}", chain(.spawned_by))]
pub struct InstanceError {
    /// Scenario name.
    pub scenario: String,
    /// Flow id of the first side that failed.
    pub flow: String,
    /// The flows that spawned the failing flow, the direct parent first.
    /// The list is empty for a flow that is not spawned.
    pub spawned_by: Vec<String>,
    /// The first failure, boxed so that `Result<(), InstanceError>` stays
    /// small.
    #[source]
    pub failure: Box<Failure>,
}

fn chain(spawned_by: &[String]) -> String {
    if spawned_by.is_empty() {
        String::new()
    } else {
        format!(" spawned by {}", spawned_by.join(" <- "))
    }
}

/// One scenario instance. See the module documentation.
pub struct ScenarioInstance<'s> {
    scenario: &'s Scenario,
    config: InstanceConfig,
    index: HashMap<&'s str, usize>,
    vars: Vars,
    states: Vec<FlowState>,
    running: Vec<usize>,
    parent: Vec<Option<usize>>,
    occurrences: HashMap<(usize, Side), u32>,
    ids: HashMap<(usize, u32), u64>,
    next_id: u64,
    records: Vec<SideRecord>,
    failures: Vec<Failure>,
    error: Option<InstanceError>,
    aborting: bool,
    clock: Option<Arc<dyn Clock>>,
}

/// What a finished side task returns.
struct SideDone {
    flow: usize,
    side: Side,
    session_id: u64,
    result: Result<(), Failure>,
    stats: SideStats,
    overlay: Vars,
}

type SideTask<'a> = Pin<Box<dyn Future<Output = SideDone> + 'a>>;

/// The parts of a side task that do not depend on the transport.
struct SideJob<'a> {
    flow_index: usize,
    flow: &'a Flow,
    program: &'a Program,
    side: Side,
    session_id: u64,
    vars: Vars,
    filler: &'a Filler,
    functions: Box<dyn TemplateFn>,
    upgrader: Option<Box<dyn TlsUpgrader>>,
    clock: Option<Arc<dyn Clock>>,
    config: InstanceConfig,
    queue: SpawnQueue,
    abort: watch::Receiver<bool>,
}

impl SideJob<'_> {
    fn ctx<'b>(&'b self, functions: &'b mut dyn TemplateFn) -> SessionCtx<'b> {
        SessionCtx {
            vars: &self.vars,
            filler: self.filler,
            flow: &self.flow.id,
            side: self.side,
            session_id: self.session_id,
            rng: session_rng(self.config.run_seed, self.session_id, self.side),
            functions,
            limits: self.config.limits,
        }
    }

    fn done(&self, result: Result<(), Failure>, stats: SideStats, overlay: Vars) -> SideDone {
        SideDone {
            flow: self.flow_index,
            side: self.side,
            session_id: self.session_id,
            result,
            stats,
            overlay,
        }
    }
}

async fn aborted(mut rx: watch::Receiver<bool>) {
    // An error means that the instance dropped the sender, which ends the
    // side as well.
    let _ = rx.wait_for(|a| *a).await;
}

async fn run_stream<O: Opener>(mut job: SideJob<'_>, mut opener: O) -> SideDone {
    let mut functions = std::mem::replace(
        &mut job.functions,
        Box::new(fps_flow::function::NoFunctions),
    );
    let upgrader = job.upgrader.take();
    let stop = aborted(job.abort.clone());
    let (result, stats, overlay) = {
        let mut session = Session::new(job.ctx(functions.as_mut()));
        session.set_spawn_queue(job.queue.clone());
        if let Some(clock) = job.clock.as_deref() {
            session.set_clock(clock);
        }
        if let Some(upgrader) = upgrader.as_deref() {
            session.set_tls_upgrader(upgrader);
        }
        let result = session.run_program(&mut opener, job.program, stop).await;
        (result, session.stats(), session.overlay().clone())
    };
    job.done(result, stats, overlay)
}

async fn run_datagram<O: DatagramOpener>(mut job: SideJob<'_>, mut opener: O) -> SideDone {
    let mut functions = std::mem::replace(
        &mut job.functions,
        Box::new(fps_flow::function::NoFunctions),
    );
    let stop = aborted(job.abort.clone());
    let (result, stats, overlay) = {
        let mut session = Session::new(job.ctx(functions.as_mut()));
        session.set_spawn_queue(job.queue.clone());
        if let Some(clock) = job.clock.as_deref() {
            session.set_clock(clock);
        }
        let result = session
            .run_datagram_program(&mut opener, job.program, stop)
            .await;
        (result, session.stats(), session.overlay().clone())
    };
    job.done(result, stats, overlay)
}

impl<'s> ScenarioInstance<'s> {
    /// Creates an instance of `scenario`, which [`Scenario::parse_instance`]
    /// compiled. The shared variables start as the scenario variables.
    pub fn new(scenario: &'s Scenario, config: InstanceConfig) -> Self {
        let n = scenario.flows.len();
        ScenarioInstance {
            scenario,
            config,
            index: scenario
                .flows
                .iter()
                .enumerate()
                .map(|(i, f)| (f.id.as_str(), i))
                .collect(),
            vars: scenario.vars.clone(),
            states: vec![FlowState::Waiting; n],
            running: vec![0; n],
            parent: vec![None; n],
            occurrences: HashMap::new(),
            ids: HashMap::new(),
            next_id: config.first_session_id,
            records: Vec::new(),
            failures: Vec::new(),
            error: None,
            aborting: false,
            clock: None,
        }
    }

    /// Makes `now()` in every side of the instance read `clock` instead of
    /// the wall clock of the host. The sides share the clock, so a clock with
    /// a step advances once per call over all of them.
    pub fn set_clock(&mut self, clock: Arc<dyn Clock>) {
        self.clock = Some(clock);
    }

    /// Returns the shared scenario variables.
    pub fn vars(&self) -> &Vars {
        &self.vars
    }

    /// Returns the state of the flow `id`.
    pub fn state(&self, id: &str) -> Option<FlowState> {
        self.index.get(id).map(|&i| self.states[i])
    }

    /// Returns every finished side in completion order.
    pub fn records(&self) -> &[SideRecord] {
        &self.records
    }

    /// Returns the failures of the sides that failed after the first one,
    /// mostly sides that the instance aborted. [`ScenarioInstance::run`]
    /// returns the first failure.
    pub fn later_failures(&self) -> &[Failure] {
        &self.failures
    }

    /// Runs the instance until every started side finished.
    ///
    /// When `stop` completes first, the instance aborts every running side,
    /// and the first aborted side fails the instance with reason `aborted`.
    /// A second call runs the scenario again with fresh state.
    pub async fn run<E: FlowEnv, F: Future<Output = ()>>(
        &mut self,
        env: &mut E,
        filler: &Filler,
        stop: F,
    ) -> Result<(), InstanceError> {
        self.reset();
        let queue = SpawnQueue::new();
        let (abort_tx, abort_rx) = watch::channel(false);
        let mut tasks: Vec<SideTask<'_>> = Vec::new();
        let starter = Starter {
            filler,
            queue: &queue,
            abort: &abort_rx,
        };
        self.schedule(env, &starter, &mut tasks);
        tokio::pin!(stop);
        let mut stopped = false;
        poll_fn(|cx| loop {
            if !stopped && stop.as_mut().poll(cx).is_ready() {
                stopped = true;
                self.aborting = true;
            }
            // Sending the abort wakes the sides, so we poll them again at once.
            let mut again = false;
            if self.aborting && !*abort_tx.borrow() {
                let _ = abort_tx.send(true);
                again = true;
            }
            let mut finished = Vec::new();
            tasks.retain_mut(|t| match t.as_mut().poll(cx) {
                Poll::Ready(done) => {
                    finished.push(done);
                    false
                }
                Poll::Pending => true,
            });
            let polled = tasks.len();
            again |= !finished.is_empty();
            for done in finished {
                self.finish(done);
            }
            for request in queue.drain() {
                self.spawn(env, &starter, &mut tasks, request);
            }
            self.schedule(env, &starter, &mut tasks);
            if tasks.is_empty() {
                return Poll::Ready(());
            }
            // New sides have never been polled, so nothing would wake us.
            if !again && tasks.len() == polled {
                return Poll::Pending;
            }
        })
        .await;
        for s in &mut self.states {
            if *s == FlowState::Waiting {
                *s = FlowState::Skipped;
            }
        }
        match self.error.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn reset(&mut self) {
        // The clock is a setting of the instance and survives a rerun.
        let clock = self.clock.take();
        let mut fresh = ScenarioInstance::new(self.scenario, self.config);
        fresh.clock = clock;
        *self = fresh;
    }

    /// Starts every waiting flow that is not spawned and whose `after` list
    /// completed, until no further flow starts.
    fn schedule<'a, E: FlowEnv>(
        &mut self,
        env: &mut E,
        starter: &Starter<'a>,
        tasks: &mut Vec<SideTask<'a>>,
    ) where
        's: 'a,
        E::Stream: 'a,
        E::Datagram: 'a,
    {
        if self.aborting {
            return;
        }
        let scenario = self.scenario;
        loop {
            let mut changed = false;
            for (i, flow) in scenario.flows.iter().enumerate() {
                if flow.spawned || self.states[i] != FlowState::Waiting {
                    continue;
                }
                let ready = flow.after.iter().all(|d| {
                    self.index
                        .get(d.as_str())
                        .is_some_and(|&j| self.states[j] == FlowState::Done)
                });
                if !ready {
                    continue;
                }
                changed = true;
                let vars = self.vars.clone();
                let mut started = false;
                for side in [Side::Client, Side::Server] {
                    if self.start(env, starter, tasks, i, side, vars.clone()) {
                        started = true;
                    }
                }
                // A flow without a side on this agent completes at once.
                self.states[i] = if started {
                    FlowState::Running
                } else {
                    FlowState::Done
                };
            }
            if !changed {
                return;
            }
        }
    }

    /// Starts one side of flow `i` when this instance runs that side and the
    /// flow has it. Returns true when the side started.
    fn start<'a, E: FlowEnv>(
        &mut self,
        env: &mut E,
        starter: &Starter<'a>,
        tasks: &mut Vec<SideTask<'a>>,
        i: usize,
        side: Side,
        vars: Vars,
    ) -> bool
    where
        's: 'a,
        E::Stream: 'a,
        E::Datagram: 'a,
    {
        let scenario: &'a Scenario = self.scenario;
        let flow = &scenario.flows[i];
        let program = match side {
            Side::Client => Some(&flow.client),
            Side::Server => flow.server.as_ref(),
        };
        let Some(program) = program.filter(|_| self.config.sides.contains(side)) else {
            return false;
        };
        let n = self.occurrences.entry((i, side)).or_insert(0);
        let occurrence = *n;
        *n += 1;
        let next = &mut self.next_id;
        let session_id = *self.ids.entry((i, occurrence)).or_insert_with(|| {
            let id = *next;
            *next = next.wrapping_add(1);
            id
        });
        let job = SideJob {
            flow_index: i,
            flow,
            program,
            side,
            session_id,
            vars,
            filler: starter.filler,
            functions: env.functions(flow, side),
            upgrader: match flow.l4 {
                L4::Tcp => env.tls_upgrader(flow, side),
                L4::Udp => None,
            },
            clock: self.clock.clone(),
            config: self.config,
            queue: starter.queue.clone(),
            abort: starter.abort.clone(),
        };
        let task: SideTask<'a> = match flow.l4 {
            L4::Tcp => Box::pin(run_stream(job, env.stream(flow, side))),
            L4::Udp => Box::pin(run_datagram(job, env.datagram(flow, side))),
        };
        tasks.push(task);
        self.running[i] += 1;
        true
    }

    /// Starts the side of a spawned flow that `request` names.
    fn spawn<'a, E: FlowEnv>(
        &mut self,
        env: &mut E,
        starter: &Starter<'a>,
        tasks: &mut Vec<SideTask<'a>>,
        request: SpawnRequest,
    ) where
        's: 'a,
        E::Stream: 'a,
        E::Datagram: 'a,
    {
        if self.aborting {
            return;
        }
        let parent = self.index.get(request.parent.as_str()).copied();
        let target = self
            .index
            .get(request.flow.as_str())
            .copied()
            .filter(|&i| self.scenario.flows[i].spawned);
        let Some(i) = target else {
            // Scenario::parse_instance rules this out, so this guards only
            // against a scenario that another loader built.
            let failure = Failure {
                flow: request.parent.clone(),
                side: request.side,
                action_index: 0,
                action_type: "spawn",
                reason: Reason::Internal,
                kind: Box::new(FailureKind::Invalid(format!(
                    "spawn of flow '{}', which does not declare spawned: true",
                    request.flow
                ))),
            };
            self.fail(parent, failure);
            return;
        };
        let mut vars = self.vars.clone();
        vars.extend(request.vars);
        self.parent[i] = parent;
        if self.start(env, starter, tasks, i, request.side, vars) && !self.is_failed(i) {
            self.states[i] = FlowState::Running;
        }
    }

    fn is_failed(&self, i: usize) -> bool {
        matches!(self.states[i], FlowState::Failed | FlowState::Aborted)
    }

    /// Records a finished side.
    fn finish(&mut self, done: SideDone) {
        let i = done.flow;
        self.running[i] = self.running[i].saturating_sub(1);
        self.records.push(SideRecord {
            flow: self.scenario.flows[i].id.clone(),
            side: done.side,
            session_id: done.session_id,
            stats: done.stats,
            failed: done.result.as_ref().err().map(|f| f.reason),
        });
        match done.result {
            Ok(()) => {
                self.vars.extend(done.overlay);
                if self.running[i] == 0 && !self.is_failed(i) {
                    self.states[i] = FlowState::Done;
                }
            }
            Err(failure) => self.fail(Some(i), failure),
        }
    }

    /// Records a failed side of flow `i`. The first failure fails the flow,
    /// its spawn chain, and the instance, and starts the abort.
    fn fail(&mut self, i: Option<usize>, failure: Failure) {
        if self.error.is_some() {
            if let Some(i) = i {
                if self.states[i] != FlowState::Failed {
                    self.states[i] = if failure.reason == Reason::Aborted {
                        FlowState::Aborted
                    } else {
                        FlowState::Failed
                    };
                }
            }
            self.failures.push(failure);
            return;
        }
        let mut spawned_by = Vec::new();
        if let Some(i) = i {
            self.states[i] = FlowState::Failed;
            let mut at = i;
            // A spawn chain visits each flow at most once.
            while let Some(p) = self.parent[at].filter(|_| spawned_by.len() < self.states.len()) {
                if !self.scenario.flows[at].spawned {
                    break;
                }
                self.states[p] = FlowState::Failed;
                spawned_by.push(self.scenario.flows[p].id.clone());
                at = p;
            }
        }
        self.error = Some(InstanceError {
            scenario: self.scenario.name.clone(),
            flow: failure.flow.clone(),
            spawned_by,
            failure: Box::new(failure),
        });
        self.aborting = true;
    }
}

/// The shared parts that every started side borrows.
struct Starter<'a> {
    filler: &'a Filler,
    queue: &'a SpawnQueue,
    abort: &'a watch::Receiver<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clippy's `result_large_err` rejects error types above 128 bytes.
    #[test]
    fn instance_error_stays_small() {
        assert!(std::mem::size_of::<InstanceError>() <= 128);
    }

    #[test]
    fn sides_select_their_side() {
        assert!(Sides::Both.contains(Side::Client) && Sides::Both.contains(Side::Server));
        assert!(Sides::Client.contains(Side::Client) && !Sides::Client.contains(Side::Server));
        assert!(Sides::Server.contains(Side::Server) && !Sides::Server.contains(Side::Client));
    }
}
