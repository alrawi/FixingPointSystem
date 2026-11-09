// SPDX-License-Identifier: GPL-3.0-only
//! The standalone run mode, `fps-agent run --plan FILE --modules DIR`.
//!
//! The mode runs a plan without a controller. It reads the plan, loads and
//! checks the modules, starts the server workers and then the client
//! workers, and writes one counter snapshot per snapshot interval to its
//! output as a JSON line. Each line is the proto3 JSON mapping of an
//! aggregate `CounterSnapshot` of every side that the process runs. Every
//! scenario and transport series carries the `side` label (CON-08), so the
//! client and the server series of one session sit side by side and do not
//! add up. The last line carries `isFinal`.
//!
//! The client workers end when their schedules end and their sessions
//! close. A process with server workers only stops them when the load
//! phases end plus the linger time. Server workers then drain for
//! [`SERVER_RAMP_DOWN`]. The criteria of the plan decide the [`Outcome`], and
//! they count the client side, or the server side in a process that runs no
//! client workers, see [`crate::criteria`]. Every error before or during the
//! run is a [`RunError`].

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use fps_interp::Side;
use fps_runtime::{Collector, RunReport, Runtime, RuntimeConfig, RuntimeError, RuntimeHandle};
use fps_sched::LoadProfile;
use fps_stats::{metric, Snapshot, SnapshotMeta, StatsError, Timestamp};

use crate::assemble::{assemble, load_profile, Sides};
use crate::control::{CaptureCell, Rates};
use crate::criteria::{evaluate_for, CriterionResult};
use crate::modules::{self, ModuleError};
use crate::plan::{Plan, PlanError};
use crate::trust::ModulePolicy;
use crate::udp::{UdpError, UdpHooks, UdpRuntime};

/// How long server workers let open sessions finish after the stop.
pub const SERVER_RAMP_DOWN: Duration = Duration::from_secs(2);

/// How long client sessions may stay open after the load phases end before
/// the agent aborts them.
pub const CLIENT_GRACE: Duration = Duration::from_secs(30);

/// The options of one run.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// The plan file.
    pub plan: PathBuf,
    /// The directory that holds one directory per module.
    pub modules: PathBuf,
    /// The sides that this process runs.
    pub sides: Sides,
    /// The interval between two snapshot lines.
    pub interval: Duration,
    /// How long a process with server workers only keeps them after the load
    /// phases end.
    pub linger: Duration,
    /// What the agent requires of the module signatures. The default asks for
    /// none.
    pub policy: ModulePolicy,
}

impl RunOptions {
    /// Returns options with both sides, one second snapshots, and a five
    /// second linger.
    pub fn new(plan: PathBuf, modules: PathBuf) -> Self {
        Self {
            plan,
            modules,
            sides: Sides::Both,
            interval: Duration::from_secs(1),
            linger: Duration::from_secs(5),
            policy: ModulePolicy::default(),
        }
    }
}

/// The verdict of a run that completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Every criterion holds.
    Pass,
    /// At least one criterion fails.
    Fail,
}

