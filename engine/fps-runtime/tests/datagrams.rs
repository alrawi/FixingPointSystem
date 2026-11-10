// SPDX-License-Identifier: GPL-3.0-only
//! Datagram flows in scenario instances on the worker runtime (ENG-26).
//!
//! The acceptance test runs `call_pcmu` of the sip_rtp module, a SIP dialog
//! over UDP with a spawned RTP flow, through a client runtime and a server
//! runtime over loopback. The server worker listens on a UDP port for the
//! entry flow, binds the RTP port when its media flow starts, and the client
//! dials every flow from a socket of its own. A capture of the client sessions
//! shows the SIP messages and the RTP packets. The second test makes every
//! call draw the same RTP port, so the calls that overlap share one socket.

#[path = "../../fps-capture/tests/common/mod.rs"]
mod capture_common;

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use capture_common::{fresh_dir, parse_frame, read_pcapng, StepClock};
use fps_backend_kernel::{ConnectConfig, ListenConfig};
use fps_capture::{Capture, CaptureConfig, RingConfig, SamplePolicy};
use fps_flow::{Manifest, ModuleSet, Scenario, Value};
use fps_hosts::{HostPool, HostSelector, PoolSpec, Selection};
use fps_module_sip_rtp::SipRtp;
use fps_runtime::{
    ClientConfig, FlowRef, FunctionFactory, Role, RunReport, Runtime, RuntimeConfig, RuntimeHandle,
    ServerConfig, SourceHosts, WorkerConfig,
};
use fps_sched::{stream_id, Arrival, LoadProfile, Objective, Phase, ScheduleConfig, Shape};
use fps_stats::{metric, Snapshot};

const SEED: u64 = 20_261_112;

/// The sessions of one call on one side: the dialog and the media flow.
const FLOW_SESSIONS: u64 = 2;
/// The RTP packets that each side sends in one call: `talk_ms` over `ptime_ms`.
const RTP_PER_SIDE: u64 = 20;
/// The size of an RTP packet of PCMU at 20 ms: the 12 byte header and 160 bytes.
const RTP_BYTES: usize = 172;

fn v4(a: u8, b: u8, c: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, a, b, c))
}

/// Every flow side of a call has its own `SipRtp`, because the value holds the
/// sequence number, the SSRC, and the pacing state of one RTP stream.
fn sip_functions() -> FunctionFactory {
    Arc::new(|| Box::new(SipRtp::default()))
}

/// Loads `call_pcmu` as the module test does, with a short call: 100 ms of
/// ringing and 400 ms of talk, which is 20 packets of 20 ms each way.
fn call_scenario(server_ip: IpAddr, client_ip: IpAddr, rtp: (i64, i64)) -> Arc<Scenario> {
    let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/sip_rtp");
    let mut set = ModuleSet::default();
    set.insert(Manifest::load(&module.join("manifest.yaml")).unwrap());
    let ctx = set.context("sip_rtp").unwrap();
    let path = module.join("scenarios/call_pcmu.yaml");
    let mut s = Scenario::load_instance(&path, Some(&ctx)).unwrap();
    s.override_vars(&[
        ("server_ip".to_owned(), Value::Str(server_ip.to_string())),
        ("client_ip".to_owned(), Value::Str(client_ip.to_string())),
        ("rtp_port_min".to_owned(), Value::Int(rtp.0)),
        ("rtp_port_max".to_owned(), Value::Int(rtp.1)),
        ("ring_ms".to_owned(), Value::Int(100)),
        ("talk_ms".to_owned(), Value::Int(400)),
    ])
    .unwrap();
    s.fold_constants().unwrap();
    Arc::new(s)
}

fn flow_ref(scenario: &Arc<Scenario>) -> FlowRef {
    let mut f = FlowRef::new(Arc::clone(scenario), 0);
    f.functions = sip_functions();
    f
}

