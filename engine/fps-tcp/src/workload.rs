// SPDX-License-Identifier: GPL-3.0-only
//! The three workloads of the harness.
//!
//! Every workload runs against any [`TcpStack`] and touches sessions only
//! through [`SessionIo`], so the numbers include the cost of the adapter. The
//! workloads read the wall clock, because they measure it, and they belong to
//! the edge of the process, not to the logic of a stack.
//!
//! * [`session_rate`] opens sessions one after the other on `concurrency`
//!   tasks, sends a request, reads a response, and closes both directions.
//! * [`throughput`] moves bulk data over concurrent sessions.
//! * [`socket_scaling`] grows the number of held sockets in steps and records,
//!   per step, how long the growth took, the memory per socket, the cost of
//!   one poll with idle sockets, and the session rate that remains.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fps_io::SessionIo;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::{JoinHandle, JoinSet};

use crate::stack::{StackStats, TcpStack};
use crate::sysinfo;

/// The most sessions that share one server port in the scale workload.
///
/// A client host has about 28,000 source ports for one server address and
/// port, which is the default range of Linux, and a user space stack has up to
/// 55,000. Staying below the smaller range keeps every 4-tuple unique on every
/// stack.
const SESSIONS_PER_PORT: usize = 20_000;

/// Errors that end a workload.
#[derive(Debug, thiserror::Error)]
pub enum BenchError {
    /// A stack operation failed.
    #[error("{what}")]
    Io {
        /// What the workload tried to do.
        what: String,
        /// The failure of the stack.
        #[source]
        source: io::Error,
    },
    /// A task of the workload ended abnormally.
    #[error("workload task failed: {0}")]
    Task(String),
    /// The workload made no progress within its limit.
    #[error("{0}")]
    Stalled(String),
    /// A session delivered a different number of bytes than the workload sent.
    #[error("{what}: expected {expected} bytes, got {actual}")]
    ByteCount {
        /// The transfer that came up short.
        what: String,
        /// The bytes sent.
        expected: u64,
        /// The bytes received.
        actual: u64,
    },
}

fn io_error(what: impl Into<String>) -> impl FnOnce(io::Error) -> BenchError {
    let what = what.into();
    move |source| BenchError::Io { what, source }
}

fn task_error(error: tokio::task::JoinError) -> BenchError {
    BenchError::Task(error.to_string())
}

/// Spawns the driver of a stack, which the workloads abort when they end.
fn spawn_driver<S: TcpStack>(stack: &S) -> JoinHandle<()> {
    let driver = stack.clone();
    tokio::spawn(async move { driver.drive().await })
}

/// The percentiles of a set of latencies in microseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LatencySummary {
    /// The median.
    pub p50_us: u64,
    /// The 99th percentile.
    pub p99_us: u64,
    /// The largest value.
    pub max_us: u64,
}

impl LatencySummary {
    /// Summarizes `samples`, which this call sorts.
    pub fn from_samples(samples: &mut [u32]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        samples.sort_unstable();
        let at = |fraction: f64| {
            let index = ((samples.len() as f64) * fraction).ceil() as usize;
            u64::from(samples[index.saturating_sub(1).min(samples.len() - 1)])
        };
        Self {
            p50_us: at(0.50),
            p99_us: at(0.99),
            max_us: u64::from(samples[samples.len() - 1]),
        }
    }
}

fn micros(elapsed: Duration) -> u32 {
    u32::try_from(elapsed.as_micros()).unwrap_or(u32::MAX)
}

/// Settings of the session rate workload.
#[derive(Clone, Debug)]
pub struct RateConfig {
    /// How long clients keep opening sessions.
    pub duration: Duration,
    /// The sessions in flight at any moment.
    pub concurrency: usize,
    /// The bytes of the request that the client sends.
    pub request_bytes: usize,
    /// The bytes of the response that the server sends.
    pub response_bytes: usize,
    /// The server port.
    pub port: u16,
}

impl Default for RateConfig {
    fn default() -> Self {
        Self {
            duration: Duration::from_secs(5),
            concurrency: 64,
            request_bytes: 64,
            response_bytes: 64,
            port: 7001,
        }
    }
}

