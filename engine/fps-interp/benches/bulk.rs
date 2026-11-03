// SPDX-License-Identifier: GPL-3.0-only
//! Bulk benchmark of the interpreter against hand written code.
//!
//! The workload repeats the Spike S1 bulk run in one process over loopback
//! TCP. One iteration runs 16 concurrent `web_get` sessions of 4 requests
//! with 1 MiB bodies, and every session runs its client and its server
//! side. Two engine workers, each a thread with its own runtime, run eight
//! sessions each. Three variants share the setup.
//!
//! - `interp` runs both sides with the interpreter, so large `fill()`
//!   bodies go out as vectored writes from the shared filler.
//! - `handwritten` replays the session with hand written code and one pre
//!   rendered response, which the Gate 0 criterion compares against.
//! - `handwritten-copy` is the hand written code that renders each response
//!   into a buffer with `render_into`, which is the copy that the
//!   interpreter made before vectored writes.
//!
//! `cargo bench -p fps-interp --bench bulk` runs the variants under
//! criterion. With `FPS_BENCH_OUT=<file>` the benchmark skips criterion and
//! runs a paired measurement instead, which interleaves the variants round
//! by round so that load from other processes falls on all of them alike,
//! and writes an fps-bench/v1 result for `bench/compare.py`.
//! `FPS_BENCH_ROUNDS` sets the number of measured rounds, 300 by default.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::ops::Range;
use std::pin::Pin;
use std::sync::{mpsc, Arc};
use std::task::{Context, Poll};
use std::thread;
use std::time::{Duration, Instant};

use criterion::{criterion_group, Criterion, Throughput};
use fps_flow::function::NoFunctions;
use fps_flow::program::Op;
use fps_flow::template::{Filler, RenderCtx};
use fps_flow::{Scenario, Template, Value, Vars};
use fps_interp::{session_rng, Limits, OpenRequest, Opener, Session, SessionCtx, Side};
use fps_io::SessionIo;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinSet, LocalSet};

const SCENARIO: &str =
    include_str!("../../../contracts/examples/modules/http1/scenarios/web_get.yaml");
const CONCURRENCY: usize = 16;
const REQUESTS: usize = 4;
const BODY: usize = 1 << 20;
const SEED: u64 = 42;
/// Engine workers, which equals the runtime threads of the S1 run.
const WORKERS: usize = 2;

/// A loopback TCP stream as a session transport.
struct Tcp(TcpStream);

impl AsyncRead for Tcp {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
    }
}

impl AsyncWrite for Tcp {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_write(cx, data)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.0.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
    }
}

impl SessionIo for Tcp {
    fn abort(&mut self) {
        // The benchmark never stops a session, so a plain close suffices.
    }
}

/// Dials the listener of the run.
struct Dial(SocketAddr);

impl Opener for Dial {
    type Io = Tcp;

    fn open(&mut self, _request: OpenRequest<'_>) -> impl Future<Output = io::Result<Tcp>> {
        let addr = self.0;
        async move {
            let s = TcpStream::connect(addr).await?;
            s.set_nodelay(true)?;
            Ok(Tcp(s))
        }
    }
}

/// Accepts the next session from the listener of the run.
struct Accept(Arc<TcpListener>);

impl Opener for Accept {
    type Io = Tcp;

    fn open(&mut self, _request: OpenRequest<'_>) -> impl Future<Output = io::Result<Tcp>> {
        let l = Arc::clone(&self.0);
        async move {
            let (s, _) = l.accept().await?;
            s.set_nodelay(true)?;
            Ok(Tcp(s))
        }
    }
}

/// Everything one variant needs, shared by all sessions of a run.
struct Setup {
    scenario: Scenario,
    filler: Filler,
    request: Vec<u8>,
    response: Vec<u8>,
    response_template: Template,
}

impl Setup {
    fn new() -> Self {
        let mut scenario = Scenario::parse(SCENARIO).unwrap();
        let size = i64::try_from(BODY).unwrap();
        let requests = i64::try_from(REQUESTS).unwrap();
        scenario.vars.insert("body_size".into(), Value::Int(size));
        scenario
            .vars
            .insert("requests".into(), Value::Int(requests));
        scenario.fold_constants().unwrap();
        let filler = Filler::new(SEED);
        let flow = &scenario.flows[0];
        let first_send = |ops: &[Op]| {
            ops.iter()
                .find_map(|op| match op {
                    Op::Send(t) => Some(t.clone()),
                    _ => None,
                })
                .unwrap()
        };
        let request_template = first_send(&flow.client.ops);
        let response_template = first_send(&flow.server.as_ref().unwrap().ops);
        let request = render(&scenario.vars, &filler, &request_template, 0);
        let response = render(&scenario.vars, &filler, &response_template, 0);
        assert_eq!(
            response.len() - BODY,
            response.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4
        );
        Setup {
            scenario,
            filler,
            request,
            response,
            response_template,
        }
    }
}