/// Errors that end a run without a verdict.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The plan file is unreadable or breaks a rule.
    #[error(transparent)]
    Plan(#[from] PlanError),
    /// A module or scenario failed its checks.
    #[error(transparent)]
    Module(#[from] ModuleError),
    /// The plan needs something that the standalone mode does not run.
    #[error("unimplemented: {0}")]
    Unimplemented(String),
    /// A TCP worker failed.
    #[error("runtime")]
    Runtime(#[from] RuntimeError),
    /// A UDP worker failed.
    #[error("udp runtime")]
    Udp(#[from] UdpError),
    /// A snapshot could not be merged or serialized.
    #[error("statistics")]
    Stats(#[from] StatsError),
    /// The output could not be written.
    #[error("write output")]
    Output(#[from] std::io::Error),
}

impl RunError {
    /// Returns a plan rule error.
    pub fn rule(message: String) -> Self {
        RunError::Plan(PlanError::Rule(message))
    }
}

/// What a finished run reports.
#[derive(Debug)]
pub struct RunSummary {
    /// The verdict.
    pub outcome: Outcome,
    /// The outcome of each criterion.
    pub criteria: Vec<CriterionResult>,
    /// The final client side totals, when the process ran client workers.
    pub client: Option<Snapshot>,
    /// The final server side totals, when the process ran server workers.
    pub server: Option<Snapshot>,
    /// The number of snapshot lines written, including the final one.
    pub lines: u64,
}

/// The TCP and UDP workers of one side.
struct SideRun {
    tcp: Option<RuntimeHandle>,
    udp: Option<UdpRuntime>,
}

impl SideRun {
    fn start(
        seed: u64,
        interval: Duration,
        tcp: Vec<fps_runtime::WorkerConfig>,
        udp: Vec<fps_runtime::WorkerConfig>,
        hooks: UdpHooks,
        tcp_capture: CaptureCell,
    ) -> Result<Self, RunError> {
        let tcp = if tcp.is_empty() {
            None
        } else {
            let mut cfg = RuntimeConfig::new(seed, tcp);
            cfg.snapshot_interval = interval;
            cfg.capture = tcp_capture;
            Some(Runtime::start(cfg)?)
        };
        let udp = if udp.is_empty() {
            None
        } else {
            Some(UdpRuntime::start_with(seed, interval, udp, hooks)?)
        };
        Ok(Self { tcp, udp })
    }

    fn is_empty(&self) -> bool {
        self.tcp.is_none() && self.udp.is_none()
    }

    fn is_finished(&self) -> bool {
        self.tcp.as_ref().is_none_or(RuntimeHandle::is_finished)
            && self.udp.as_ref().is_none_or(UdpRuntime::is_finished)
    }

    fn stop(&self, ramp_down: Duration) {
        if let Some(t) = &self.tcp {
            t.stop(ramp_down);
        }
        if let Some(u) = &self.udp {
            u.stop(ramp_down);
        }
    }

    fn total(&mut self, side: Side) -> Result<Snapshot, StatsError> {
        let mut out = Snapshot::aggregate();
        if let Some(t) = &mut self.tcp {
            out.merge(&t.collect().total(side)?)?;
        }
        if let Some(u) = &mut self.udp {
            out.merge(&u.collect().total(side)?)?;
        }
        Ok(out)
    }

    /// Waits for every worker and returns the final totals of `side`.
    fn join(self, side: Side) -> Result<Snapshot, RunError> {
        let mut out = Snapshot::aggregate();
        if let Some(t) = self.tcp {
            let RunReport { workers, collector } = t.join();
            for (_, r) in workers {
                r?;
            }
            out.merge(&collector.total(side)?)?;
        }
        if let Some(u) = self.udp {
            let (collector, workers): (Collector, _) = u.join();
            for (_, r) in workers {
                r?;
            }
            out.merge(&collector.total(side)?)?;
        }
        Ok(out)
    }
}

/// Returns the side that the `side` label names for a side of the engine.
fn stats_side(side: Side) -> fps_stats::Side {
    match side {
        Side::Client => fps_stats::Side::Client,
        Side::Server => fps_stats::Side::Server,
    }
}

/// Merges the totals of the two sides into one snapshot. A side that the
/// process does not run has an empty total. The `side` label keeps the series
/// of the sides apart.
fn both_sides(client: &Snapshot, server: &Snapshot) -> Result<Snapshot, StatsError> {
    let mut out = Snapshot::aggregate();
    out.merge(client)?;
    out.merge(server)?;
    Ok(out)
}

/// A plan that passed its checks, with its modules loaded and its workers
/// assembled, which waits for [`Prepared::start`].
pub struct Prepared {
    plan: Plan,
    assembly: crate::assemble::Assembly,
    profile: LoadProfile,
    reports: Side,
    notes: Vec<String>,
    warnings: Vec<String>,
    summary: String,
}

impl std::fmt::Debug for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prepared")
            .field("plan_id", &self.plan.plan_id)
            .field("summary", &self.summary)
            .finish_non_exhaustive()
    }
}

/// Checks `plan`, loads its modules from `modules_dir` under `policy`, and
/// assembles the workers of `sides`.
pub fn prepare(
    plan: Plan,
    modules_dir: &Path,
    sides: Sides,
    policy: &ModulePolicy,
) -> Result<Prepared, RunError> {
    plan.check()?;
    let loaded = modules::load(&plan, modules_dir, policy)?;
    let assembly = assemble(&plan, &loaded, sides)?;
    let profile = load_profile(&plan)?;
    let reports = if assembly.has_clients() {
        Side::Client
    } else {
        Side::Server
    };
    let summary = format!(
        "plan {} with {} tcp and {} udp client workers and {} tcp and {} udp server workers, native functions [{}], load {:?}",
        plan.plan_id,
        assembly.tcp_clients.len(),
        assembly.udp_clients.len(),
        assembly.tcp_servers.len(),
        assembly.udp_servers.len(),
        loaded.functions.modules().join(", "),
        plan.load_duration()
    );
    Ok(Prepared {
        plan,
        assembly,
        profile,
        reports,
        notes: loaded.notes,
        warnings: loaded.warnings,
        summary,
    })
}

impl Prepared {
    /// Returns the plan.
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// Returns the warnings of module loading: the modules that the agent
    /// accepted without a valid signature, and the missing trust store.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Returns the notes of module loading, such as a module that runs
    /// without native functions.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// Returns one line that describes the workers and the load.
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Starts the server workers and then the client workers, which take
    /// cumulative snapshots every `interval`.
    pub fn start(self, interval: Duration) -> Result<Active, RunError> {
        self.start_with(interval, CaptureCell::new())
    }

    /// Like [`Prepared::start`], and the TCP and UDP workers tap the sessions
    /// that begin while `capture` holds a capture.
    pub fn start_with(self, interval: Duration, capture: CaptureCell) -> Result<Active, RunError> {
        let Prepared {
            plan,
            assembly,
            profile,
            reports,
            ..
        } = self;
        // A client session holds both directions of its conversation, so a
        // process that runs clients taps those and not its server sessions.
        let tap_servers = !assembly.has_clients();
        // A process with clients and servers would record each connection
        // twice, so its server runtime gets a cell that stays empty.
        let server_cell = if tap_servers {
            capture.clone()
        } else {
            CaptureCell::new()
        };
        let client_cell = capture.clone();
        let hooks = UdpHooks {
            capture,
            tap_servers,
        };
        let servers = SideRun::start(
            plan.seed,
            interval,
            assembly.tcp_servers,
            assembly.udp_servers,
            hooks.clone(),
            server_cell,
        )?;
        let clients = match SideRun::start(
            plan.seed,
            interval,
            assembly.tcp_clients,
            assembly.udp_clients,
            hooks,
            client_cell,
        ) {
            Ok(c) => c,
            Err(e) => {
                servers.stop(Duration::ZERO);
                let _ = servers.join(Side::Server);
                return Err(e);
            }
        };
        Ok(Active {
            plan,
            profile,
            reports,
            servers,
            clients,
            start: Instant::now(),
            baseline: None,
            reset: false,
        })
    }
}

/// A started plan.
///
/// The caller samples it with [`Active::sample`], asks [`Active::is_done`]
/// whether the load ended, and calls [`Active::finish`] once to collect the
/// final totals. [`Active::stop`] ends the run early.
pub struct Active {
    plan: Plan,
    profile: LoadProfile,
    reports: Side,
    servers: SideRun,
    clients: SideRun,
    start: Instant,
    baseline: Option<Snapshot>,
    reset: bool,
}

impl std::fmt::Debug for Active {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Active")
            .field("plan_id", &self.plan.plan_id)
            .field("elapsed", &self.start.elapsed())
            .finish_non_exhaustive()
    }
}

/// The totals of a finished run.
#[derive(Debug)]
pub struct Finished {
    /// The final totals of every side that the process ran, in one snapshot
    /// whose series carry the `side` label.
    pub shown: Snapshot,
    /// The side that the criteria count by default, which is the client side
    /// unless the process ran no client workers.
    pub criteria_side: fps_stats::Side,
    /// The final client side totals, when the process ran client workers.
    pub client: Option<Snapshot>,
    /// The final server side totals, when the process ran server workers.
    pub server: Option<Snapshot>,
    /// The totals of every side at the end of the warmup.
    pub baseline: Option<Snapshot>,
    /// The time from the start to the end of the run.
    pub elapsed: Duration,
}

impl Active {
    /// Returns the plan.
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// Returns the rate registries of the client workers, which
    /// `UpdateRates` drives. A mix entry that no client worker of this process
    /// serves is not in them.
    pub fn rates(&self) -> Rates {
        Rates::new(
            self.clients.tcp.as_ref().map(RuntimeHandle::rate_control),
            self.clients.udp.as_ref().map(UdpRuntime::rate_control),
            self.plan.mix.iter().map(|e| e.id.clone()).collect(),
        )
    }

    /// Returns the time since the start.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Returns the name of the load phase at `elapsed`, or an empty string
    /// after the last phase.
    pub fn phase_at(&self, elapsed: Duration) -> String {
        self.profile
            .phase_at(elapsed.as_secs_f64())
            .map(|p| p.name.clone())
            .unwrap_or_default()
    }

    /// Returns true when the run ended on its own. Client workers end when
    /// their schedules end and their sessions close. A process with server
    /// workers only ends `linger` after the load phases.
    pub fn is_done(&self, linger: Duration) -> bool {
        if self.clients.is_empty() {
            self.start.elapsed() >= self.plan.load_duration() + linger
        } else {
            self.clients.is_finished()
        }
    }

    /// Returns true when every client worker ended, which is at once for a
    /// process without client workers.
    pub fn clients_finished(&self) -> bool {
        self.clients.is_finished()
    }

    /// Resets client sessions that stay open [`CLIENT_GRACE`] after the load
    /// phases, and returns a message the first time it does.
    pub fn enforce_grace(&mut self) -> Option<String> {
        if self.clients.is_empty()
            || self.reset
            || self.start.elapsed() < self.plan.load_duration() + CLIENT_GRACE
        {
            return None;
        }
        self.clients.stop(Duration::ZERO);
        self.reset = true;
        Some(format!(
            "client sessions still open {CLIENT_GRACE:?} after the load phases, resetting them"
        ))
    }

    /// Returns the cumulative totals of every side that the process runs, in
    /// one snapshot, and records the warmup baseline once the warmup has
    /// passed.
    pub fn sample(&mut self) -> Result<Snapshot, RunError> {
        let elapsed = self.start.elapsed();
        let c = self.clients.total(Side::Client)?;
        let s = self.servers.total(Side::Server)?;
        let both = both_sides(&c, &s)?;
        if self.baseline.is_none() && !self.plan.warmup.is_zero() && elapsed >= self.plan.warmup {
            self.baseline = Some(both.clone());
        }
        Ok(both)
    }

    /// Returns the side that the criteria count by default.
    pub fn criteria_side(&self) -> fps_stats::Side {
        stats_side(self.reports)
    }

    /// Stops new client sessions and lets open sessions finish within
    /// `ramp_down`. [`Active::finish`] then stops the servers.
    pub fn stop(&self, ramp_down: Duration) {
        self.clients.stop(ramp_down);
        if ramp_down.is_zero() {
            self.servers.stop(Duration::ZERO);
        }
    }

    /// Waits for the client workers, stops the server workers with
    /// `server_ramp_down`, waits for them, and returns the final totals.
    pub fn finish(self, server_ramp_down: Duration) -> Result<Finished, RunError> {
        let Active {
            reports,
            servers,
            clients,
            start,
            baseline,
            ..
        } = self;
        let has_clients = !clients.is_empty();
        let has_servers = !servers.is_empty();
        let client_total = clients.join(Side::Client);
        servers.stop(server_ramp_down);
        let server_total = servers.join(Side::Server);
        let (client_total, server_total) = (client_total?, server_total?);
        let shown = both_sides(&client_total, &server_total)?;
        Ok(Finished {
            shown,
            criteria_side: stats_side(reports),
            client: has_clients.then_some(client_total),
            server: has_servers.then_some(server_total),
            baseline,
            elapsed: start.elapsed(),
        })
    }

    /// Resets every session at once and waits for every worker, for an
    /// error path that discards the totals.
    pub fn abort(self) {
        self.stop(Duration::ZERO);
        let _ = self.finish(Duration::ZERO);
    }
}

/// Writes snapshot lines with increasing sequence numbers.
struct Printer<'a> {
    out: &'a mut dyn Write,
    plan_id: String,
    sequence: u64,
}

impl Printer<'_> {
    fn line(&mut self, snap: &Snapshot, phase: String, is_final: bool) -> Result<(), RunError> {
        self.sequence += 1;
        let meta = SnapshotMeta {
            plan_id: self.plan_id.clone(),
            sequence: self.sequence,
            time: Timestamp::from_system_time(SystemTime::now()),
            phase,
            is_final,
        };
        let text = snap.to_proto_json_string(&meta)?;
        writeln!(self.out, "{text}")?;
        self.out.flush()?;
        Ok(())
    }
}

