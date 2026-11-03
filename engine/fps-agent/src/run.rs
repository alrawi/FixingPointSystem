// SPDX-License-Identifier: GPL-3.0-only
//! The standalone run mode, `fps-agent run --plan FILE --modules DIR`.
//!
//! The mode runs a plan without a controller. It reads the plan, loads and
//! checks the modules, starts the server workers and then the client
//! workers, and writes one counter snapshot per snapshot interval to its
//! output as a JSON line. Each line is the proto3 JSON mapping of an
//! aggregate `CounterSnapshot` of the side that the process reports, which
//! is the client side when the process runs client workers and the server
//! side otherwise, because the catalog has no side label and the two sides
//! of one session must not add up. The last line carries `isFinal`.
//!
//! The client workers end when their schedules end and their sessions
//! close. A process with server workers only stops them when the load
//! phases end plus the linger time. Server workers then drain for
//! [`SERVER_RAMP_DOWN`]. The criteria of the plan decide the [`Outcome`],
//! and every error before or during the run is a [`RunError`].

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use fps_interp::Side;
use fps_runtime::{Collector, RunReport, Runtime, RuntimeConfig, RuntimeError, RuntimeHandle};
use fps_sched::LoadProfile;
use fps_stats::{metric, Snapshot, SnapshotMeta, StatsError, Timestamp};

use crate::assemble::{assemble, load_profile, Sides};
use crate::criteria::{evaluate, CriterionResult};
use crate::modules::{self, ModuleError};
use crate::plan::{Plan, PlanError};
use crate::udp::{UdpError, UdpRuntime};

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
    ) -> Result<Self, RunError> {
        let tcp = if tcp.is_empty() {
            None
        } else {
            let mut cfg = RuntimeConfig::new(seed, tcp);
            cfg.snapshot_interval = interval;
            Some(Runtime::start(cfg)?)
        };
        let udp = if udp.is_empty() {
            None
        } else {
            Some(UdpRuntime::start(seed, interval, udp)?)
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

/// Writes snapshot lines with increasing sequence numbers.
struct Printer<'a> {
    out: &'a mut dyn Write,
    plan_id: String,
    profile: LoadProfile,
    sequence: u64,
}

impl Printer<'_> {
    fn line(&mut self, snap: &Snapshot, elapsed: Duration, is_final: bool) -> Result<(), RunError> {
        self.sequence += 1;
        let phase = self
            .profile
            .phase_at(elapsed.as_secs_f64())
            .map(|p| p.name.clone())
            .unwrap_or_default();
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
    let loaded = modules::load(&plan, &opts.modules)?;
    for note in &loaded.notes {
        writeln!(log, "fps-agent: {note}")?;
    }
    let assembly = assemble(&plan, &loaded, opts.sides)?;
    let profile = load_profile(&plan)?;
    let load = plan.load_duration();
    let reports = if assembly.has_clients() {
        Side::Client
    } else {
        Side::Server
    };
    writeln!(
        log,
        "fps-agent: plan {} with {} tcp and {} udp client workers and {} tcp and {} udp server workers, native functions [{}], load {:?}",
        plan.plan_id,
        assembly.tcp_clients.len(),
        assembly.udp_clients.len(),
        assembly.tcp_servers.len(),
        assembly.udp_servers.len(),
        loaded.functions.modules().join(", "),
        load
    )?;

    let mut servers = SideRun::start(
        plan.seed,
        opts.interval,
        assembly.tcp_servers,
        assembly.udp_servers,
    )?;
    let mut clients = match SideRun::start(
        plan.seed,
        opts.interval,
        assembly.tcp_clients,
        assembly.udp_clients,
    ) {
        Ok(c) => c,
        Err(e) => {
            servers.stop(Duration::ZERO);
            let _ = servers.join(Side::Server);
            return Err(e);
        }
    };
    let start = Instant::now();
    let mut printer = Printer {
        out,
        plan_id: plan.plan_id.clone(),
        profile,
        sequence: 0,
    };
    let mut baseline: Option<(Snapshot, Snapshot)> = None;
    let mut next = start + opts.interval;
    let mut aborted = false;
    let result = (|| -> Result<(), RunError> {
        loop {
            let done = if clients.is_empty() {
                start.elapsed() >= load + opts.linger
            } else {
                clients.is_finished()
            };
            if done {
                return Ok(());
            }
            if !clients.is_empty() && !aborted && start.elapsed() >= load + CLIENT_GRACE {
                writeln!(
                    log,
                    "fps-agent: client sessions still open {CLIENT_GRACE:?} after the load phases, resetting them"
                )?;
                clients.stop(Duration::ZERO);
                aborted = true;
            }
            let now = Instant::now();
            if now < next {
                std::thread::sleep((next - now).min(Duration::from_millis(50)));
                continue;
            }
            next += opts.interval;
            let elapsed = start.elapsed();
            let c = clients.total(Side::Client)?;
            let s = servers.total(Side::Server)?;
            if baseline.is_none() && !plan.warmup.is_zero() && elapsed >= plan.warmup {
                baseline = Some((c.clone(), s.clone()));
            }
            let shown = if reports == Side::Client { &c } else { &s };
            printer.line(shown, elapsed, false)?;
        }
    })();
    if let Err(e) = result {
        clients.stop(Duration::ZERO);
        servers.stop(Duration::ZERO);
        let _ = clients.join(Side::Client);
        let _ = servers.join(Side::Server);
        return Err(e);
    }
    let has_clients = !clients.is_empty();
    let has_servers = !servers.is_empty();
    let client_total = clients.join(Side::Client);
    servers.stop(SERVER_RAMP_DOWN);
    let server_total = servers.join(Side::Server);
    let (client_total, server_total) = (client_total?, server_total?);
    let shown = if reports == Side::Client {
        &client_total
    } else {
        &server_total
    };
    printer.line(shown, start.elapsed(), true)?;
    let lines = printer.sequence;

    let base = baseline.as_ref().map(|(c, s)| match reports {
        Side::Client => c,
        Side::Server => s,
    });
    let criteria = evaluate(&plan.criteria, shown, base)?;
    for side in [
        (has_clients, "client", &client_total),
        (has_servers, "server", &server_total),
    ] {
        if side.0 {
            writeln!(
                log,
                "fps-agent: {} sessions attempted {} succeeded {} failed {}",
                side.1,
                side.2.counter_total(metric::SESSIONS_ATTEMPTED),
                side.2.counter_total(metric::SESSIONS_SUCCEEDED),
                side.2.counter_total(metric::SESSIONS_FAILED)
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
        client: has_clients.then_some(client_total),
        server: has_servers.then_some(server_total),
        lines,
    })
}
