// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of the run time commands `UpdateRates`, `StartCapture`, and
//! `StopCapture` against an in process fake controller.
//!
//! The harness copies `tests/session.rs`. The plan is the example plan on
//! loopback pools with an HTTP entry on TCP and a DNS entry on UDP, so one
//! update spans the TCP registry of `fps-runtime` and the UDP registry of the
//! agent. The fake controller finds the capture file where the Agent page says
//! it is, below the capture directory of the agent.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fps_agent::convert::plan_to_pb;
use fps_agent::modules::sha256_digest;
use fps_agent::pb::controller_service_server::{ControllerService, ControllerServiceServer};
use fps_agent::pb::{self, session_request, session_response, AgentState, ErrorCode};
use fps_agent::plan::Plan;
use fps_agent::session::{serve, Backoff, Config};
use serde_json::Value as Json;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tokio_stream::Stream;
use tonic::{Request, Response, Status, Streaming};

const WAIT: Duration = Duration::from_secs(20);

fn contracts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts")
}

/// Returns the example plan on loopback pools with the given phases.
fn loopback_plan(c: u8, s: u8, http_port: u16, dns_port: u16, phases: Json) -> pb::Plan {
    let text = std::fs::read_to_string(contracts().join("examples/plans/http_dns_smoke.json"))
        .expect("example plan");
    let mut plan: Json = serde_json::from_str(&text).expect("example plan JSON");
    for sc in plan["scenarios"].as_array_mut().expect("scenarios") {
        let src = sc["source"].as_str().expect("source");
        let src = src
            .replace("server_port: 8080", &format!("server_port: {http_port}"))
            .replace("server_port: 53", &format!("server_port: {dns_port}"));
        sc["digest"] = sha256_digest(src.as_bytes()).into();
        sc["source"] = src.into();
    }
    plan["hostPools"][0]["ipv4Prefix"] = format!("127.{c}.0.0/24").into();
    plan["hostPools"][1]["ipv4Prefix"] = format!("127.{s}.0.0/24").into();
    for w in plan["workers"].as_array_mut().expect("workers") {
        w["rates"][0]["sessionsPerSecond"] = 40.into();
        w["rates"][1]["sessionsPerSecond"] = 10.into();
    }
    plan["load"]["phases"] = phases;
    plan["warmup"] = "0s".into();
    let plan = Plan::parse(&plan.to_string()).expect("loopback plan");
    plan_to_pb(&plan)
}

fn phases(ramp: u32, steady: u32) -> Json {
    serde_json::json!([
        { "name": "ramp_up", "duration": format!("{ramp}s"), "shape": "PHASE_SHAPE_LINEAR", "endFraction": 1 },
        { "name": "steady", "duration": format!("{steady}s"), "shape": "PHASE_SHAPE_HOLD",
          "startFraction": 1, "endFraction": 1 }
    ])
}

type Outbound = mpsc::Sender<Result<pb::SessionResponse, Status>>;

/// What the fake controller shares with the test.
#[derive(Default)]
struct Shared {
    registrations: Mutex<Vec<pb::RegisterRequest>>,
    /// The sender of the current session.
    outbound: Mutex<Option<Outbound>>,
    /// The test end of the agent messages, tagged with the session number.
    inbound: Mutex<Option<mpsc::UnboundedSender<(u64, pb::SessionRequest)>>>,
    sessions: Mutex<u64>,
    /// While true, the fake neither acknowledges heartbeats nor reads.
    silent: AtomicBool,
    /// When set, an upload call ends with `UNAVAILABLE` once it received this
    /// many bytes, like a controller that loses its connection.
    cut_upload_after: Mutex<Option<u64>>,
    /// What every `UploadCapture` call received, in the order they ended.
    uploads: Mutex<Vec<Uploaded>>,
}

/// One `UploadCapture` call as the fake controller saw it.
#[derive(Debug, Default)]
struct Uploaded {
    open: Option<pb::UploadCaptureOpen>,
    /// The chunks in arrival order.
    chunks: Vec<(u64, Vec<u8>)>,
    end: Option<pb::CaptureEnd>,
    /// Set when the call ended with a status because the fake cut it.
    cut: bool,
}

#[derive(Clone)]
struct Fake(Arc<Shared>);

#[tonic::async_trait]
impl ControllerService for Fake {
    async fn register(
        &self,
        req: Request<pb::RegisterRequest>,
    ) -> Result<Response<pb::RegisterResponse>, Status> {
        let req = req.into_inner();
        let id = req.agent_id.clone();
        let mut regs = self.0.registrations.lock().unwrap();
        regs.push(req);
        Ok(Response::new(pb::RegisterResponse {
            controller_id: "fake".into(),
            agent_id: id,
            session_token: format!("token-{}", regs.len()),
            heartbeat_interval: Some(prost_types::Duration {
                seconds: 0,
                nanos: 200_000_000,
            }),
            heartbeat_timeout: Some(prost_types::Duration {
                seconds: 1,
                nanos: 0,
            }),
        }))
    }