fn server_runtime(scenario: &Arc<Scenario>, ip: IpAddr) -> RuntimeHandle {
    let worker = WorkerConfig {
        id: 10,
        core: None,
        role: Role::Server(ServerConfig {
            flow: flow_ref(scenario),
            listen: vec![SocketAddr::new(ip, 0)],
            config: ListenConfig::default(),
        }),
    };
    Runtime::start(RuntimeConfig::new(SEED, vec![worker])).unwrap()
}

/// One client worker that starts `rate` calls per second for one second from
/// sources in `127.<pool>.0.0/24`, one source host per call.
fn client_config(
    scenario: &Arc<Scenario>,
    server: SocketAddr,
    pool: u8,
    rate: f64,
) -> RuntimeConfig {
    let prefix = format!("127.{pool}.0.0/24");
    let hosts = HostPool::parse(&PoolSpec {
        name: "clients",
        ipv4: Some(&prefix),
        count: 64,
        ..PoolSpec::default()
    })
    .unwrap();
    let range = hosts.range();
    let profile = LoadProfile::new(vec![Phase::new(
        "steady",
        Duration::from_secs(1),
        Shape::Hold,
        1.0,
        1.0,
    )])
    .unwrap();
    let mut schedule = ScheduleConfig::new(Objective::SessionsPerSecond(rate), profile, SEED);
    schedule.arrival = Arrival::Constant;
    schedule.stream = stream_id(0, 0);
    let worker = WorkerConfig {
        id: 0,
        core: None,
        role: Role::Client(Box::new(ClientConfig {
            flow: flow_ref(scenario),
            schedule,
            servers: vec![server.ip()],
            port: Some(server.port()),
            sources: SourceHosts {
                selector: HostSelector::new("clients", range, Selection::RoundRobin, SEED, 0)
                    .unwrap(),
                pool: hosts,
                ipv6: false,
            },
            connect: ConnectConfig::default(),
        })),
    };
    let mut config = RuntimeConfig::new(SEED, vec![worker]);
    config.snapshot_interval = Duration::from_millis(100);
    config
}

fn run_pair(server: RuntimeHandle, client: RuntimeConfig) -> (RunReport, RunReport) {
    let client = Runtime::start(client).unwrap().join();
    server.stop(Duration::from_secs(2));
    let server = server.join();
    for report in [&client, &server] {
        for (id, r) in &report.workers {
            assert!(r.is_ok(), "worker {id}: {r:?}");
        }
    }
    (client, server)
}

fn exit(report: &RunReport, worker: u32) -> &Snapshot {
    &report.workers[&worker].as_ref().unwrap().sessions
}

fn start_capture(name: &str) -> Capture {
    let dir = fresh_dir(name);
    let mut ring = RingConfig::new(&dir);
    ring.prefix = "dgram".into();
    let mut config = CaptureConfig::new(ring);
    config.sample = SamplePolicy::EveryNth(1);
    config.seed = SEED;
    Capture::start(config, Arc::new(StepClock::default())).unwrap()
}

