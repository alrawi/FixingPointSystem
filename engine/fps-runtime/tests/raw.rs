// SPDX-License-Identifier: GPL-3.0-only
//! The `raw` action on the worker runtime (ENG-29).
//!
//! A test module of type `strike` has one scenario whose client writes bytes
//! with `raw`, and the client runtime dials a plain `TcpListener` of the test.
//! The listener sees the exact bytes of every session, including the ones that
//! a template would change or refuse: `{{` and `}}`, a NUL, and bytes that are
//! no UTF-8. The module is built here, so the test does not depend on the
//! strike module of the repository. The second test loads the same scenario in
//! a module of type `app`, which must fail with a named error.

use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use fps_backend_kernel::ConnectConfig;
use fps_flow::{LoadError, Manifest, ModuleSet, Scenario};
use fps_hosts::{HostPool, HostSelector, PoolSpec, Selection};
use fps_interp::Side;
use fps_runtime::{ClientConfig, FlowRef, Role, Runtime, RuntimeConfig, SourceHosts, WorkerConfig};
use fps_sched::{stream_id, Arrival, LoadProfile, Objective, Phase, ScheduleConfig, Shape};
use fps_stats::metric;

const SEED: u64 = 20_261_112;

/// Constant arrivals at 5 per second for 1 second give 5 starts.
const SESSIONS: u64 = 5;

/// The bytes of the raw action, as hex: `GET /{{ x }} HTTP/1.1`, a NUL, `}}`,
/// a lone 0xff, and an empty line.
const HEX: &str = "474554202f7b7b2078207d7d20485454502f312e3100007d7dff0d0a0d0a";

/// The same bytes.
const BYTES: &[u8] = b"GET /{{ x }} HTTP/1.1\x00\x00}}\xff\r\n\r\n";

fn scenario_text() -> String {
    format!(
        "scenario: raw_strike\nflows:\n  - id: a\n    module: m\n    client:\n      - connect: {{ port: 9000 }}\n      - raw: {{ hex: \"{HEX}\" }}\n      - close: fin\n"
    )
}

fn module_of(module_type: &str) -> ModuleSet {
    let mut set = ModuleSet::default();
    set.insert(Manifest::parse(&format!("name: m\ntype: {module_type}\n")).unwrap());
    set
}

fn load(module_type: &str) -> Result<Scenario, LoadError> {
    let set = module_of(module_type);
    let ctx = set.context("m").unwrap();
    let mut s = Scenario::parse_with(&scenario_text(), Some(&ctx))?;
    s.fold_constants().unwrap();
    Ok(s)
}

#[test]
fn the_strike_module_scenario_decodes_to_the_expected_bytes() {
    // Guards the two constants of this file against each other.
    let decoded = fps_flow::template::decode_hex(HEX).unwrap();
    assert_eq!(decoded, BYTES);
}

#[test]
fn a_strike_session_sends_the_exact_bytes_of_a_raw_action() {
    let scenario = Arc::new(load("strike").unwrap());
    let listener =
        TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 60, 0, 1)), 0)).unwrap();
    let server = listener.local_addr().unwrap();
    // `close: fin` waits for the peer to close too, so each connection is read
    // to the end of the stream, which is the FIN of the client, and then dropped.
    let (seen_tx, seen) = mpsc::channel();
    std::thread::spawn(move || {
        for _ in 0..SESSIONS {
            let Ok((mut conn, _)) = listener.accept() else {
                return;
            };
            let mut got = Vec::new();
            if conn.read_to_end(&mut got).is_ok() {
                let _ = seen_tx.send(got);
            }
        }
    });

    let hosts = HostPool::parse(&PoolSpec {
        name: "clients",
        ipv4: Some("127.61.0.0/24"),
        count: 16,
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
    let mut schedule = ScheduleConfig::new(Objective::SessionsPerSecond(5.0), profile, SEED);
    schedule.arrival = Arrival::Constant;
    schedule.stream = stream_id(0, 0);
    let worker = WorkerConfig {
        id: 0,
        core: None,
        role: Role::Client(Box::new(ClientConfig {
            flow: FlowRef::new(Arc::clone(&scenario), 0),
            schedule,
            servers: vec![server.ip()],
            port: Some(server.port()),
            sources: SourceHosts {
                selector: HostSelector::new("clients", range, Selection::RoundRobin, SEED, 0)
                    .unwrap(),
                pool: hosts.clone(),
                ipv6: false,
            },
            connect: ConnectConfig::default(),
        })),
    };
    let report = Runtime::start(RuntimeConfig::new(SEED, vec![worker]))
        .unwrap()
        .join();
    for (id, r) in &report.workers {
        assert!(r.is_ok(), "worker {id}: {r:?}");
    }

    let c = report.collector.total(Side::Client).unwrap();
    let sessions = c.counter_total(metric::SESSIONS_ATTEMPTED);
    assert_eq!(sessions, SESSIONS);
    assert_eq!(c.counter_total(metric::SESSIONS_SUCCEEDED), sessions);
    assert_eq!(c.counter_total(metric::SESSIONS_FAILED), 0);
    assert_eq!(
        c.counter_total(metric::BYTES_TX),
        sessions * BYTES.len() as u64
    );

    for n in 0..sessions {
        let got = seen
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|e| panic!("session {n} sent nothing: {e}"));
        assert_eq!(got, BYTES, "session {n}");
    }
}

#[test]
fn the_same_scenario_in_an_app_module_fails_to_load() {
    let err = load("app").expect_err("raw in an app module");
    assert!(matches!(err, LoadError::ReservedAction { .. }), "{err:?}");
    assert!(
        err.to_string()
            .ends_with("flow 'a': client: action 2: 'raw' is reserved for module types [\"fuzzer\", \"strike\"]"),
        "{err}"
    );
    // A type that no module of the repository uses yet is no exception.
    for module_type in ["tunnel", "evasion", "transport", "host", "content"] {
        assert!(matches!(
            load(module_type),
            Err(LoadError::ReservedAction { .. })
        ));
    }
}