    async fn upload_capture(
        &self,
        req: Request<Streaming<pb::UploadCaptureRequest>>,
    ) -> Result<Response<pb::UploadCaptureResponse>, Status> {
        let mut stream = req.into_inner();
        let mut up = Uploaded::default();
        let cut_after = *self.0.cut_upload_after.lock().unwrap();
        let mut received = 0u64;
        let mut next = None;
        while let Some(msg) = stream.message().await? {
            match msg.body {
                Some(pb::upload_capture_request::Body::Open(o)) if up.open.is_none() => {
                    let token = format!("token-{}", self.0.registrations.lock().unwrap().len());
                    if o.session_token != token {
                        return Err(Status::unauthenticated("another session token"));
                    }
                    next = Some(o.start_offset);
                    up.open = Some(o);
                }
                Some(pb::upload_capture_request::Body::Chunk(c)) if up.open.is_some() => {
                    assert_eq!(Some(c.offset), next, "chunks are in ascending order");
                    assert!(!c.data.is_empty() && c.data.len() <= 262_144);
                    next = Some(c.offset + c.data.len() as u64);
                    received += c.data.len() as u64;
                    up.chunks.push((c.offset, c.data.to_vec()));
                    if cut_after.is_some_and(|n| received >= n) {
                        up.cut = true;
                        self.0.uploads.lock().unwrap().push(up);
                        return Err(Status::unavailable("the connection broke"));
                    }
                }
                Some(pb::upload_capture_request::Body::End(e)) if up.end.is_none() => {
                    up.end = Some(e);
                    break;
                }
                other => return Err(Status::invalid_argument(format!("unexpected {other:?}"))),
            }
        }
        let received_offset = next.unwrap_or(0);
        let verified = up.end.is_some();
        self.0.uploads.lock().unwrap().push(up);
        Ok(Response::new(pb::UploadCaptureResponse {
            received_offset,
            verified,
        }))
    }

    type SessionStream =
        Pin<Box<dyn Stream<Item = Result<pb::SessionResponse, Status>> + Send + 'static>>;

    async fn session(
        &self,
        req: Request<Streaming<pb::SessionRequest>>,
    ) -> Result<Response<Self::SessionStream>, Status> {
        let mut stream = req.into_inner();
        let first = stream.message().await?;
        let token = format!("token-{}", self.0.registrations.lock().unwrap().len());
        match first.and_then(|m| m.body) {
            Some(session_request::Body::Open(o)) if o.session_token == token => {}
            _ => return Err(Status::unauthenticated("open with the latest token first")),
        }
        let number = {
            let mut n = self.0.sessions.lock().unwrap();
            *n += 1;
            *n
        };
        let (tx, rx) = mpsc::channel(64);
        *self.0.outbound.lock().unwrap() = Some(tx.clone());
        let shared = Arc::clone(&self.0);
        tokio::spawn(async move {
            while let Ok(Some(msg)) = stream.message().await {
                if shared.silent.load(Ordering::SeqCst) {
                    continue;
                }
                if let Some(session_request::Body::Heartbeat(hb)) = &msg.body {
                    let ack = pb::SessionResponse {
                        command_id: 0,
                        body: Some(session_response::Body::HeartbeatAck(pb::HeartbeatAck {
                            sequence: hb.sequence,
                            controller_time: None,
                        })),
                    };
                    if tx.send(Ok(ack)).await.is_err() {
                        return;
                    }
                }
                if let Some(t) = shared.inbound.lock().unwrap().as_ref() {
                    let _ = t.send((number, msg));
                }
            }
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }
}

/// The test side of the fake controller and the agent under test.
struct Harness {
    shared: Arc<Shared>,
    inbound: mpsc::UnboundedReceiver<(u64, pb::SessionRequest)>,
    next_command: u64,
    /// Every heartbeat state and snapshot seen, in order.
    heartbeats: Vec<pb::Heartbeat>,
    snapshots: Vec<pb::CounterSnapshot>,
    stop_agent: Option<oneshot::Sender<()>>,
    agent: Option<tokio::task::JoinHandle<()>>,
}

impl Harness {
    async fn start(agent_id: &str, capture_dir: &Path) -> Self {
        Self::start_with(agent_id, capture_dir, |_| {}).await
    }

    /// Starts the agent after `tweak` changed its configuration.
    async fn start_with(
        agent_id: &str,
        capture_dir: &Path,
        tweak: impl FnOnce(&mut Config),
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let shared = Arc::new(Shared::default());
        let (tx, inbound) = mpsc::unbounded_channel();
        *shared.inbound.lock().unwrap() = Some(tx);
        let svc = ControllerServiceServer::new(Fake(Arc::clone(&shared)));
        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(svc)
                .serve_with_incoming(TcpListenerStream::new(listener))
                .await
                .unwrap();
        });
        let mut cfg = Config::new(
            format!("http://{addr}"),
            agent_id.into(),
            contracts().join("examples/modules"),
        );
        cfg.backoff = Backoff {
            initial: Duration::from_millis(50),
            max: Duration::from_millis(400),
        };
        cfg.captures.dir = capture_dir.to_path_buf();
        tweak(&mut cfg);
        let (stop_agent, rx) = oneshot::channel::<()>();
        let agent = tokio::spawn(async move {
            serve(cfg, async {
                let _ = rx.await;
            })
            .await
            .unwrap();
        });
        Self {
            shared,
            inbound,
            next_command: 0,
            heartbeats: Vec::new(),
            snapshots: Vec::new(),
            stop_agent: Some(stop_agent),
            agent: Some(agent),
        }
    }

    /// Receives the next agent message and records heartbeats and
    /// snapshots.
    async fn recv(&mut self) -> (u64, session_request::Body) {
        let (n, msg) = tokio::time::timeout(WAIT, self.inbound.recv())
            .await
            .expect("an agent message within the wait")
            .expect("the fake is alive");
        let body = msg.body.expect("a message body");
        match &body {
            session_request::Body::Heartbeat(hb) => self.heartbeats.push(hb.clone()),
            session_request::Body::Snapshot(s) => self.snapshots.push(s.clone()),
            _ => {}
        }
        (n, body)
    }