/// Renders `t` with the generator of server session `session_id`.
fn render(vars: &Vars, filler: &Filler, t: &Template, session_id: u64) -> Vec<u8> {
    let mut rng = session_rng(SEED, session_id, Side::Server);
    let overlay = Vars::new();
    let mut none = NoFunctions;
    let mut ctx = RenderCtx {
        vars,
        overlay: &overlay,
        session_id,
        flow: "page",
        last: &[],
        rng: &mut rng,
        filler,
        functions: &mut none,
    };
    let mut out = Vec::new();
    t.render_into(&mut ctx, &mut out).unwrap();
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Variant {
    Interp,
    Handwritten,
    HandwrittenCopy,
}

/// Runs both sides of one session with the interpreter.
async fn interp_session(setup: Arc<Setup>, addr: SocketAddr, listener: Arc<TcpListener>, id: u64) {
    let flow = &setup.scenario.flows[0];
    let server_program = flow.server.as_ref().unwrap();
    let (mut fc, mut fs) = (NoFunctions, NoFunctions);
    let mut client = Session::new(SessionCtx {
        vars: &setup.scenario.vars,
        filler: &setup.filler,
        flow: &flow.id,
        side: Side::Client,
        session_id: id,
        rng: session_rng(SEED, id, Side::Client),
        functions: &mut fc,
        limits: Limits::default(),
    });
    let mut server = Session::new(SessionCtx {
        vars: &setup.scenario.vars,
        filler: &setup.filler,
        flow: &flow.id,
        side: Side::Server,
        session_id: id,
        rng: session_rng(SEED, id, Side::Server),
        functions: &mut fs,
        limits: Limits::default(),
    });
    let (mut dial, mut accept) = (Dial(addr), Accept(listener));
    let (c, s) = tokio::join!(
        client.run_program(&mut dial, &flow.client, std::future::pending()),
        server.run_program(&mut accept, server_program, std::future::pending()),
    );
    c.unwrap();
    s.unwrap();
    assert_eq!(client.stats().bytes_rx, server.stats().bytes_tx);
}

/// Reads until `\r\n\r\n` and keeps the bytes after it in `buf`.
async fn read_head(io: &mut TcpStream, buf: &mut Vec<u8>, scratch: &mut [u8]) {
    loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            buf.drain(..i + 4);
            return;
        }
        let n = io.read(scratch).await.unwrap();
        assert!(n > 0, "peer closed");
        buf.extend_from_slice(&scratch[..n]);
    }
}

/// Closes with FIN and drains until the peer closes.
async fn close_fin(io: &mut TcpStream, scratch: &mut [u8]) {
    io.shutdown().await.unwrap();
    while io.read(scratch).await.unwrap() > 0 {}
}

/// Runs both sides of one session with hand written code.
async fn handwritten_session(
    setup: Arc<Setup>,
    addr: SocketAddr,
    listener: Arc<TcpListener>,
    id: u64,
    copy: bool,
) {
    let client = {
        let setup = Arc::clone(&setup);
        async move {
            let mut io = TcpStream::connect(addr).await.unwrap();
            io.set_nodelay(true).unwrap();
            let mut scratch = vec![0u8; 64 * 1024];
            let mut buf = Vec::with_capacity(16 * 1024);
            for _ in 0..REQUESTS {
                io.write_all(&setup.request).await.unwrap();
                read_head(&mut io, &mut buf, &mut scratch).await;
                let mut left = BODY - buf.len().min(BODY);
                buf.drain(..buf.len().min(BODY));
                while left > 0 {
                    let n = io.read(&mut scratch).await.unwrap();
                    assert!(n > 0, "peer closed");
                    let used = n.min(left);
                    buf.extend_from_slice(&scratch[used..n]);
                    left -= used;
                }
            }
            close_fin(&mut io, &mut scratch).await;
        }
    };
    let server = async move {
        let (mut io, _) = listener.accept().await.unwrap();
        io.set_nodelay(true).unwrap();
        let mut scratch = vec![0u8; 16 * 1024];
        let mut buf = Vec::with_capacity(16 * 1024);
        let mut rng = session_rng(SEED, id, Side::Server);
        let mut wbuf = Vec::with_capacity(16 * 1024);
        let overlay = Vars::new();
        let mut none = NoFunctions;
        for _ in 0..REQUESTS {
            read_head(&mut io, &mut buf, &mut scratch).await;
            if copy {
                wbuf.clear();
                let ctx = &mut RenderCtx {
                    vars: &setup.scenario.vars,
                    overlay: &overlay,
                    session_id: id,
                    flow: "page",
                    last: &[],
                    rng: &mut rng,
                    filler: &setup.filler,
                    functions: &mut none,
                };
                setup.response_template.render_into(ctx, &mut wbuf).unwrap();
                io.write_all(&wbuf).await.unwrap();
            } else {
                io.write_all(&setup.response).await.unwrap();
            }
        }
        close_fin(&mut io, &mut scratch).await;
    };
    tokio::join!(client, server);
}