/// The raw outcome of one run of the session rate workload.
#[derive(Clone, Debug)]
pub struct RateRun {
    /// Sessions that completed, including the close of both directions.
    pub sessions: u64,
    /// Sessions that failed.
    pub errors: u64,
    /// The time from the first connect to the last close.
    pub elapsed: Duration,
    /// The time of every completed session in microseconds.
    pub samples: Vec<u32>,
    /// The counters of the stack at the end of the run.
    pub stats: StackStats,
}

/// The result of the session rate workload, over one or more runs.
#[derive(Clone, Debug, PartialEq)]
pub struct RateReport {
    /// Sessions that completed.
    pub sessions: u64,
    /// Sessions that failed.
    pub errors: u64,
    /// The longest run in seconds.
    pub elapsed_secs: f64,
    /// Completed sessions per second over all runs.
    pub sessions_per_s: f64,
    /// The time of one session from connect to close.
    pub latency: LatencySummary,
}

impl RateReport {
    /// Combines runs that executed at the same time, one per thread.
    pub fn from_runs(runs: Vec<RateRun>) -> Self {
        let sessions = runs.iter().map(|r| r.sessions).sum();
        let errors = runs.iter().map(|r| r.errors).sum();
        let elapsed = runs.iter().map(|r| r.elapsed).max().unwrap_or_default();
        let mut samples: Vec<u32> = runs.into_iter().flat_map(|r| r.samples).collect();
        let elapsed_secs = elapsed.as_secs_f64();
        Self {
            sessions,
            errors,
            elapsed_secs,
            sessions_per_s: if elapsed_secs > 0.0 {
                sessions as f64 / elapsed_secs
            } else {
                0.0
            },
            latency: LatencySummary::from_samples(&mut samples),
        }
    }
}