    /// Waits for a heartbeat that reports `state`.
    async fn wait_state(&mut self, state: AgentState) -> pb::Heartbeat {
        let deadline = Instant::now() + WAIT;
        loop {
            assert!(Instant::now() < deadline, "no heartbeat with {state:?}");
            if let (_, session_request::Body::Heartbeat(hb)) = self.recv().await {
                if hb.state == state as i32 {
                    return hb;
                }
            }
        }
    }

    /// Sends one command on the current session and waits for its result.
    async fn command(&mut self, body: session_response::Body) -> pb::CommandResult {
        self.next_command += 1;
        let id = self.next_command;
        let tx = self
            .shared
            .outbound
            .lock()
            .unwrap()
            .clone()
            .expect("a session");
        tx.send(Ok(pb::SessionResponse {
            command_id: id,
            body: Some(body),
        }))
        .await
        .unwrap();
        loop {
            if let (_, session_request::Body::Result(r)) = self.recv().await {
                if r.command_id == id {
                    return r;
                }
            }
        }
    }

    async fn shutdown(mut self) {
        if let Some(s) = self.stop_agent.take() {
            let _ = s.send(());
        }
        if let Some(a) = self.agent.take() {
            tokio::time::timeout(WAIT, a).await.unwrap().unwrap();
        }
    }
}

fn prepare(plan: &pb::Plan) -> session_response::Body {
    session_response::Body::Prepare(pb::PrepareCommand {
        plan: Some(plan.clone()),
        counter_interval: Some(prost_types::Duration {
            seconds: 0,
            nanos: 500_000_000,
        }),
        per_worker: false,
    })
}

fn start(plan_id: &str) -> session_response::Body {
    session_response::Body::Start(pb::StartCommand {
        plan_id: plan_id.into(),
        start_at: None,
    })
}

fn stop(plan_id: &str, mode: pb::StopMode, drain_secs: i64) -> session_response::Body {
    session_response::Body::Stop(pb::StopCommand {
        plan_id: plan_id.into(),
        mode: mode as i32,
        drain_timeout: Some(prost_types::Duration {
            seconds: drain_secs,
            nanos: 0,
        }),
    })
}

fn fetch(
    plan_id: &str,
    capture_id: &str,
    start_offset: u64,
    transfer_id: &str,
    max_chunk_bytes: u32,
) -> session_response::Body {
    session_response::Body::FetchCapture(pb::FetchCaptureCommand {
        plan_id: plan_id.into(),
        capture_id: capture_id.into(),
        start_offset,
        transfer_id: transfer_id.into(),
        max_chunk_bytes,
    })
}

/// Waits until the fake controller saw `n` upload calls and returns the
/// last one.
async fn wait_upload(h: &mut Harness, n: usize) -> Uploaded {
    let deadline = Instant::now() + WAIT;
    loop {
        {
            let mut ups = h.shared.uploads.lock().unwrap();
            if ups.len() >= n {
                return std::mem::take(&mut ups[n - 1]);
            }
        }
        assert!(Instant::now() < deadline, "no upload call {n}");
        // Keep the session served while the upload runs.
        let _ = tokio::time::timeout(Duration::from_millis(50), h.recv()).await;
    }
}

fn error_code(r: &pb::CommandResult) -> Option<ErrorCode> {
    r.error
        .as_ref()
        .map(|e| ErrorCode::try_from(e.code).expect("a known code"))
}

/// Sums the counters of `metric` on the client side whose `scenario` label
/// contains `scenario`. A snapshot holds the series of both sides, and a sum
/// over both would count every session twice.
fn by_scenario(snap: &pb::CounterSnapshot, metric: &str, scenario: &str) -> u64 {
    by_scenario_on(snap, metric, scenario, "client")
}

/// Like [`by_scenario`] for `side`.
fn by_scenario_on(snap: &pb::CounterSnapshot, metric: &str, scenario: &str, side: &str) -> u64 {
    snap.counters
        .iter()
        .filter(|c| {
            c.metric == metric
                && c.labels.get("side").is_some_and(|s| s == side)
                && c.labels
                    .get("scenario")
                    .is_some_and(|s| s.contains(scenario))
        })
        .map(|c| match c.value {
            Some(pb::counter_value::Value::Count(n)) => n,
            _ => 0,
        })
        .sum()
}

/// Receives `n` more snapshots.
async fn advance(h: &mut Harness, n: usize) {
    let target = h.snapshots.len() + n;
    while h.snapshots.len() < target {
        h.recv().await;
    }
}

/// Returns the sessions per second of `scenario` over the next `n` snapshots,
/// which come every half second.
async fn rate_of(h: &mut Harness, scenario: &str, n: usize) -> f64 {
    let last = |h: &Harness| {
        by_scenario(
            h.snapshots.last().expect("a snapshot"),
            "sessions.attempted",
            scenario,
        )
    };
    let before = last(h);
    advance(h, n).await;
    let after = last(h);
    (after - before) as f64 / (n as f64 * 0.5)
}

/// Sums the gauge `sessions.concurrent` of the last snapshot over both sides
/// and every scenario: the sessions that the agent has open right now.
fn open_sessions(h: &Harness) -> f64 {
    h.snapshots
        .last()
        .expect("a snapshot")
        .counters
        .iter()
        .filter(|c| c.metric == "sessions.concurrent")
        .map(|c| match c.value {
            Some(pb::counter_value::Value::Gauge(g)) => g,
            _ => 0.0,
        })
        .sum()
}