/// Runs `calls` calls through a server and a client runtime and checks what
/// the counters and the capture of the client show. It returns the number of
/// distinct RTP ports that the calls used.
fn run_calls(name: &str, rtp: (i64, i64), pool: u8, calls: u64) -> usize {
    let (server_ip, client_ip) = (v4(pool, 0, 1), v4(pool, 1, 1));
    let scenario = call_scenario(server_ip, client_ip, rtp);
    assert_eq!(scenario.flows.len(), 2);
    let server = server_runtime(&scenario, server_ip);
    let sip = server.listen_addrs()[0];
    assert_eq!(sip.ip(), server_ip);
    let config = client_config(&scenario, sip, pool + 1, calls as f64);
    let capture = start_capture(name);
    config.capture.set(capture.clone());
    let (client, server) = run_pair(server, config);

    // The counters: every call started the dialog and the media flow on both
    // sides, and every side succeeded.
    let sessions = calls * FLOW_SESSIONS;
    for (report, worker, side) in [(&client, 0, "client"), (&server, 10, "server")] {
        let s = exit(report, worker);
        assert_eq!(
            s.counter_total(metric::SESSIONS_ATTEMPTED),
            sessions,
            "{side}"
        );
        assert_eq!(
            s.counter_total(metric::SESSIONS_SUCCEEDED),
            sessions,
            "{side}"
        );
        assert_eq!(s.counter_total(metric::SESSIONS_FAILED), 0, "{side}");
    }
    // The datagrams that the client sent arrived at the server and the other
    // way round, so the transports lost nothing.
    let (tx, rx) = (
        exit(&client, 0).counter_total(metric::BYTES_TX),
        exit(&server, 10).counter_total(metric::BYTES_RX),
    );
    assert_eq!(tx, rx, "what the client sent, the server received");
    assert!(
        tx > calls * RTP_PER_SIDE * RTP_BYTES as u64,
        "{tx} bytes sent"
    );

    // The capture: the client sessions, SIP and RTP, as UDP frames. Every
    // flow of every call dialed from a source port of its own.
    let files = capture.flush();
    let stats = capture.stop();
    assert_eq!(stats.sessions_dropped_queue, 0, "{stats:?}");
    assert_eq!(stats.sessions_written, sessions, "{stats:?}");
    assert_eq!(files.len(), 1, "{files:?}");
    let packets: Vec<_> = read_pcapng(&files[0])
        .packets
        .iter()
        .map(|(ts, f)| parse_frame(*ts, f))
        .collect();
    assert!(packets.iter().all(|p| p.udp), "every frame is UDP");
    let flows: BTreeSet<_> = packets
        .iter()
        .filter(|p| p.dst == server_ip)
        .map(|p| (p.src, p.sport, p.dport))
        .collect();
    assert_eq!(
        flows.len() as u64,
        sessions,
        "one 5-tuple per flow: {flows:?}"
    );
    let sip_port = sip.port();
    let rtp_from_client: Vec<_> = packets
        .iter()
        .filter(|p| p.dst == server_ip && p.dport != sip_port)
        .collect();
    let rtp_to_client: Vec<_> = packets
        .iter()
        .filter(|p| p.src == server_ip && p.sport != sip_port)
        .collect();
    for rtp in [&rtp_from_client, &rtp_to_client] {
        assert!(
            rtp.iter()
                .all(|p| p.payload.len() == RTP_BYTES && p.payload[0] >> 6 == 2),
            "RTP version 2 packets of {RTP_BYTES} bytes"
        );
        assert_eq!(
            rtp.len() as u64,
            calls * RTP_PER_SIDE,
            "20 packets each way per call"
        );
    }
    let sip_packets = packets
        .iter()
        .filter(|p| p.dport == sip_port || p.sport == sip_port)
        .filter(|p| p.payload.starts_with(b"INVITE ") || p.payload.starts_with(b"SIP/2.0 200 "))
        .count() as u64;
    assert!(
        sip_packets >= 2 * calls,
        "INVITE and 200 OK of every call: {sip_packets}"
    );
    rtp_from_client
        .iter()
        .map(|p| p.dport)
        .collect::<BTreeSet<_>>()
        .len()
}

/// The acceptance test: SIP calls with RTP run through the instance runner.
#[test]
fn a_sip_call_with_rtp_runs_through_the_instance_runner() {
    let ports = run_calls("datagrams-sip", (18_000, 18_998), 80, 4);
    assert!(ports >= 1);
}

/// Every call draws the same RTP port, so the calls that overlap share the
/// socket of the server and each takes the datagrams of its own host.
#[test]
fn concurrent_calls_that_draw_the_same_rtp_port_all_succeed() {
    let ports = run_calls("datagrams-same-port", (18_500, 18_500), 84, 6);
    assert_eq!(ports, 1, "one RTP port served every call");
}