/// Runs the sessions of one worker for one iteration.
async fn round(variant: Variant, setup: Arc<Setup>, listener: Arc<TcpListener>, ids: Range<u64>) {
    let addr = listener.local_addr().unwrap();
    let mut set = JoinSet::new();
    for id in ids {
        let (setup, listener) = (Arc::clone(&setup), Arc::clone(&listener));
        set.spawn_local(async move {
            match variant {
                Variant::Interp => interp_session(setup, addr, listener, id).await,
                Variant::Handwritten => {
                    handwritten_session(setup, addr, listener, id, false).await;
                }
                Variant::HandwrittenCopy => {
                    handwritten_session(setup, addr, listener, id, true).await;
                }
            }
        });
    }
    while let Some(r) = set.join_next().await {
        r.unwrap();
    }
}

/// One engine worker, a thread with its own runtime and listener that owns
/// its sessions, as the engine runs them.
struct Worker {
    jobs: mpsc::Sender<(Variant, Range<u64>)>,
    done: mpsc::Receiver<()>,
}

impl Worker {
    fn spawn(setup: Arc<Setup>) -> Self {
        let (jobs, job_rx) = mpsc::channel::<(Variant, Range<u64>)>();
        let (done_tx, done) = mpsc::channel();
        thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let listener = Arc::new(rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap());
            let local = LocalSet::new();
            for (variant, ids) in job_rx {
                let job = round(variant, Arc::clone(&setup), Arc::clone(&listener), ids);
                local.block_on(&rt, job);
                if done_tx.send(()).is_err() {
                    break;
                }
            }
        });
        Worker { jobs, done }
    }
}

/// Runs `CONCURRENCY` sessions spread over the workers and returns the
/// time until the last one finished.
fn iteration(workers: &[Worker], variant: Variant, base: u64) -> Duration {
    let per = (CONCURRENCY / workers.len()) as u64;
    let start = Instant::now();
    for (i, w) in (0u64..).zip(workers) {
        let first = base + i * per;
        w.jobs.send((variant, first..first + per)).unwrap();
    }
    for w in workers {
        w.done.recv().unwrap();
    }
    start.elapsed()
}

fn bulk(c: &mut Criterion) {
    let setup = Arc::new(Setup::new());
    let workers: Vec<Worker> = (0..WORKERS)
        .map(|_| Worker::spawn(Arc::clone(&setup)))
        .collect();
    let mut group = c.benchmark_group("bulk-1mib");
    group.throughput(Throughput::Bytes((CONCURRENCY * REQUESTS * BODY) as u64));
    group.sample_size(30);
    group.measurement_time(Duration::from_secs(20));
    let mut base = 0u64;
    for (name, variant) in [
        ("interp", Variant::Interp),
        ("handwritten", Variant::Handwritten),
        ("handwritten-copy", Variant::HandwrittenCopy),
    ] {
        group.bench_function(name, |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    total += iteration(&workers, variant, base);
                    base += CONCURRENCY as u64;
                }
                total
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bulk);

const VARIANTS: [(&str, Variant); 3] = [
    ("interp", Variant::Interp),
    ("handwritten", Variant::Handwritten),
    ("handwritten-copy", Variant::HandwrittenCopy),
];
/// The Gate 0 criterion, a session rate of at least 75% of hand written code.
const MIN_RATIO: f64 = 0.75;
const WARMUP_ROUNDS: usize = 10;

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    let m = v.len() / 2;
    if v.len().is_multiple_of(2) {
        (v[m - 1] + v[m]) / 2.0
    } else {
        v[m]
    }
}