/// Pauses every mix entry and waits until no session is open, so that a
/// capture that stops next has no session in flight.
///
/// A session that is still open when a capture stops never reaches the writer:
/// the writer has gone when the session ends, and the capture counts the
/// session as dropped. Under load that happens to a few sessions of every stop,
/// so a test that asserts `dropped == 0` must stop the capture at rest. The
/// snapshot after the gauge reads zero leaves the recorder of the last session
/// time to drop, because the gauge falls when the session ends and the recorder
/// drops with the tapped transport just after.
async fn quiesce(h: &mut Harness, plan_id: &str, entries: &[&str]) {
    let paused = entries.iter().map(|e| sps(e, 0.0)).collect();
    let r = h.command(update_rates(plan_id, paused)).await;
    assert!(r.error.is_none(), "{r:?}");
    for _ in 0..40 {
        advance(h, 1).await;
        if open_sessions(h) == 0.0 {
            advance(h, 2).await;
            assert_eq!(open_sessions(h), 0.0, "a paused entry started a session");
            return;
        }
    }
    panic!("{} sessions stayed open after the pause", open_sessions(h));
}

fn update_rates(plan_id: &str, rates: Vec<pb::RateUpdate>) -> session_response::Body {
    session_response::Body::UpdateRates(pb::UpdateRatesCommand {
        plan_id: plan_id.into(),
        rates,
    })
}

fn sps(entry: &str, rate: f64) -> pb::RateUpdate {
    pb::RateUpdate {
        mix_entry: entry.into(),
        target: Some(pb::rate_update::Target::SessionsPerSecond(rate)),
    }
}

fn start_capture(plan_id: &str, id: &str) -> pb::StartCaptureCommand {
    pb::StartCaptureCommand {
        plan_id: plan_id.into(),
        capture_id: id.into(),
        ..Default::default()
    }
}

fn capture(cmd: pb::StartCaptureCommand) -> session_response::Body {
    session_response::Body::StartCapture(cmd)
}

fn stop_capture(plan_id: &str, id: &str) -> session_response::Body {
    session_response::Body::StopCapture(pb::StopCaptureCommand {
        plan_id: plan_id.into(),
        capture_id: id.into(),
    })
}

fn stop_result(r: &pb::CommandResult) -> &pb::StopCaptureResult {
    match &r.result {
        Some(pb::command_result::Result::StopCapture(s)) => s,
        _ => panic!("stop_capture result {r:?}"),
    }
}

/// Reads a pcapng file block by block, with no code of the agent, and
/// returns the number of section headers and of enhanced packet blocks.
fn read_pcapng(bytes: &[u8]) -> (usize, usize) {
    let (mut at, mut sections, mut packets) = (0, 0, 0);
    while at < bytes.len() {
        let kind = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        assert!(
            len >= 12 && len.is_multiple_of(4) && at + len <= bytes.len(),
            "block at {at}"
        );
        let tail = u32::from_le_bytes(bytes[at + len - 4..at + len].try_into().unwrap()) as usize;
        assert_eq!(tail, len, "trailing length of the block at {at}");
        match kind {
            0x0A0D_0D0A => sections += 1,
            6 => packets += 1,
            _ => {}
        }
        at += len;
    }
    (sections, packets)
}

/// Counts the TCP and the UDP packets of a pcapng file of Ethernet frames, and
/// the TCP packets whose source or destination port is `tcp_port`, with no
/// code of the agent.
fn transports(bytes: &[u8], tcp_port: u16) -> (usize, usize, usize) {
    let (mut at, mut tcp, mut udp, mut on_port) = (0, 0, 0, 0);
    while at < bytes.len() {
        let kind = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        if kind == 6 {
            // Enhanced packet block: 8 bytes of header, 20 of fixed fields.
            let cap = u32::from_le_bytes(bytes[at + 20..at + 24].try_into().unwrap()) as usize;
            let f = &bytes[at + 28..at + 28 + cap];
            let (proto, l4) = match [f[12], f[13]] {
                [0x08, 0x00] => (f[23], &f[34..]),
                [0x86, 0xdd] => (f[20], &f[54..]),
                other => panic!("ethertype {other:?}"),
            };
            match proto {
                6 => {
                    tcp += 1;
                    let src = u16::from_be_bytes([l4[0], l4[1]]);
                    let dst = u16::from_be_bytes([l4[2], l4[3]]);
                    if src == tcp_port || dst == tcp_port {
                        on_port += 1;
                    }
                }
                17 => udp += 1,
                other => panic!("protocol {other}"),
            }
        }
        at += len;
    }
    (tcp, udp, on_port)
}

