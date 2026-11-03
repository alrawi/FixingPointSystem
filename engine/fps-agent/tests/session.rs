// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of the controller session against an in process fake
//! controller.
//!
//! The fake serves the generated `ControllerService` stub on a loopback
//! port. It hands every message of the agent to the test and lets the test
//! send commands, end the stream, or fall silent. The plans derive from
//! `contracts/examples/plans/http_dns_smoke.json` with loopback pools and
//! ports above 1024, as in `tests/standalone.rs`.

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
    async fn start(agent_id: &str) -> Self {
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
        cfg.capabilities.engine_api = "0.1.0".into();
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

    /// Ends the current session with a status, as a controller restart does.
    fn drop_session(&self) {
        let tx = self
            .shared
            .outbound
            .lock()
            .unwrap()
            .take()
            .expect("a session");
        let _ = tx.try_send(Err(Status::unavailable("the controller shuts down")));
    }

    fn registrations(&self) -> Vec<pb::RegisterRequest> {
        self.shared.registrations.lock().unwrap().clone()
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

fn count(snap: &pb::CounterSnapshot, metric: &str) -> u64 {
    snap.counters
        .iter()
        .filter(|c| c.metric == metric)
        .map(|c| match c.value {
            Some(pb::counter_value::Value::Count(n)) => n,
            _ => 0,
        })
        .sum()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fake_controller_drives_prepare_start_and_stop_and_receives_snapshots() {
    let mut h = Harness::start("agent-a").await;
    let hb = h.wait_state(AgentState::Idle).await;
    assert_eq!(hb.plan_id, "");
    let reg = &h.registrations()[0];
    assert_eq!(reg.agent_id, "agent-a");
    assert_eq!(reg.state, AgentState::Idle as i32);

    let plan = loopback_plan(81, 82, 18_180, 15_453, phases(2, 6));
    let id = plan.plan_id.clone();

    // Out of order calls fail with the codes of the contract.
    let r = h.command(start(&id)).await;
    assert_eq!(error_code(&r), Some(ErrorCode::FailedPrecondition), "{r:?}");
    let r = h.command(stop(&id, pb::StopMode::RampDown, 1)).await;
    assert_eq!(error_code(&r), Some(ErrorCode::FailedPrecondition), "{r:?}");

    let r = h.command(prepare(&plan)).await;
    assert!(r.error.is_none(), "{r:?}");
    let Some(pb::command_result::Result::Prepare(p)) = r.result else {
        panic!("prepare result {r:?}")
    };
    assert_eq!(p.plan_id, id);
    assert_eq!(p.state, AgentState::Prepared as i32);
    h.wait_state(AgentState::Prepared).await;

    let r = h.command(prepare(&plan)).await;
    assert_eq!(error_code(&r), Some(ErrorCode::FailedPrecondition), "{r:?}");
    let r = h.command(start("run-0001/other")).await;
    assert_eq!(error_code(&r), Some(ErrorCode::NotFound), "{r:?}");

    let r = h.command(start(&id)).await;
    assert!(
        matches!(r.result, Some(pb::command_result::Result::Start(_))),
        "{r:?}"
    );
    h.wait_state(AgentState::Running).await;
    while h.snapshots.len() < 3 {
        h.recv().await;
    }

    let r = h.command(stop(&id, pb::StopMode::RampDown, 2)).await;
    assert!(r.error.is_none(), "{r:?}");
    let Some(pb::command_result::Result::Stop(s)) = r.result else {
        panic!("stop result {r:?}")
    };
    assert_eq!(s.state, AgentState::Stopped as i32);
    let last = s.final_snapshot.expect("a final snapshot");
    assert!(last.is_final);
    assert_eq!(last.plan_id, id);
    assert!(h
        .heartbeats
        .iter()
        .any(|b| b.state == AgentState::Stopping as i32));
    h.wait_state(AgentState::Stopped).await;

    // Snapshots are aggregate, cumulative, and numbered from 1 without gaps.
    assert!(h.snapshots.len() >= 4, "{} snapshots", h.snapshots.len());
    for (i, snap) in h.snapshots.iter().enumerate() {
        assert_eq!(snap.sequence, i as u64 + 1);
        assert_eq!(snap.plan_id, id);
        assert!(snap.aggregate);
    }
    let fin = h.snapshots.last().unwrap();
    assert!(fin.is_final);
    assert_eq!(fin.sequence, last.sequence);
    let attempted = count(fin, "sessions.attempted");
    let succeeded = count(fin, "sessions.succeeded");
    assert!(
        succeeded > 10,
        "attempted {attempted} succeeded {succeeded}"
    );
    let mut prev = 0;
    for snap in &h.snapshots {
        let n = count(snap, "sessions.attempted");
        assert!(n >= prev, "cumulative counters");
        prev = n;
    }
    let names: Vec<(&str, u64)> = fin
        .histograms
        .iter()
        .map(|x| (x.metric.as_str(), x.count))
        .collect();
    assert!(
        fin.histograms
            .iter()
            .any(|x| x.count > 0 && !x.hdr_v2.is_empty()),
        "{names:?}"
    );

    // A repeated stop reports the same final snapshot.
    let r = h.command(stop(&id, pb::StopMode::Abort, 0)).await;
    let Some(pb::command_result::Result::Stop(s)) = r.result else {
        panic!("stop result {r:?}")
    };
    assert_eq!(s.final_snapshot.map(|f| f.sequence), Some(last.sequence));

    // A stopped agent prepares again, and stop releases an unstarted plan.
    let r = h.command(prepare(&plan)).await;
    assert!(r.error.is_none(), "{r:?}");
    let r = h.command(stop(&id, pb::StopMode::RampDown, 1)).await;
    let Some(pb::command_result::Result::Stop(s)) = r.result else {
        panic!("stop result {r:?}")
    };
    assert_eq!(s.state, AgentState::Idle as i32);
    assert!(s.final_snapshot.is_none());
    h.wait_state(AgentState::Idle).await;
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_plan_fails_prepare_and_leaves_the_state_unchanged() {
    let mut h = Harness::start("agent-a").await;
    h.wait_state(AgentState::Idle).await;
    let mut plan = loopback_plan(83, 84, 18_280, 15_553, phases(1, 1));
    plan.modules[0].digest = format!("sha256:{}", "0".repeat(64));
    let r = h.command(prepare(&plan)).await;
    assert_eq!(error_code(&r), Some(ErrorCode::FailedPrecondition), "{r:?}");

    let mut plan = loopback_plan(83, 84, 18_280, 15_553, phases(1, 1));
    plan.agent_id = "agent-b".into();
    let r = h.command(prepare(&plan)).await;
    assert_eq!(error_code(&r), Some(ErrorCode::InvalidArgument), "{r:?}");

    let mut plan = loopback_plan(83, 84, 18_280, 15_553, phases(1, 1));
    plan.backend = pb::Backend::Dpdk as i32;
    let r = h.command(prepare(&plan)).await;
    assert_eq!(error_code(&r), Some(ErrorCode::Unimplemented), "{r:?}");

    let hb = h.wait_state(AgentState::Idle).await;
    assert_eq!(hb.plan_id, "");
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_continues_across_a_reconnection_and_stops_on_its_own() {
    let mut h = Harness::start("agent-a").await;
    h.wait_state(AgentState::Idle).await;
    let plan = loopback_plan(85, 86, 18_380, 15_653, phases(1, 3));
    let id = plan.plan_id.clone();
    assert!(h.command(prepare(&plan)).await.error.is_none());
    assert!(h.command(start(&id)).await.error.is_none());
    while h.snapshots.len() < 2 {
        h.recv().await;
    }
    let before = h.snapshots.last().unwrap().sequence;

    h.drop_session();
    let deadline = Instant::now() + WAIT;
    while h.registrations().len() < 2 {
        assert!(Instant::now() < deadline, "no second registration");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let reg = h.registrations()[1].clone();
    assert_eq!(reg.state, AgentState::Running as i32, "{reg:?}");
    assert_eq!(reg.plan_id, id);

    // The run ends on its own with a final snapshot and a STOPPED heartbeat
    // on the second session.
    let deadline = Instant::now() + WAIT;
    loop {
        assert!(Instant::now() < deadline, "no final snapshot");
        if let (n, session_request::Body::Snapshot(s)) = h.recv().await {
            if s.is_final {
                assert_eq!(n, 2);
                assert!(s.sequence > before);
                break;
            }
        }
    }
    h.wait_state(AgentState::Stopped).await;
    let seqs: Vec<u64> = h.snapshots.iter().map(|s| s.sequence).collect();
    assert!(seqs.windows(2).all(|w| w[1] > w[0]), "{seqs:?}");
    h.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_silent_controller_makes_the_agent_abort_and_release_the_plan() {
    let mut h = Harness::start("agent-a").await;
    h.wait_state(AgentState::Idle).await;
    let plan = loopback_plan(87, 88, 18_480, 15_753, phases(1, 30));
    let id = plan.plan_id.clone();
    assert!(h.command(prepare(&plan)).await.error.is_none());
    assert!(h.command(start(&id)).await.error.is_none());
    h.wait_state(AgentState::Running).await;

    let silent_at = Instant::now();
    h.shared.silent.store(true, Ordering::SeqCst);
    let deadline = Instant::now() + WAIT;
    while h.registrations().len() < 2 {
        assert!(
            Instant::now() < deadline,
            "no registration after the silence"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // The heartbeat timeout of the fake is one second.
    assert!(silent_at.elapsed() >= Duration::from_millis(900));
    h.shared.silent.store(false, Ordering::SeqCst);
    let reg = h.registrations()[1].clone();
    assert_eq!(reg.state, AgentState::Idle as i32, "{reg:?}");
    assert_eq!(reg.plan_id, "");
    let hb = h.wait_state(AgentState::Idle).await;
    assert_eq!(hb.plan_id, "");
    h.shutdown().await;
}
