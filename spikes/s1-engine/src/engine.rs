//! Benchmark driver: emulated servers, closed loop clients, and statistics.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use hdrhistogram::Histogram;
use rand::rngs::SmallRng;
use rand::SeedableRng;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket, TcpStream};

use crate::interp::{Fail, Session, Shared};
use crate::program::{Program, Scenario};
use crate::template::{Filler, RenderCtx, Template, Vars};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Execute the scenario's flow programs.
    Interp,
    /// Hand written equivalent of http1/web_get with pre rendered bytes.
    Baseline,
}

#[derive(Clone, Debug)]
pub struct BenchConfig {
    pub mode: Mode,
    pub concurrency: usize,
    pub duration: Duration,
    pub server_ip: IpAddr,
    pub client_net: Ipv4Addr,
    pub client_hosts: u32,
    pub seed: u64,
    pub connect_timeout: Duration,
    /// Optional cap on new sessions per second across all clients.
    pub rate: Option<f64>,
}

impl Default for BenchConfig {
    fn default() -> Self {
        BenchConfig {
            mode: Mode::Interp,
            concurrency: 64,
            duration: Duration::from_secs(10),
            server_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            client_net: Ipv4Addr::new(127, 1, 0, 0),
            client_hosts: 254,
            seed: 1,
            connect_timeout: Duration::from_secs(5),
            rate: None,
        }
    }
}

#[derive(Default)]
struct Counters {
    started: AtomicU64,
    ok: AtomicU64,
    failed: AtomicU64,
    server_ok: AtomicU64,
    server_failed: AtomicU64,
    bytes_tx: AtomicU64,
    bytes_rx: AtomicU64,
    active: AtomicI64,
    max_active: AtomicI64,
    failures: Mutex<HashMap<String, u64>>,
}

impl Counters {
    fn fail(&self, label: String) {
        self.failed.fetch_add(1, Ordering::Relaxed);
        *self.failures.lock().unwrap().entry(label).or_default() += 1;
    }
}

#[derive(Debug, Serialize)]
pub struct LatencyUs {
    pub p50: u64,
    pub p90: u64,
    pub p99: u64,
    pub max: u64,
}

#[derive(Debug, Serialize)]
pub struct BenchResult {
    pub scenario: String,
    pub mode: Mode,
    pub concurrency: usize,
    pub runtime_threads: usize,
    pub duration_s: f64,
    pub sessions_ok: u64,
    pub sessions_failed: u64,
    pub server_sessions_ok: u64,
    pub server_sessions_failed: u64,
    pub failures: HashMap<String, u64>,
    pub sessions_per_s: f64,
    /// Median of per second session counts, excluding the first and last second.
    pub steady_sessions_per_s: f64,
    pub per_second: Vec<u64>,
    /// Application layer bits per second, both directions, measured at the client.
    pub l7_bits_per_s: f64,
    pub session_latency_us: LatencyUs,
    pub max_concurrent_sessions: i64,
    pub cpu_seconds: f64,
    pub cpu_cores_busy: f64,
    pub sessions_per_cpu_second: f64,
}

fn cpu_seconds() -> f64 {
    // SAFETY: getrusage writes into the zeroed struct we pass.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    tv(ru.ru_utime) + tv(ru.ru_stime)
}

fn static_int(t: &Template, vars: &Vars, filler: &Filler) -> Result<i64> {
    let overlay = Vars::new();
    let mut rng = SmallRng::seed_from_u64(0);
    let mut ctx = RenderCtx {
        vars,
        overlay: &overlay,
        session_id: 0,
        flow: "",
        rng: &mut rng,
        filler,
    };
    t.render_int(&mut ctx)
}

fn port_of(p: &Program, vars: &Vars, filler: &Filler) -> Result<u16> {
    let port = static_int(&p.port, vars, filler)?;
    u16::try_from(port)
        .ok()
        .filter(|p| *p > 0)
        .with_context(|| format!("port {port} is out of range"))
}

async fn listen(addr: SocketAddr) -> Result<TcpListener> {
    let sock = if addr.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    sock.set_reuseaddr(true)?;
    sock.bind(addr).with_context(|| format!("binding {addr}"))?;
    Ok(sock.listen(4096)?)
}