/// The ring file size at which the HTTP sessions of the plan are too large and
/// the DNS sessions are not.
const LOSSY_FILE_BYTES: u64 = 1_500;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fps-agent-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fake_controller_changes_rates_and_reads_a_capture() {
    let dir = scratch("run-control");
    let mut h = Harness::start("agent-a", &dir).await;
    h.wait_state(AgentState::Idle).await;
    let plan = loopback_plan(83, 84, 18_280, 15_553, phases(1, 40));
    let id = plan.plan_id.clone();

    // Run time commands before the run fail with the codes of the contract.
    let r = h
        .command(update_rates(&id, vec![sps("web_get", 1.0)]))
        .await;
    assert_eq!(error_code(&r), Some(ErrorCode::FailedPrecondition), "{r:?}");
    let r = h.command(capture(start_capture(&id, "early"))).await;
    assert_eq!(error_code(&r), Some(ErrorCode::FailedPrecondition), "{r:?}");
    let r = h.command(stop_capture(&id, "early")).await;
    assert_eq!(error_code(&r), Some(ErrorCode::NotFound), "{r:?}");

    let r = h.command(prepare(&plan)).await;
    assert!(r.error.is_none(), "{r:?}");
    let r = h.command(start(&id)).await;
    assert!(r.error.is_none(), "{r:?}");
    h.wait_state(AgentState::Running).await;
    advance(&mut h, 3).await;

    // Every snapshot series carries the side label, and the snapshot holds
    // the client and the server side of the sessions that this agent runs.
    let snap = h.snapshots.last().expect("a snapshot");
    assert!(
        snap.counters.iter().all(|c| c.labels.contains_key("side")),
        "a series lacks the side label: {snap:?}"
    );
    for side in ["client", "server"] {
        assert!(
            by_scenario_on(snap, "sessions.attempted", "", side) > 0,
            "no {side} series with sessions: {snap:?}"
        );
    }

    // The compiled rates are 80 sessions per second for the HTTP entry on TCP
    // and 20 for the DNS entry on UDP, over two workers each.
    let web = rate_of(&mut h, "web_get", 4).await;
    let dns = rate_of(&mut h, "lookup", 4).await;
    assert!(web > 40.0 && web < 160.0, "web {web}");
    assert!(dns > 5.0 && dns < 50.0, "dns {dns}");

    // A command that breaks a rule applies none of its updates, the valid
    // ones included.
    let kind = pb::RateUpdate {
        mix_entry: "web_get".into(),
        target: Some(pb::rate_update::Target::ConcurrentSessions(4)),
    };
    let no_target = pb::RateUpdate {
        mix_entry: "web_get".into(),
        target: None,
    };
    for (rates, why) in [
        (
            vec![sps("dns_lookup", 400.0), sps("nope", 1.0)],
            "unknown entry",
        ),
        (
            vec![sps("dns_lookup", 400.0), sps("dns_lookup", 5.0)],
            "twice",
        ),
        (vec![sps("dns_lookup", 400.0), kind], "wrong kind"),
        (
            vec![sps("dns_lookup", 400.0), sps("web_get", -1.0)],
            "below zero",
        ),
        (vec![sps("dns_lookup", 400.0), no_target], "no target"),
    ] {
        let r = h.command(update_rates(&id, rates)).await;
        assert_eq!(
            error_code(&r),
            Some(ErrorCode::InvalidArgument),
            "{why}: {r:?}"
        );
    }
    let r = h
        .command(update_rates("run-0001/other", vec![sps("dns_lookup", 1.0)]))
        .await;
    assert_eq!(error_code(&r), Some(ErrorCode::NotFound), "{r:?}");
    let dns = rate_of(&mut h, "lookup", 4).await;
    assert!(dns < 50.0, "refused updates changed the rate: {dns}");

    // The mix entries are named by the ids of the plan. One update spans the
    // TCP registry and the UDP registry: DNS rises to 100 per second, and the
    // HTTP entry pauses.
    let r = h
        .command(update_rates(
            &id,
            vec![sps("dns_lookup", 100.0), sps("web_get", 0.0)],
        ))
        .await;
    assert!(
        matches!(r.result, Some(pb::command_result::Result::UpdateRates(ref u)) if u.applied_at.is_some()),
        "{r:?}"
    );
    advance(&mut h, 2).await;
    let web = rate_of(&mut h, "web_get", 4).await;
    let dns = rate_of(&mut h, "lookup", 4).await;
    assert!(web < 4.0, "the paused entry still starts sessions: {web}");
    assert!(dns > 60.0 && dns < 160.0, "dns {dns}");
    let r = h
        .command(update_rates(&id, vec![sps("web_get", 120.0)]))
        .await;
    assert!(r.error.is_none(), "{r:?}");
    advance(&mut h, 2).await;
    let web = rate_of(&mut h, "web_get", 4).await;
    let dns = rate_of(&mut h, "lookup", 4).await;
    assert!(web > 60.0, "the resumed entry did not start: {web}");
    assert!(
        dns > 60.0,
        "an entry that the update omits keeps its target: {dns}"
    );

    // What the agent cannot capture is refused, so it never records more than
    // the controller asked for. A port that is no port name and no server port
    // is refused as well (AGT-12).
    let with = |f: &dyn Fn(&mut pb::StartCaptureCommand)| {
        let mut c = start_capture(&id, "c");
        f(&mut c);
        c
    };
    for (cmd, code) in [
        (
            with(&|c| c.filter = "tcp port 80".into()),
            ErrorCode::InvalidArgument,
        ),
        (
            with(&|c| c.port = "nope".into()),
            ErrorCode::InvalidArgument,
        ),
        (
            with(&|c| c.port = "70000".into()),
            ErrorCode::InvalidArgument,
        ),
        (
            with(&|c| c.port = "90-80".into()),
            ErrorCode::InvalidArgument,
        ),
        (
            with(&|c| c.port = "80,,81".into()),
            ErrorCode::InvalidArgument,
        ),
        (with(&|c| c.snap_length = 96), ErrorCode::Unimplemented),
        (start_capture(&id, ""), ErrorCode::InvalidArgument),
        (start_capture(&id, "../escape"), ErrorCode::InvalidArgument),
        (start_capture("run-0001/other", "c"), ErrorCode::NotFound),
    ] {
        let r = h.command(capture(cmd.clone())).await;
        assert_eq!(error_code(&r), Some(code), "{cmd:?}: {r:?}");
    }

    // A capture that starts in the middle of the run reaches the workers.
    let r = h.command(capture(start_capture(&id, "cap-1"))).await;
    let Some(pb::command_result::Result::StartCapture(s)) = r.result else {
        panic!("start_capture result {r:?}")
    };
    assert_eq!(s.capture_id, "cap-1");
    assert!(s.started_at.is_some());
    for (cmd, why) in [
        (start_capture(&id, "cap-1"), "an id in use"),
        (start_capture(&id, "cap-2"), "a second capture at once"),
    ] {
        let r = h.command(capture(cmd)).await;
        assert_eq!(
            error_code(&r),
            Some(ErrorCode::FailedPrecondition),
            "{why}: {r:?}"
        );
    }
    advance(&mut h, 4).await;
    // A capture that still runs cannot be downloaded.
    let r = h.command(fetch(&id, "cap-1", 0, "t-run", 0)).await;
    assert_eq!(error_code(&r), Some(ErrorCode::FailedPrecondition), "{r:?}");
    let r = h.command(stop_capture(&id, "nope")).await;
    assert_eq!(error_code(&r), Some(ErrorCode::NotFound), "{r:?}");
    let r = h.command(stop_capture("run-0001/other", "cap-1")).await;
    assert_eq!(error_code(&r), Some(ErrorCode::NotFound), "{r:?}");
    // The capture stops at rest. A capture that stops under load counts the
    // sessions that are open at that moment as dropped, because they end after
    // the writer has gone (AGT-18), so the assertion on `dropped` below holds
    // only when no session is open. It still fails for a session that the
    // capture loses, as `a_capture_that_loses_sessions_reports_them_as_dropped`
    // shows.
    quiesce(&mut h, &id, &["web_get", "dns_lookup"]).await;
    let r = h.command(stop_capture(&id, "cap-1")).await;
    assert!(r.error.is_none(), "{r:?}");
    let first = stop_result(&r).clone();
    assert_eq!(first.capture_id, "cap-1");
    assert!(first.packets > 20, "{first:?}");
    assert_eq!(first.dropped, 0, "{first:?}");
    // The run goes on at the rates of the plan for the captures below.
    let r = h
        .command(update_rates(
            &id,
            vec![sps("web_get", 120.0), sps("dns_lookup", 100.0)],
        ))
        .await;
    assert!(r.error.is_none(), "{r:?}");

    // The fake controller locates the file below the capture directory, reads
    // it, and checks the digest, the size, and the packet count.
    let file = fps_agent::control::capture_path(&dir, &id, "cap-1");
    let bytes = std::fs::read(&file).expect("the capture file");
    assert_eq!(bytes.len() as u64, first.bytes);
    assert_eq!(sha256_digest(&bytes), first.digest);
    let (sections, packets) = read_pcapng(&bytes);
    assert!(sections >= 1);
    assert_eq!(packets as u64, first.packets);
    // One capture holds the TCP sessions of the HTTP entry and the UDP
    // sessions of the DNS entry (AGT-10): both read the same cell.
    let (tcp, udp, on_port) = transports(&bytes, 18_280);
    assert!(tcp > 10, "the capture holds {tcp} TCP packets");
    assert!(udp > 2, "the capture holds {udp} UDP packets");
    assert_eq!(tcp, on_port, "every TCP packet belongs to the HTTP entry");
    // No ring directory is left beside the file.
    let leftovers: Vec<_> = std::fs::read_dir(file.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, ["cap-1.pcapng"]);

    // The command is idempotent.
    let r = h.command(stop_capture(&id, "cap-1")).await;
    assert_eq!(stop_result(&r), &first);

    // A capture ends at its time limit on its own and still answers.
    let mut limited = start_capture(&id, "cap-2");
    limited.max_duration = Some(prost_types::Duration {
        seconds: 1,
        nanos: 0,
    });
    let r = h.command(capture(limited)).await;
    assert!(r.error.is_none(), "{r:?}");
    advance(&mut h, 6).await;
    let limited = fps_agent::control::capture_path(&dir, &id, "cap-2");
    assert!(limited.exists(), "the time limit did not end the capture");
    let r = h.command(stop_capture(&id, "cap-2")).await;
    let second = stop_result(&r).clone();
    assert_eq!(
        sha256_digest(&std::fs::read(&limited).unwrap()),
        second.digest
    );

    // A capture that is still running when the run ends is closed with it.
    let r = h.command(capture(start_capture(&id, "cap-3"))).await;
    assert!(r.error.is_none(), "{r:?}");
    advance(&mut h, 2).await;
    let r = h.command(stop(&id, pb::StopMode::RampDown, 2)).await;
    assert!(r.error.is_none(), "{r:?}");
    h.wait_state(AgentState::Stopped).await;
    let r = h.command(stop_capture(&id, "cap-3")).await;
    let third = stop_result(&r).clone();
    assert!(third.packets > 0, "{third:?}");
    let bytes = std::fs::read(fps_agent::control::capture_path(&dir, &id, "cap-3")).unwrap();
    assert_eq!(sha256_digest(&bytes), third.digest);

    // The rates and the captures are for a running plan only.
    let r = h
        .command(update_rates(&id, vec![sps("web_get", 1.0)]))
        .await;
    assert_eq!(error_code(&r), Some(ErrorCode::FailedPrecondition), "{r:?}");
    let r = h.command(capture(start_capture(&id, "late"))).await;
    assert_eq!(error_code(&r), Some(ErrorCode::FailedPrecondition), "{r:?}");

    // The capture download of CON-13. The refusals come first. Every one of
    // them answers at once and starts no upload.
    let cap3 = std::fs::read(fps_agent::control::capture_path(&dir, &id, "cap-3")).unwrap();
    let size = cap3.len() as u64;
    for (cmd, code, why) in [
        (
            fetch(&id, "nope", 0, "t-x", 0),
            ErrorCode::NotFound,
            "an unknown capture",
        ),
        (
            fetch("run-0001/other", "cap-3", 0, "t-x", 0),
            ErrorCode::NotFound,
            "an unknown plan",
        ),
        (
            fetch(&id, "cap-3", size + 1, "t-x", 0),
            ErrorCode::InvalidArgument,
            "an offset beyond the size",
        ),
        (
            fetch(&id, "cap-3", 0, "", 0),
            ErrorCode::InvalidArgument,
            "no transfer id",
        ),
        (
            fetch(&id, "cap-3", 0, "has space", 0),
            ErrorCode::InvalidArgument,
            "a bad transfer id",
        ),
        (
            fetch(&id, "cap-3", 0, &"t".repeat(129), 0),
            ErrorCode::InvalidArgument,
            "a long transfer id",
        ),
    ] {
        let r = h.command(cmd).await;
        assert_eq!(error_code(&r), Some(code), "{why}: {r:?}");
    }
    assert!(
        h.shared.uploads.lock().unwrap().is_empty(),
        "a refusal started an upload"
    );

    // The controller loses its connection after the first chunks, and the
    // download resumes at the offset that it holds with a new transfer id.
    *h.shared.cut_upload_after.lock().unwrap() = Some(size / 3);
    let r = h.command(fetch(&id, "cap-3", 0, "t-1", 64)).await;
    let Some(pb::command_result::Result::FetchCapture(f)) = r.result else {
        panic!("fetch_capture result {r:?}")
    };
    assert_eq!(f.capture_id, "cap-3");
    assert_eq!(f.transfer_id, "t-1");
    assert_eq!(f.total_bytes, third.bytes);
    assert_eq!(f.digest, third.digest);
    assert_eq!(f.chunk_bytes, 64, "the agent honors a smaller chunk size");
    let first_part = wait_upload(&mut h, 1).await;
    assert!(first_part.cut);
    let open = first_part.open.as_ref().expect("an open message");
    assert_eq!(
        (
            open.transfer_id.as_str(),
            open.capture_id.as_str(),
            open.start_offset
        ),
        ("t-1", "cap-3", 0)
    );
    let mut held: Vec<u8> = first_part.chunks.iter().flat_map(|c| c.1.clone()).collect();
    assert!(
        !held.is_empty() && (held.len() as u64) < size,
        "{} of {size}",
        held.len()
    );
    assert_eq!(held, cap3[..held.len()]);
    assert!(first_part.chunks.iter().all(|c| c.1.len() <= 64));

    *h.shared.cut_upload_after.lock().unwrap() = None;
    let resume_at = held.len() as u64;
    // The slot of the broken transfer is free as soon as its task ended, which
    // a controller that resumes at once may beat by a moment.
    let deadline = Instant::now() + WAIT;
    let f = loop {
        let r = h.command(fetch(&id, "cap-3", resume_at, "t-2", 0)).await;
        match error_code(&r) {
            None => {
                let Some(pb::command_result::Result::FetchCapture(f)) = r.result else {
                    panic!("fetch_capture result {r:?}")
                };
                break f;
            }
            Some(ErrorCode::ResourceExhausted) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            _ => panic!("the resumed fetch_capture failed: {r:?}"),
        }
    };
    assert_eq!(
        (f.total_bytes, f.digest.as_str()),
        (third.bytes, third.digest.as_str())
    );
    assert_eq!(
        f.chunk_bytes, 262_144,
        "zero asks for the default chunk size"
    );
    let second_part = wait_upload(&mut h, 2).await;
    assert!(!second_part.cut);
    let open = second_part.open.as_ref().expect("an open message");
    assert_eq!(
        (open.transfer_id.as_str(), open.start_offset),
        ("t-2", resume_at)
    );
    assert_eq!(
        second_part.chunks[0].0, resume_at,
        "the first chunk starts at the offset"
    );
    held.extend(second_part.chunks.iter().flat_map(|c| c.1.clone()));
    let end = second_part.end.as_ref().expect("an end message");
    assert_eq!(end.total_bytes, third.bytes);
    assert_eq!(
        end.digest, third.digest,
        "the end digest covers the whole file"
    );
    assert_eq!(held, cap3, "the two parts join into the file");
    assert_eq!(sha256_digest(&held), third.digest);

    // An offset equal to the size sends no chunk and still ends the call.
    let deadline = Instant::now() + WAIT;
    loop {
        let r = h.command(fetch(&id, "cap-3", size, "t-3", 0)).await;
        match error_code(&r) {
            None => break,
            Some(ErrorCode::ResourceExhausted) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            _ => panic!("fetch at the end of the file failed: {r:?}"),
        }
    }
    let empty = wait_upload(&mut h, 3).await;
    assert!(empty.chunks.is_empty());
    assert_eq!(empty.end.as_ref().map(|e| e.total_bytes), Some(size));
    h.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// Returns the TCP packets, the UDP packets, and the packets of the TCP port
/// `tcp_port` of the capture `id`, after the stop result and the file agree.
async fn captured(
    h: &mut Harness,
    dir: &Path,
    plan: &str,
    id: &str,
    port: &str,
    tcp_port: u16,
) -> (usize, usize, usize) {
    let mut cmd = start_capture(plan, id);
    cmd.port = port.into();
    let r = h.command(capture(cmd)).await;
    assert!(r.error.is_none(), "port {port:?}: {r:?}");
    advance(h, 4).await;
    let r = h.command(stop_capture(plan, id)).await;
    assert!(r.error.is_none(), "port {port:?}: {r:?}");
    let done = stop_result(&r).clone();
    let bytes = std::fs::read(fps_agent::control::capture_path(dir, plan, id)).unwrap();
    assert_eq!(bytes.len() as u64, done.bytes, "port {port:?}");
    assert_eq!(sha256_digest(&bytes), done.digest, "port {port:?}");
    let (sections, packets) = read_pcapng(&bytes);
    assert!(sections >= 1, "port {port:?}");
    assert_eq!(packets as u64, done.packets, "port {port:?}");
    let (tcp, udp, on_port) = transports(&bytes, tcp_port);
    assert_eq!(tcp + udp, packets, "port {port:?}");
    (tcp, udp, on_port)
}

/// AGT-12: a capture with a port keeps the sessions of that port, for TCP and
/// for UDP, and an empty port keeps all of them.
/// AGT-18: the `dropped` of a capture is not always zero. A ring file that is
/// too small for the larger sessions loses them, and the stop result says so,
/// with no session open at the stop. The test pairs with the zero that
/// `a_fake_controller_changes_rates_and_reads_a_capture` asserts at rest, so
/// that assertion fails when the capture really loses a session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_capture_that_loses_sessions_reports_them_as_dropped() {
    let dir = scratch("dropped");
    let mut h = Harness::start_with("agent-a", &dir, |cfg| {
        // The HTTP sessions do not fit one file, and the DNS sessions do.
        cfg.captures.file_bytes = LOSSY_FILE_BYTES;
    })
    .await;
    h.wait_state(AgentState::Idle).await;
    let plan = loopback_plan(89, 90, 18_284, 15_556, phases(1, 60));
    let id = plan.plan_id.clone();
    let r = h.command(prepare(&plan)).await;
    assert!(r.error.is_none(), "{r:?}");
    let r = h.command(start(&id)).await;
    assert!(r.error.is_none(), "{r:?}");
    h.wait_state(AgentState::Running).await;
    advance(&mut h, 3).await;

    let r = h.command(capture(start_capture(&id, "lossy"))).await;
    assert!(r.error.is_none(), "{r:?}");
    advance(&mut h, 4).await;
    quiesce(&mut h, &id, &["web_get", "dns_lookup"]).await;
    let r = h.command(stop_capture(&id, "lossy")).await;
    assert!(r.error.is_none(), "{r:?}");
    let lossy = stop_result(&r).clone();
    assert!(lossy.dropped > 0, "no drop was reported: {lossy:?}");
    assert!(lossy.packets > 0, "the DNS sessions fit: {lossy:?}");

    let r = h.command(stop(&id, pb::StopMode::RampDown, 2)).await;
    assert!(r.error.is_none(), "{r:?}");
    h.wait_state(AgentState::Stopped).await;
    h.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_capture_with_a_port_skips_the_sessions_of_other_ports() {
    let dir = scratch("port-filter");
    let mut h = Harness::start("agent-a", &dir).await;
    h.wait_state(AgentState::Idle).await;
    let (http, dns) = (18_283, 15_555);
    let plan = loopback_plan(87, 88, http, dns, phases(1, 60));
    let id = plan.plan_id.clone();
    let r = h.command(prepare(&plan)).await;
    assert!(r.error.is_none(), "{r:?}");
    let r = h.command(start(&id)).await;
    assert!(r.error.is_none(), "{r:?}");
    h.wait_state(AgentState::Running).await;
    advance(&mut h, 3).await;

    // Every session first: the run has TCP sessions of the HTTP entry and UDP
    // sessions of the DNS entry.
    let (tcp, udp, on_port) = captured(&mut h, &dir, &id, "all", "", http).await;
    assert!(tcp > 10 && udp > 2, "tcp {tcp}, udp {udp}");
    assert_eq!(tcp, on_port);

    // A TCP server port skips every UDP session, and the UDP port every TCP one.
    let (tcp, udp, on_port) = captured(&mut h, &dir, &id, "tcp", &http.to_string(), http).await;
    assert!(tcp > 10, "the TCP port kept {tcp} TCP packets");
    assert_eq!((udp, on_port), (0, tcp), "the TCP port kept UDP sessions");
    let (tcp, udp, _) = captured(&mut h, &dir, &id, "udp", &dns.to_string(), http).await;
    assert!(udp > 2, "the UDP port kept {udp} UDP packets");
    assert_eq!(tcp, 0, "the UDP port kept TCP sessions");

    // A list and a range name ports, and a port with no session keeps nothing.
    let list = format!("{http},{dns}");
    let (tcp, udp, _) = captured(&mut h, &dir, &id, "list", &list, http).await;
    assert!(tcp > 10 && udp > 2, "tcp {tcp}, udp {udp}");
    let range = format!("{}-{}", dns - 5, dns + 5);
    let (tcp, udp, _) = captured(&mut h, &dir, &id, "range", &range, http).await;
    assert!(udp > 2 && tcp == 0, "tcp {tcp}, udp {udp}");
    let (tcp, udp, _) = captured(&mut h, &dir, &id, "none", "9", http).await;
    assert_eq!((tcp, udp), (0, 0), "port 9 has no session");

    // The names of the ports of the plan select the sessions of their pools.
    for name in ["client0", "server0"] {
        let (tcp, udp, _) = captured(&mut h, &dir, &id, name, name, http).await;
        assert!(tcp > 10 && udp > 2, "{name}: tcp {tcp}, udp {udp}");
    }

    let r = h.command(stop(&id, pb::StopMode::RampDown, 2)).await;
    assert!(r.error.is_none(), "{r:?}");
    h.wait_state(AgentState::Stopped).await;
    h.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}
