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
        _req: Request<Streaming<pb::UploadCaptureRequest>>,
    ) -> Result<Response<pb::UploadCaptureResponse>, Status> {
        Err(Status::unimplemented(
            "the fake controller takes no uploads",
        ))
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

fn error_code(r: &pb::CommandResult) -> Option<ErrorCode> {
    r.error
        .as_ref()
        .map(|e| ErrorCode::try_from(e.code).expect("a known code"))
}

/// Sums the counters of `metric` whose `scenario` label contains `scenario`.
fn by_scenario(snap: &pb::CounterSnapshot, metric: &str, scenario: &str) -> u64 {
    snap.counters
        .iter()
        .filter(|c| {
            c.metric == metric
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
    // the controller asked for.
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
            with(&|c| c.port = "client0".into()),
            ErrorCode::Unimplemented,
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
    let r = h.command(stop_capture(&id, "nope")).await;
    assert_eq!(error_code(&r), Some(ErrorCode::NotFound), "{r:?}");
    let r = h.command(stop_capture("run-0001/other", "cap-1")).await;
    assert_eq!(error_code(&r), Some(ErrorCode::NotFound), "{r:?}");
    let r = h.command(stop_capture(&id, "cap-1")).await;
    assert!(r.error.is_none(), "{r:?}");
    let first = stop_result(&r).clone();
    assert_eq!(first.capture_id, "cap-1");
    assert!(first.packets > 20, "{first:?}");
    assert_eq!(first.dropped, 0, "{first:?}");

    // The fake controller locates the file below the capture directory, reads
    // it, and checks the digest, the size, and the packet count.
    let file = fps_agent::control::capture_path(&dir, &id, "cap-1");
    let bytes = std::fs::read(&file).expect("the capture file");
    assert_eq!(bytes.len() as u64, first.bytes);
    assert_eq!(sha256_digest(&bytes), first.digest);
    let (sections, packets) = read_pcapng(&bytes);
    assert!(sections >= 1);
    assert_eq!(packets as u64, first.packets);
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

    // The capture download of CON-13 is not served until AGT-09.
    let r = h
        .command(session_response::Body::FetchCapture(
            pb::FetchCaptureCommand {
                plan_id: id.clone(),
                capture_id: "cap-3".into(),
                transfer_id: "t-1".into(),
                ..Default::default()
            },
        ))
        .await;
    assert_eq!(error_code(&r), Some(ErrorCode::Unimplemented), "{r:?}");
    h.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);
}