/// Serves requests of `request_bytes` with `response_bytes` on `port`, until
/// the task is aborted, and counts the sessions that failed.
fn serve_requests<S: TcpStack>(
    stack: &S,
    port: u16,
    request_bytes: usize,
    response_bytes: usize,
    failures: Arc<AtomicU64>,
) -> JoinHandle<()> {
    let stack = stack.clone();
    tokio::spawn(async move {
        loop {
            let Ok(mut session) = stack.accept(port).await else {
                failures.fetch_add(1, Ordering::Relaxed);
                return;
            };
            let failures = Arc::clone(&failures);
            tokio::spawn(async move {
                if answer(&mut session, request_bytes, response_bytes)
                    .await
                    .is_err()
                {
                    failures.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    })
}

/// The server half of one session: read the request, send the response, wait
/// for the end of the client, and close.
async fn answer<T: SessionIo>(
    session: &mut T,
    request_bytes: usize,
    response_bytes: usize,
) -> io::Result<()> {
    let mut buffer = vec![0u8; request_bytes.max(response_bytes).max(1)];
    session.read_exact(&mut buffer[..request_bytes]).await?;
    session.write_all(&buffer[..response_bytes]).await?;
    while session.read(&mut buffer).await? != 0 {}
    session.shutdown().await
}

/// The client half of one session: connect, send, read, close, and wait for
/// the end of the server.
async fn request<S: TcpStack>(
    stack: &S,
    port: u16,
    buffer: &mut [u8],
    request_bytes: usize,
    response_bytes: usize,
) -> io::Result<()> {
    let mut session = stack.connect(port).await?;
    session.write_all(&buffer[..request_bytes]).await?;
    session.read_exact(&mut buffer[..response_bytes]).await?;
    session.shutdown().await?;
    while session.read(buffer).await? != 0 {}
    Ok(())
}

/// Runs the session rate workload on one stack and returns the raw run.
pub async fn session_rate<S: TcpStack>(
    stack: &S,
    config: &RateConfig,
) -> Result<RateRun, BenchError> {
    stack.listen(config.port).map_err(io_error("listen"))?;
    let driver = spawn_driver(stack);
    let failures = Arc::new(AtomicU64::new(0));
    let server = serve_requests(
        stack,
        config.port,
        config.request_bytes,
        config.response_bytes,
        Arc::clone(&failures),
    );
    let started = Instant::now();
    let deadline = started + config.duration;
    let mut clients = JoinSet::new();
    for _ in 0..config.concurrency.max(1) {
        let stack = stack.clone();
        let config = config.clone();
        clients.spawn(async move {
            let mut buffer = vec![0u8; config.request_bytes.max(config.response_bytes).max(1)];
            let mut samples = Vec::new();
            let mut errors = 0u64;
            while Instant::now() < deadline {
                let begin = Instant::now();
                match request(
                    &stack,
                    config.port,
                    &mut buffer,
                    config.request_bytes,
                    config.response_bytes,
                )
                .await
                {
                    Ok(()) => samples.push(micros(begin.elapsed())),
                    Err(_) => {
                        errors += 1;
                        break;
                    }
                }
            }
            (samples, errors)
        });
    }
    let limit = config.duration + Duration::from_secs(60);
    let joined = tokio::time::timeout(limit, async {
        let mut samples = Vec::new();
        let mut errors = 0u64;
        while let Some(done) = clients.join_next().await {
            let (mut part, failed) = done.map_err(task_error)?;
            samples.append(&mut part);
            errors += failed;
        }
        Ok::<_, BenchError>((samples, errors))
    })
    .await;
    let elapsed = started.elapsed();
    let stats = stack.stats();
    server.abort();
    driver.abort();
    let (samples, errors) = joined
        .map_err(|_| BenchError::Stalled(format!("sessions did not finish within {limit:?}")))??;
    Ok(RateRun {
        sessions: samples.len() as u64,
        errors: errors + failures.load(Ordering::Relaxed),
        elapsed,
        samples,
        stats,
    })
}

/// The direction of the bulk data of the throughput workload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// The client sends and the server receives.
    Upload,
    /// The server sends and the client receives.
    Download,
}

/// Settings of the throughput workload.
#[derive(Clone, Debug)]
pub struct ThroughputConfig {
    /// The concurrent sessions.
    pub sessions: usize,
    /// The bytes that each session moves.
    pub bytes_per_session: u64,
    /// The size of one read or write call in bytes.
    pub chunk: usize,
    /// Which side sends.
    pub direction: Direction,
    /// The server port.
    pub port: u16,
}

impl Default for ThroughputConfig {
    fn default() -> Self {
        Self {
            sessions: 4,
            bytes_per_session: 128 * 1024 * 1024,
            chunk: 64 * 1024,
            direction: Direction::Upload,
            port: 7002,
        }
    }
}

/// The result of one throughput run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThroughputRun {
    /// The bytes that the receiving side read.
    pub bytes: u64,
    /// The time from the first connect to the last close.
    pub elapsed: Duration,
    /// The counters of the stack at the end of the run.
    pub stats: StackStats,
}

/// The result of the throughput workload, over one or more runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThroughputReport {
    /// The bytes delivered over all runs.
    pub bytes: u64,
    /// The longest run in seconds.
    pub elapsed_secs: f64,
    /// Delivered application bits per second over all runs.
    pub gbit_per_s: f64,
}

impl ThroughputReport {
    /// Combines runs that executed at the same time, one per thread.
    pub fn from_runs(runs: &[ThroughputRun]) -> Self {
        let bytes: u64 = runs.iter().map(|r| r.bytes).sum();
        let elapsed = runs.iter().map(|r| r.elapsed).max().unwrap_or_default();
        let elapsed_secs = elapsed.as_secs_f64();
        Self {
            bytes,
            elapsed_secs,
            gbit_per_s: if elapsed_secs > 0.0 {
                bytes as f64 * 8.0 / elapsed_secs / 1e9
            } else {
                0.0
            },
        }
    }
}

/// Writes `total` bytes in chunks of `chunk` and closes the write side.
async fn send_bulk<T: SessionIo>(session: &mut T, total: u64, chunk: usize) -> io::Result<()> {
    let buffer = vec![0xA5u8; chunk.max(1)];
    let mut left = total;
    while left > 0 {
        let take = usize::try_from(left)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        session.write_all(&buffer[..take]).await?;
        left -= take as u64;
    }
    session.shutdown().await
}

/// Reads to the end of the stream and returns the byte count.
async fn drain<T: SessionIo>(session: &mut T, chunk: usize) -> io::Result<u64> {
    let mut buffer = vec![0u8; chunk.max(1)];
    let mut total = 0u64;
    loop {
        let read = session.read(&mut buffer).await?;
        if read == 0 {
            return Ok(total);
        }
        total += read as u64;
    }
}

/// Runs the throughput workload on one stack.
pub async fn throughput<S: TcpStack>(
    stack: &S,
    config: &ThroughputConfig,
) -> Result<ThroughputRun, BenchError> {
    stack.listen(config.port).map_err(io_error("listen"))?;
    let driver = spawn_driver(stack);
    let started = Instant::now();
    let mut tasks: JoinSet<Result<u64, BenchError>> = JoinSet::new();
    for _ in 0..config.sessions.max(1) {
        let (client_stack, server_stack) = (stack.clone(), stack.clone());
        let config = config.clone();
        let client_config = config.clone();
        tasks.spawn(async move {
            let mut session = client_stack
                .connect(client_config.port)
                .await
                .map_err(io_error("connect"))?;
            match client_config.direction {
                Direction::Upload => {
                    send_bulk(
                        &mut session,
                        client_config.bytes_per_session,
                        client_config.chunk,
                    )
                    .await
                    .map_err(io_error("send"))?;
                    drain(&mut session, 16).await.map_err(io_error("close"))?;
                    Ok(0)
                }
                Direction::Download => drain(&mut session, client_config.chunk)
                    .await
                    .map_err(io_error("receive")),
            }
        });
        tasks.spawn(async move {
            let mut session = server_stack
                .accept(config.port)
                .await
                .map_err(io_error("accept"))?;
            match config.direction {
                Direction::Upload => {
                    let received = drain(&mut session, config.chunk)
                        .await
                        .map_err(io_error("receive"))?;
                    session.shutdown().await.map_err(io_error("close"))?;
                    Ok(received)
                }
                Direction::Download => {
                    send_bulk(&mut session, config.bytes_per_session, config.chunk)
                        .await
                        .map_err(io_error("send"))?;
                    drain(&mut session, 16).await.map_err(io_error("close"))?;
                    Ok(0)
                }
            }
        });
    }
    let limit = Duration::from_secs(600);
    let joined = tokio::time::timeout(limit, async {
        let mut bytes = 0u64;
        while let Some(done) = tasks.join_next().await {
            bytes += done.map_err(task_error)??;
        }
        Ok::<_, BenchError>(bytes)
    })
    .await;
    let elapsed = started.elapsed();
    let stats = stack.stats();
    driver.abort();
    let bytes = joined.map_err(|_| {
        BenchError::Stalled(format!("the transfer did not finish within {limit:?}"))
    })??;
    let expected = config.bytes_per_session * config.sessions.max(1) as u64;
    if bytes != expected {
        return Err(BenchError::ByteCount {
            what: "throughput transfer".into(),
            expected,
            actual: bytes,
        });
    }
    Ok(ThroughputRun {
        bytes,
        elapsed,
        stats,
    })
}

/// Settings of the socket scaling workload.
#[derive(Clone, Debug)]
pub struct ScaleConfig {
    /// The socket counts to reach, in increasing order. A session has a
    /// client socket and a server socket, and both count.
    pub steps: Vec<usize>,
    /// The sessions that one shard opens at the same time while it grows.
    pub batch: usize,
    /// The time that one step may take before the workload stops.
    pub step_budget: Duration,
    /// The sessions of the rate probe after each step.
    pub probe_sessions: usize,
    /// The first server port of the held sessions.
    pub base_port: u16,
    /// The server port of the probe sessions.
    pub probe_port: u16,
}

impl Default for ScaleConfig {
    fn default() -> Self {
        Self {
            steps: vec![1_000, 10_000, 100_000],
            batch: 256,
            step_budget: Duration::from_secs(120),
            probe_sessions: 200,
            base_port: 9000,
            probe_port: 7003,
        }
    }
}

/// What one step of the socket scaling workload measured.
#[derive(Clone, Debug, PartialEq)]
pub struct ScaleStep {
    /// The socket count that the step aimed for.
    pub target_sockets: usize,
    /// The sockets that the stacks hold after the step, on both hosts.
    pub sockets: u64,
    /// True when the step reached its target within the budget.
    pub reached: bool,
    /// The time that the step took to open its sessions.
    pub setup_secs: f64,
    /// Sockets opened per second during the step.
    pub setup_sockets_per_s: f64,
    /// The resident memory of the process after the step, when it is known.
    pub rss_bytes: Option<u64>,
    /// The growth of the resident memory per socket since the workload began.
    pub bytes_per_socket: Option<f64>,
    /// The time of one full poll of every shard with idle sockets, in microseconds.
    pub idle_poll_us: f64,
    /// Probe sessions per second while the sockets are held.
    pub probe_sessions_per_s: f64,
    /// The probe latencies.
    pub probe_latency: LatencySummary,
}

/// The result of the socket scaling workload.
#[derive(Clone, Debug, PartialEq)]
pub struct ScaleReport {
    /// The shards, which are independent client and server pairs.
    pub shards: usize,
    /// One entry per step that ran, including the last one that stopped early.
    pub steps: Vec<ScaleStep>,
    /// Why the workload stopped before the last step, when it did.
    pub stopped: Option<String>,
}

impl ScaleReport {
    /// Returns the most sockets that a step reached.
    pub fn max_sockets(&self) -> u64 {
        self.steps
            .iter()
            .filter(|s| s.reached)
            .map(|s| s.sockets)
            .max()
            .unwrap_or(0)
    }
}

/// Opens `pairs` sessions on one shard and keeps both ends in `held`.
async fn grow_shard<S: TcpStack>(
    stack: &S,
    held: &mut Vec<S::Session>,
    opened: &mut usize,
    listened: &mut usize,
    pairs: usize,
    config: &ScaleConfig,
    deadline: Instant,
) -> Result<bool, BenchError> {
    let mut left = pairs;
    while left > 0 {
        if Instant::now() > deadline {
            return Ok(false);
        }
        let round = left.min(config.batch.max(1));
        let mut tasks: JoinSet<Result<S::Session, BenchError>> = JoinSet::new();
        for offset in 0..round {
            let port_index = (*opened + offset) / SESSIONS_PER_PORT;
            while *listened <= port_index {
                let port = config.base_port + u16::try_from(*listened).unwrap_or(u16::MAX);
                stack.listen(port).map_err(io_error("listen"))?;
                *listened += 1;
            }
            let port = config.base_port + u16::try_from(port_index).unwrap_or(u16::MAX);
            let (client, server) = (stack.clone(), stack.clone());
            tasks.spawn(async move { client.connect(port).await.map_err(io_error("connect")) });
            tasks.spawn(async move { server.accept(port).await.map_err(io_error("accept")) });
        }
        let limit = config.step_budget + Duration::from_secs(30);
        let joined = tokio::time::timeout(limit, async {
            while let Some(done) = tasks.join_next().await {
                held.push(done.map_err(task_error)??);
            }
            Ok::<_, BenchError>(())
        })
        .await;
        joined.map_err(|_| {
            BenchError::Stalled(format!(
                "{round} sessions did not open within {limit:?} with {} held",
                held.len()
            ))
        })??;
        *opened += round;
        left -= round;
    }
    Ok(true)
}

/// Measures the cost of one full poll of every shard, in microseconds.
fn idle_poll_us<S: TcpStack>(stacks: &[S]) -> f64 {
    const ROUNDS: u32 = 3;
    for stack in stacks {
        stack.poll_once();
    }
    let started = Instant::now();
    for _ in 0..ROUNDS {
        for stack in stacks {
            stack.poll_once();
        }
    }
    started.elapsed().as_secs_f64() * 1e6 / f64::from(ROUNDS)
}

/// Runs the socket scaling workload over `stacks`, which are the shards.
///
/// All shards run on the calling thread. The workload opens sessions in
/// batches until the number of sockets of a step is reached, and it stops at
/// the first step that exceeds its budget, so a stack that slows down with the
/// socket count ends the run instead of hanging it.
pub async fn socket_scaling<S: TcpStack>(
    stacks: &[S],
    config: &ScaleConfig,
) -> Result<ScaleReport, BenchError> {
    let shards = stacks.len().max(1);
    let Some(first) = stacks.first() else {
        return Ok(ScaleReport {
            shards: 0,
            steps: Vec::new(),
            stopped: Some("no stacks".into()),
        });
    };
    let drivers: Vec<_> = stacks.iter().map(spawn_driver).collect();
    first
        .listen(config.probe_port)
        .map_err(io_error("listen probe"))?;
    let failures = Arc::new(AtomicU64::new(0));
    let probe_server = serve_requests(first, config.probe_port, 64, 64, Arc::clone(&failures));
    let rss_start = sysinfo::resident_bytes();

    let mut held: Vec<Vec<S::Session>> = stacks.iter().map(|_| Vec::new()).collect();
    let mut opened = vec![0usize; shards];
    let mut listened = vec![0usize; shards];
    let mut steps = Vec::new();
    let mut stopped = None;
    for &target in &config.steps {
        // A session holds two sockets, one on each host of a shard.
        let pairs_per_shard = target.div_ceil(2 * shards);
        let started = Instant::now();
        let deadline = started + config.step_budget;
        let before: u64 = stacks.iter().map(|s| s.stats().sockets_open).sum();
        let mut reached = true;
        let mut refused = None;
        for (index, stack) in stacks.iter().enumerate() {
            let need = pairs_per_shard.saturating_sub(opened[index]);
            let grown = grow_shard(
                stack,
                &mut held[index],
                &mut opened[index],
                &mut listened[index],
                need,
                config,
                deadline,
            )
            .await;
            match grown {
                Ok(true) => {}
                Ok(false) => {
                    reached = false;
                    break;
                }
                // A stack that refuses a socket has reached its limit, such as
                // the descriptor limit of a process, and that ends the run
                // with the steps measured so far.
                Err(BenchError::Io { what, source }) => {
                    reached = false;
                    refused = Some(format!("{what}: {source}"));
                    break;
                }
                Err(other) => return Err(other),
            }
        }
        let setup_secs = started.elapsed().as_secs_f64();
        let sockets: u64 = stacks.iter().map(|s| s.stats().sockets_open).sum();
        let rss_bytes = sysinfo::resident_bytes();
        let bytes_per_socket = match (rss_bytes, rss_start) {
            (Some(now), Some(start)) if sockets > 0 => {
                Some(now.saturating_sub(start) as f64 / sockets as f64)
            }
            _ => None,
        };
        let idle_poll = idle_poll_us(stacks);
        let probe = if refused.is_some() {
            // No descriptor is left for the probe sessions.
            RateReport::from_runs(Vec::new())
        } else {
            match probe_rate(first, config).await {
                Ok(probe) => probe,
                Err(BenchError::Io { what, source }) => {
                    refused = Some(format!("{what}: {source}"));
                    RateReport::from_runs(Vec::new())
                }
                Err(other) => return Err(other),
            }
        };
        steps.push(ScaleStep {
            target_sockets: target,
            sockets,
            reached,
            setup_secs,
            setup_sockets_per_s: if setup_secs > 0.0 {
                sockets.saturating_sub(before) as f64 / setup_secs
            } else {
                0.0
            },
            rss_bytes,
            bytes_per_socket,
            idle_poll_us: idle_poll,
            probe_sessions_per_s: probe.sessions_per_s,
            probe_latency: probe.latency,
        });
        if !reached || refused.is_some() {
            stopped = Some(match refused {
                Some(reason) => format!(
                    "step {target} stopped after {setup_secs:.1} s with {sockets} sockets, the stack refused a socket ({reason})"
                ),
                None => format!(
                    "step {target} stopped after {setup_secs:.1} s with {sockets} sockets, over the budget of {:?}",
                    config.step_budget
                ),
            });
            break;
        }
    }
    probe_server.abort();
    for driver in drivers {
        driver.abort();
    }
    if failures.load(Ordering::Relaxed) > 0 {
        return Err(BenchError::Task("the probe server failed".into()));
    }
    Ok(ScaleReport {
        shards,
        steps,
        stopped,
    })
}

/// Runs `probe_sessions` request sessions one after the other on `stack`.
async fn probe_rate<S: TcpStack>(
    stack: &S,
    config: &ScaleConfig,
) -> Result<RateReport, BenchError> {
    let mut buffer = [0u8; 64];
    let mut samples = Vec::with_capacity(config.probe_sessions);
    let started = Instant::now();
    for _ in 0..config.probe_sessions {
        let begin = Instant::now();
        request(stack, config.probe_port, &mut buffer, 64, 64)
            .await
            .map_err(io_error("probe session"))?;
        samples.push(micros(begin.elapsed()));
    }
    Ok(RateReport::from_runs(vec![RateRun {
        sessions: samples.len() as u64,
        errors: 0,
        elapsed: started.elapsed(),
        samples,
        stats: stack.stats(),
    }]))
}
