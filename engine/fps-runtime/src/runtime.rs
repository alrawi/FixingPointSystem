// SPDX-License-Identifier: GPL-3.0-only
//! Starting, stopping, and joining the worker threads.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use fps_capture::CaptureCell;
use fps_interp::Side;
use fps_sched::RateControl;
use fps_stats::Snapshot;
use tokio::sync::watch;

use crate::collector::{Collector, WorkerSnapshot};
use crate::config::RuntimeConfig;
use crate::control::Control;
use crate::worker::{run_thread, WorkerStart};
use crate::RuntimeError;

/// How one worker ended.
#[derive(Debug)]
pub struct WorkerExit {
    /// The worker id.
    pub worker: u32,
    /// The side that the worker ran.
    pub side: Side,
    /// `None` when the worker had no core, otherwise whether pinning worked.
    pub pinned: Option<bool>,
    /// The final snapshot of the worker shard.
    pub sessions: Snapshot,
}

/// The result of a run, which [`RuntimeHandle::join`] returns.
#[derive(Debug)]
pub struct RunReport {
    /// How each worker ended, by worker id.
    pub workers: BTreeMap<u32, Result<WorkerExit, RuntimeError>>,
    /// Every snapshot that the workers sent, including the final ones.
    pub collector: Collector,
}

/// Starts runtimes.
#[derive(Debug)]
pub struct Runtime;

impl Runtime {
    /// Validates `config`, starts one thread per worker, and waits until every
    /// worker is ready, which for a server worker means that its listeners
    /// are bound.
    ///
    /// When a worker fails to start, the call aborts the others, joins every
    /// thread, and returns the first error.
    pub fn start(config: RuntimeConfig) -> Result<RuntimeHandle, RuntimeError> {
        config.validate()?;
        let (control_tx, control_rx) = watch::channel(Control::Run);
        let (snap_tx, snap_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let mut threads = Vec::with_capacity(config.workers.len());
        let mut handle = RuntimeHandle {
            control: control_tx,
            threads: Vec::new(),
            snapshots: snap_rx,
            collector: Collector::new(),
            listen: BTreeMap::new(),
            rates: Arc::new(RateControl::new()),
            capture: config.capture.clone(),
        };
        let mut failure = None;
        for w in config.workers.iter().cloned() {
            let id = w.id;
            let start = WorkerStart {
                config: w,
                seed: config.seed,
                snapshot_interval: config.snapshot_interval,
                control: control_rx.clone(),
                snapshots: snap_tx.clone(),
                ready: ready_tx.clone(),
                capture: config.capture.clone(),
            };
            match std::thread::Builder::new()
                .name(format!("fps-worker-{id}"))
                .spawn(move || run_thread(start))
            {
                Ok(t) => threads.push((id, t)),
                Err(source) => {
                    failure = Some(RuntimeError::Thread { worker: id, source });
                    break;
                }
            }
        }
        drop(ready_tx);
        handle.threads = threads;
        for _ in 0..handle.threads.len() {
            match ready_rx.recv() {
                Ok((id, Ok(ready))) => {
                    handle.listen.insert(id, ready.addrs);
                    if let Some((entry, rate)) = ready.rate {
                        if let Err(source) = handle.rates.register(&entry, rate) {
                            failure.get_or_insert(RuntimeError::Schedule { worker: id, source });
                        }
                    }
                }
                Ok((_, Err(e))) => {
                    failure.get_or_insert(e);
                }
                Err(_) => {
                    failure.get_or_insert(RuntimeError::Lost { worker: u32::MAX });
                    break;
                }
            }
        }
        match failure {
            None => Ok(handle),
            Some(e) => {
                handle.abort();
                let _ = handle.join();
                Err(e)
            }
        }
    }
}

/// Controls a running set of workers.
///
/// Dropping the handle without [`RuntimeHandle::join`] aborts every worker
/// and detaches the threads.
#[derive(Debug)]
pub struct RuntimeHandle {
    control: watch::Sender<Control>,
    threads: Vec<(u32, JoinHandle<Result<WorkerExit, RuntimeError>>)>,
    snapshots: Receiver<WorkerSnapshot>,
    collector: Collector,
    listen: BTreeMap<u32, Vec<SocketAddr>>,
    rates: Arc<RateControl>,
    capture: CaptureCell,
}

impl RuntimeHandle {
    /// Returns the bound listen addresses of every server worker, in worker id
    /// order and in the order of each worker's configuration.
    pub fn listen_addrs(&self) -> Vec<SocketAddr> {
        self.listen.values().flatten().copied().collect()
    }

    /// Returns the registry of the rate handles of the client workers, by mix
    /// entry name, for the agent to drive with `UpdateRates`.
    ///
    /// Every client worker registered its handle before [`Runtime::start`]
    /// returned, under [`crate::FlowRef::entry`] or the scenario name. The registry
    /// outlives the handle, so a clone of the [`Arc`] stays valid after
    /// [`RuntimeHandle::join`], when it changes nothing. A worker reads its
    /// handle at least every 100 ms, also when its rate is 0 or its sessions
    /// sit at the cap, so a change takes effect within that time.
    pub fn rate_control(&self) -> Arc<RateControl> {
        Arc::clone(&self.rates)
    }

    /// Returns the cell that the workers read when a session opens, for the
    /// agent to start and stop a capture in the middle of the run.
    ///
    /// [`CaptureCell::set`] taps every TCP session that opens from then on,
    /// on a client worker and on a server worker, and [`CaptureCell::clear`]
    /// ends it. Sessions that are already open are not tapped. While the cell
    /// is empty a session pays one atomic load when it opens.
    pub fn capture_cell(&self) -> CaptureCell {
        self.capture.clone()
    }

    /// Stops new sessions and lets open sessions finish within `ramp_down`,
    /// after which the workers reset the sessions still open. A second stop
    /// can only shorten the ramp down.
    pub fn stop(&self, ramp_down: Duration) {
        self.control
            .send_modify(|c| *c = c.then(Control::Drain(ramp_down)));
    }

    /// Resets every open session at once and ends every worker.
    pub fn abort(&self) {
        self.control.send_modify(|c| *c = c.then(Control::Abort));
    }

    /// Takes the snapshots that arrived since the last call and returns the
    /// collector that holds them.
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

    /// Waits for every worker to end and returns the run report.
    ///
    /// Workers end when their schedules finish and their sessions close, or
    /// after a stop or an abort. A server worker ends only after a stop or an
    /// abort.
    pub fn join(mut self) -> RunReport {
        let mut workers = BTreeMap::new();
        for (id, t) in std::mem::take(&mut self.threads) {
            let r = t.join().unwrap_or(Err(RuntimeError::Lost { worker: id }));
            workers.insert(id, r);
        }
        self.collect();
        RunReport {
            workers,
            collector: std::mem::take(&mut self.collector),
        }
    }
}

impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        if !self.threads.is_empty() {
            self.abort();
        }
    }
}