/// Reads the host facts that `bench/compare.py` checks.
fn host_json() -> String {
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into());
    let vcpus = thread::available_parallelism().map_or(0, usize::from);
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map_or_else(|_| "unknown".into(), |k| k.trim().to_string());
    format!(r#"{{"cpu": {cpu:?}, "vcpus": {vcpus}, "kernel": {kernel:?}}}"#)
}

/// Tolerance of the absolute rates. Five consecutive 300 round runs on the
/// shared 2 vCPU recording host differed by up to a factor of two in
/// absolute rate, because other processes loaded the host, so the rates
/// only catch gross regressions.
const RATE_TOLERANCE: f64 = 0.60;
/// Tolerance of the ratios, which moved by at most 7% between those runs,
/// because the paired rounds put the same load on every variant. The
/// ratios are the metrics to watch.
const RATIO_TOLERANCE: f64 = 0.10;

fn metric(name: &str, value: f64, tolerance: f64, unit: &str) -> String {
    format!(
        r#"    "{name}": {{"value": {value:.4}, "better": "higher", "tolerance": {tolerance:.2}, "unit": "{unit}"}}"#
    )
}

/// Runs the variants in rotated order round by round and writes the
/// medians as an fps-bench/v1 result.
fn paired(out: &std::ffi::OsStr) {
    let rounds: usize = std::env::var("FPS_BENCH_ROUNDS")
        .ok()
        .and_then(|r| r.parse().ok())
        .unwrap_or(300);
    let setup = Arc::new(Setup::new());
    let workers: Vec<Worker> = (0..WORKERS)
        .map(|_| Worker::spawn(Arc::clone(&setup)))
        .collect();
    let mut times = [Vec::new(), Vec::new(), Vec::new()];
    let (mut ratios, mut copy_ratios) = (Vec::new(), Vec::new());
    let mut base = 0u64;
    for round in 0..WARMUP_ROUNDS + rounds {
        let mut t = [0f64; 3];
        for k in 0..VARIANTS.len() {
            let i = (round + k) % VARIANTS.len();
            t[i] = iteration(&workers, VARIANTS[i].1, base).as_secs_f64();
            base += CONCURRENCY as u64;
        }
        if round >= WARMUP_ROUNDS {
            for i in 0..3 {
                times[i].push(t[i]);
            }
            ratios.push(t[1] / t[0]);
            copy_ratios.push(t[1] / t[2]);
        }
    }
    let mut metrics = Vec::new();
    for (i, (name, _)) in VARIANTS.iter().enumerate() {
        let rate = CONCURRENCY as f64 / median(&mut times[i]);
        let bits = rate * (REQUESTS * BODY * 8) as f64;
        metrics.push(metric(
            &format!("bulk-1mib-{name}.sessions_per_s"),
            rate,
            RATE_TOLERANCE,
            "sessions/s",
        ));
        metrics.push(metric(
            &format!("bulk-1mib-{name}.l7_bits_per_s"),
            bits,
            RATE_TOLERANCE,
            "bit/s",
        ));
    }
    let ratio = median(&mut ratios);
    metrics.push(metric(
        "bulk-1mib.interp_over_baseline",
        ratio,
        RATIO_TOLERANCE,
        "ratio",
    ));
    metrics.push(metric(
        "bulk-1mib.copy_over_baseline",
        median(&mut copy_ratios),
        RATIO_TOLERANCE,
        "ratio",
    ));
    let recorded = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .map_or_else(
            |_| "unknown".into(),
            |o| String::from_utf8_lossy(&o.stdout).trim().to_string(),
        );
    let doc = format!(
        "{{\n  \"format\": \"fps-bench/v1\",\n  \"suite\": \"fps-interp-bulk\",\n  \"recorded\": \"{recorded}\",\n  \"source\": \"FPS_BENCH_ROUNDS={rounds} cargo bench -p fps-interp --bench bulk\",\n  \"host\": {},\n  \"metrics\": {{\n{}\n  }}\n}}\n",
        host_json(),
        metrics.join(",\n")
    );
    std::fs::write(out, doc).unwrap();
    let verdict = if ratio >= MIN_RATIO {
        "meets"
    } else {
        "misses"
    };
    println!("interp over hand written {ratio:.3} over {rounds} rounds, {verdict} the {MIN_RATIO} criterion");
}

fn main() {
    if let Some(out) = std::env::var_os("FPS_BENCH_OUT") {
        paired(&out);
        return;
    }
    benches();
    Criterion::default().configure_from_args().final_summary();
}