/// Runs the plan of `opts`, writes snapshot lines to `out` and operator
/// messages to `log`, and returns the summary.
pub fn run(
    opts: &RunOptions,
    out: &mut dyn Write,
    log: &mut dyn Write,
) -> Result<RunSummary, RunError> {
    let plan = Plan::load(&opts.plan)?;
    let prepared = prepare(plan, &opts.modules, opts.sides, &opts.policy)?;
    for warning in prepared.warnings() {
        writeln!(log, "fps-agent: warning: {warning}")?;
    }
    for note in prepared.notes() {
        writeln!(log, "fps-agent: {note}")?;
    }
    writeln!(log, "fps-agent: {}", prepared.summary())?;
    let criteria_plan = prepared.plan().criteria.clone();
    let mut active = prepared.start(opts.interval)?;
    let mut printer = Printer {
        out,
        plan_id: active.plan().plan_id.clone(),
        sequence: 0,
    };
    let mut next = Instant::now() + opts.interval;
    let result = (|| -> Result<(), RunError> {
        loop {
            if active.is_done(opts.linger) {
                return Ok(());
            }
            if let Some(message) = active.enforce_grace() {
                writeln!(log, "fps-agent: {message}")?;
            }
            let now = Instant::now();
            if now < next {
                std::thread::sleep((next - now).min(Duration::from_millis(50)));
                continue;
            }
            next += opts.interval;
            let elapsed = active.elapsed();
            let shown = active.sample()?;
            printer.line(&shown, active.phase_at(elapsed), false)?;
        }
    })();
    if let Err(e) = result {
        active.abort();
        return Err(e);
    }
    let phase_end = active.phase_at(active.elapsed());
    let finished = active.finish(SERVER_RAMP_DOWN)?;
    printer.line(&finished.shown, phase_end, true)?;
    let lines = printer.sequence;

    let criteria = evaluate_for(
        &criteria_plan,
        &finished.shown,
        finished.baseline.as_ref(),
        finished.criteria_side,
    )?;
    for side in [("client", &finished.client), ("server", &finished.server)] {
        if let (name, Some(total)) = side {
            writeln!(
                log,
                "fps-agent: {} sessions attempted {} succeeded {} failed {}",
                name,
                total.counter_total(metric::SESSIONS_ATTEMPTED),
                total.counter_total(metric::SESSIONS_SUCCEEDED),
                total.counter_total(metric::SESSIONS_FAILED)
            )?;
        }
    }
    for c in &criteria {
        writeln!(
            log,
            "fps-agent: criterion {} {} {} measured {} {}",
            c.metric,
            c.op.symbol(),
            c.threshold,
            c.value,
            if c.holds { "holds" } else { "fails" }
        )?;
    }
    let outcome = if criteria.iter().all(|c| c.holds) {
        Outcome::Pass
    } else {
        Outcome::Fail
    };
    writeln!(
        log,
        "fps-agent: verdict {}",
        match outcome {
            Outcome::Pass => "pass",
            Outcome::Fail => "fail",
        }
    )?;
    Ok(RunSummary {
        outcome,
        criteria,
        client: finished.client,
        server: finished.server,
        lines,
    })
}