async fn connect(src: Ipv4Addr, dst: SocketAddr, timeout: Duration) -> std::io::Result<TcpStream> {
    let sock = TcpSocket::new_v4()?;
    // Let connect() pick the source port per destination, which keeps the
    // ephemeral port space per (source, destination) pair.
    let one: libc::c_int = 1;
    // SAFETY: valid fd and option buffer for the duration of the call.
    unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::IPPROTO_IP,
            libc::IP_BIND_ADDRESS_NO_PORT,
            &one as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
    sock.bind(SocketAddr::new(IpAddr::V4(src), 0))?;
    let stream = tokio::time::timeout(timeout, sock.connect(dst))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "connect timeout"))??;
    stream.set_nodelay(true)?;
    Ok(stream)
}

/// Pre rendered bytes for the baseline mode.
struct Baseline {
    requests: usize,
    request: Vec<u8>,
    response: Vec<u8>,
    body_size: usize,
}

impl Baseline {
    fn new(vars: &Vars, filler: &Filler) -> Result<Self> {
        let get = |k: &str| {
            vars.get(k)
                .with_context(|| format!("baseline mode needs variable '{k}' (http1/web_get)"))
        };
        let body_size = get("body_size")?.as_i64().context("body_size")? as usize;
        let requests = get("requests")?.as_i64().context("requests")? as usize;
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {}\r\nAccept: */*\r\n\r\n",
            get("path")?,
            get("host")?,
            get("user_agent")?
        )
        .into_bytes();
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nServer: fps/0.1\r\nContent-Type: text/html\r\nContent-Length: {body_size}\r\n\r\n"
        )
        .into_bytes();
        filler.fill(&mut response, body_size, &mut SmallRng::seed_from_u64(0));
        Ok(Baseline {
            requests,
            request,
            response,
            body_size,
        })
    }
}

async fn read_until(io: &mut TcpStream, buf: &mut Vec<u8>, pat: &[u8]) -> Result<(), Fail> {
    let finder = memchr::memmem::Finder::new(pat);
    loop {
        if let Some(i) = finder.find(buf) {
            buf.drain(..i + pat.len());
            return Ok(());
        }
        let mut chunk = [0u8; 16384];
        match io.read(&mut chunk).await {
            Ok(0) => return Err(Fail::Eof { op: 0 }),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) => {
                return Err(Fail::Io {
                    op: 0,
                    kind: e.kind(),
                })
            }
        }
    }
}

async fn read_exact_n(io: &mut TcpStream, buf: &mut Vec<u8>, mut n: usize) -> Result<(), Fail> {
    let take = n.min(buf.len());
    buf.drain(..take);
    n -= take;
    let mut chunk = vec![0u8; 65536];
    while n > 0 {
        match io.read(&mut chunk).await {
            Ok(0) => return Err(Fail::Eof { op: 1 }),
            Ok(k) if k <= n => n -= k,
            Ok(k) => {
                buf.extend_from_slice(&chunk[n..k]);
                n = 0;
            }
            Err(e) => {
                return Err(Fail::Io {
                    op: 1,
                    kind: e.kind(),
                })
            }
        }
    }
    Ok(())
}

async fn drain(io: &mut TcpStream) -> Result<(), Fail> {
    io.shutdown().await.map_err(|e| Fail::Io {
        op: 2,
        kind: e.kind(),
    })?;
    let mut chunk = [0u8; 16384];
    loop {
        match io.read(&mut chunk).await {
            Ok(0) => return Ok(()),
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => return Ok(()),
            Err(e) => {
                return Err(Fail::Io {
                    op: 2,
                    kind: e.kind(),
                })
            }
        }
    }
}

async fn baseline_client(io: &mut TcpStream, b: &Baseline) -> Result<(u64, u64), Fail> {
    let mut buf = Vec::with_capacity(16384);
    let (mut tx, mut rx) = (0u64, 0u64);
    for _ in 0..b.requests {
        io.write_all(&b.request).await.map_err(|e| Fail::Io {
            op: 0,
            kind: e.kind(),
        })?;
        tx += b.request.len() as u64;
        read_until(io, &mut buf, b"\r\n\r\n").await?;
        read_exact_n(io, &mut buf, b.body_size).await?;
        rx += b.response.len() as u64;
    }
    drain(io).await?;
    Ok((tx, rx))
}

async fn baseline_server(io: &mut TcpStream, b: &Baseline) -> Result<(), Fail> {
    let mut buf = Vec::with_capacity(16384);
    for _ in 0..b.requests {
        read_until(io, &mut buf, b"\r\n\r\n").await?;
        io.write_all(&b.response).await.map_err(|e| Fail::Io {
            op: 1,
            kind: e.kind(),
        })?;
    }
    drain(io).await
}

struct Ctx {
    scenario: Scenario,
    filler: Filler,
    cfg: BenchConfig,
    counters: Counters,
    stop: AtomicBool,
    next_session: AtomicU64,
    baseline: Option<Baseline>,
    ports: Vec<u16>,
}

async fn serve(ctx: Arc<Ctx>, flow_idx: usize, listener: TcpListener) {
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(x) => x,
            Err(_) => continue,
        };
        let ctx = ctx.clone();
        tokio::spawn(async move {
            let _ = stream.set_nodelay(true);
            let result = if let Some(b) = &ctx.baseline {
                let mut s = stream;
                baseline_server(&mut s, b).await
            } else {
                let flow = &ctx.scenario.flows[flow_idx];
                let ops = &flow.server.as_ref().expect("server program").ops;
                let sid = ctx.next_session.fetch_add(1, Ordering::Relaxed);
                let rng = SmallRng::seed_from_u64(ctx.cfg.seed ^ sid.rotate_left(17) ^ 0xA5A5);
                let shared = Shared {
                    vars: &ctx.scenario.vars,
                    filler: &ctx.filler,
                };
                let mut session = Session::new(stream, shared, sid, &flow.id, rng);
                session.run(ops).await
            };
            match result {
                Ok(()) => ctx.counters.server_ok.fetch_add(1, Ordering::Relaxed),
                Err(_) => ctx.counters.server_failed.fetch_add(1, Ordering::Relaxed),
            };
        });
    }
}

async fn client_session(ctx: &Ctx, src: Ipv4Addr) -> Result<(u64, u64), String> {
    let (mut tx, mut rx) = (0, 0);
    for (i, flow) in ctx.scenario.flows.iter().enumerate() {
        let dst = SocketAddr::new(ctx.cfg.server_ip, ctx.ports[i]);
        let stream = connect(src, dst, ctx.cfg.connect_timeout)
            .await
            .map_err(|e| format!("connect_{:?}", e.kind()).to_lowercase())?;
        if let Some(b) = &ctx.baseline {
            let mut s = stream;
            let (t, r) = baseline_client(&mut s, b).await.map_err(|f| f.label())?;
            tx += t;
            rx += r;
        } else {
            let sid = ctx.next_session.fetch_add(1, Ordering::Relaxed);
            let rng = SmallRng::seed_from_u64(ctx.cfg.seed ^ sid);
            let shared = Shared {
                vars: &ctx.scenario.vars,
                filler: &ctx.filler,
            };
            let mut session = Session::new(stream, shared, sid, &flow.id, rng);
            let res = session.run(&flow.client.ops).await;
            tx += session.stats.bytes_tx;
            rx += session.stats.bytes_rx;
            res.map_err(|f| f.label())?;
        }
    }
    Ok((tx, rx))
}

async fn client_worker(ctx: Arc<Ctx>, worker: usize) -> Histogram<u64> {
    let mut hist = Histogram::<u64>::new_with_bounds(1, 60_000_000, 3).expect("histogram");
    let mut host = worker as u32;
    let pace = ctx
        .cfg
        .rate
        .map(|r| Duration::from_secs_f64(ctx.cfg.concurrency as f64 / r));
    let mut next_start = Instant::now();
    while !ctx.stop.load(Ordering::Relaxed) {
        if let Some(p) = pace {
            tokio::time::sleep_until(next_start.into()).await;
            next_start += p;
        }
        host = host.wrapping_add(1);
        let src = Ipv4Addr::from(u32::from(ctx.cfg.client_net) + 1 + host % ctx.cfg.client_hosts);
        let c = &ctx.counters;
        c.started.fetch_add(1, Ordering::Relaxed);
        let active = c.active.fetch_add(1, Ordering::Relaxed) + 1;
        c.max_active.fetch_max(active, Ordering::Relaxed);
        let t0 = Instant::now();
        let res = client_session(&ctx, src).await;
        c.active.fetch_sub(1, Ordering::Relaxed);
        match res {
            Ok((tx, rx)) => {
                c.ok.fetch_add(1, Ordering::Relaxed);
                c.bytes_tx.fetch_add(tx, Ordering::Relaxed);
                c.bytes_rx.fetch_add(rx, Ordering::Relaxed);
                let _ = hist.record(t0.elapsed().as_micros().max(1) as u64);
            }
            Err(label) => c.fail(label),
        }
    }
    hist
}

/// Run a benchmark on the current Tokio runtime.
pub async fn run(
    scenario: Scenario,
    cfg: BenchConfig,
    runtime_threads: usize,
) -> Result<BenchResult> {
    if cfg.concurrency == 0 {
        bail!("concurrency must be positive");
    }
    let filler = Filler::new(cfg.seed);
    let mut ports = Vec::new();
    for flow in &scenario.flows {
        let server = flow
            .server
            .as_ref()
            .with_context(|| format!("flow '{}' has no server side to emulate", flow.id))?;
        let sp = port_of(server, &scenario.vars, &filler)?;
        let cp = port_of(&flow.client, &scenario.vars, &filler)?;
        if sp != cp {
            bail!(
                "flow '{}' connects to port {cp} but its server accepts on {sp}",
                flow.id
            );
        }
        if flow.client.host.is_some() {
            bail!(
                "flow '{}': explicit connect hosts are outside spike S1",
                flow.id
            );
        }
        ports.push(sp);
    }
    let mut seen = std::collections::HashSet::new();
    if !ports.iter().all(|p| seen.insert(*p)) {
        bail!("spike S1 needs a distinct server port per flow");
    }
    let baseline = match cfg.mode {
        Mode::Baseline => {
            if scenario.flows.len() != 1 {
                bail!("baseline mode reproduces http1/web_get, which has one flow");
            }
            Some(Baseline::new(&scenario.vars, &filler)?)
        }
        Mode::Interp => None,
    };

    let ctx = Arc::new(Ctx {
        scenario,
        filler,
        cfg: cfg.clone(),
        counters: Counters::default(),
        stop: AtomicBool::new(false),
        next_session: AtomicU64::new(0),
        baseline,
        ports: ports.clone(),
    });

    let mut servers = Vec::new();
    for (i, port) in ports.iter().enumerate() {
        let listener = listen(SocketAddr::new(cfg.server_ip, *port)).await?;
        servers.push(tokio::spawn(serve(ctx.clone(), i, listener)));
    }

    let cpu0 = cpu_seconds();
    let t0 = Instant::now();
    let workers: Vec<_> = (0..cfg.concurrency)
        .map(|w| tokio::spawn(client_worker(ctx.clone(), w)))
        .collect();

    let mut per_second = Vec::new();
    let mut last = 0;
    let mut tick =
        tokio::time::interval_at((t0 + Duration::from_secs(1)).into(), Duration::from_secs(1));
    for _ in 0..cfg.duration.as_secs().max(1) {
        tick.tick().await;
        let now = ctx.counters.ok.load(Ordering::Relaxed);
        per_second.push(now - last);
        last = now;
    }
    ctx.stop.store(true, Ordering::Relaxed);
    let mut hist = Histogram::<u64>::new_with_bounds(1, 60_000_000, 3)?;
    for w in workers {
        hist.add(w.await?)?;
    }
    let wall = t0.elapsed().as_secs_f64();
    let cpu = cpu_seconds() - cpu0;
    // Give server tasks a moment to record the last sessions.
    tokio::time::sleep(Duration::from_millis(200)).await;
    for s in servers {
        s.abort();
    }

    let c = &ctx.counters;
    let ok = c.ok.load(Ordering::Relaxed);
    let mut steady: Vec<u64> = if per_second.len() > 2 {
        per_second[1..per_second.len() - 1].to_vec()
    } else {
        per_second.clone()
    };
    steady.sort_unstable();
    let steady_median = steady.get(steady.len() / 2).copied().unwrap_or(0) as f64;
    let bytes = c.bytes_tx.load(Ordering::Relaxed) + c.bytes_rx.load(Ordering::Relaxed);
    let q = |p: f64| {
        if hist.is_empty() {
            0
        } else {
            hist.value_at_quantile(p)
        }
    };
    let failures = c.failures.lock().unwrap().clone();
    Ok(BenchResult {
        scenario: ctx.scenario.name.clone(),
        mode: cfg.mode,
        concurrency: cfg.concurrency,
        runtime_threads,
        duration_s: wall,
        sessions_ok: ok,
        sessions_failed: c.failed.load(Ordering::Relaxed),
        server_sessions_ok: c.server_ok.load(Ordering::Relaxed),
        server_sessions_failed: c.server_failed.load(Ordering::Relaxed),
        failures,
        sessions_per_s: ok as f64 / wall,
        steady_sessions_per_s: steady_median,
        per_second,
        l7_bits_per_s: bytes as f64 * 8.0 / wall,
        session_latency_us: LatencyUs {
            p50: q(0.5),
            p90: q(0.9),
            p99: q(0.99),
            max: hist.max(),
        },
        max_concurrent_sessions: c.max_active.load(Ordering::Relaxed),
        cpu_seconds: cpu,
        cpu_cores_busy: cpu / wall,
        sessions_per_cpu_second: if cpu > 0.0 { ok as f64 / cpu } else { 0.0 },
    })
}
