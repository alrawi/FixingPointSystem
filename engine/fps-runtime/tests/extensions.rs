// SPDX-License-Identifier: GPL-3.0-only
//! WebAssembly module extensions in the worker runtime (ENG-17).
//!
//! The extension is the WAT component of the `fps-wasm` tests, which
//! implements the `extension` world by hand. Its functions are `echo`, `spin`
//! (an endless loop), `rand`, and `fail`.

#[path = "../../fps-wasm/tests/support/mod.rs"]
mod support;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fps_backend_kernel::{ConnectConfig, ListenConfig};
use fps_flow::function::NoFunctions;
use fps_flow::manifest::Manifest;
use fps_flow::{FnCall, FnErrorKind, Scenario, TemplateFn};
use fps_hosts::{HostPool, HostSelector, PoolSpec, Selection};
use fps_interp::Side;
use fps_runtime::{
    extension_rng, ClientConfig, ExtensionError, ExtensionSet, FlowRef, Role, RunReport, Runtime,
    RuntimeConfig, ServerConfig, SessionKey, SourceHosts, Transport, WorkerConfig,
};
use fps_sched::{stream_id, Arrival, LoadProfile, Objective, Phase, ScheduleConfig, Shape};
use fps_stats::{metric, SeriesValue, Snapshot};
use fps_wasm::{CloseReason, Limits, WasmFailure};
use rand::RngCore;
use support::{component_bytes, component_wat};

const SEED: u64 = 20_261_110;
/// Sessions that each client worker starts.
const SESSIONS: u64 = 40;
const GOOD_CLIENT: u32 = 20;
const LOOPING_CLIENT: u32 = 21;

/// Both sides call the extension. The client also reads the answer.
const GOOD: &str = r#"
scenario: good
flows:
  - id: good
    module: t
    client:
      - connect: { port: 9000 }
      - send: "{{ t.echo('ping') }}\r\n"
      - expect: { pattern: "pong\r\n", timeout_ms: 5000 }
      - close: fin
    server:
      - accept: { port: 9000 }
      - expect: { pattern: "ping\r\n", timeout_ms: 5000 }
      - send: "{{ t.echo('pong') }}\r\n"
      - close: fin
"#;

/// The client renders an endless loop of the extension.
const LOOPING: &str = r#"
scenario: looping
flows:
  - id: looping
    module: t
    client:
      - connect: { port: 9001 }
      - send: "{{ t.spin() }}"
      - close: fin
    server:
      - accept: { port: 9001 }
      - expect: { pattern: "never", timeout_ms: 5000 }
"#;

const MANIFEST: &str = r#"
name: t
type: app
functions:
  - name: echo
    params:
      - { name: data, type: bytes }
  - name: spin
  - name: grow
  - name: probe
  - name: rand
  - name: fail
"#;

fn compile(text: &str) -> Arc<Scenario> {
    let mut s = Scenario::parse(text).unwrap();
    s.fold_constants().unwrap();
    Arc::new(s)
}

fn small_fuel() -> Limits {
    Limits {
        fuel_per_call: 200_000,
        ..Limits::default()
    }
}

fn set(limits: Limits) -> Arc<ExtensionSet> {
    let mut set = ExtensionSet::new().unwrap();
    set.load("t", &component_bytes(), limits, None).unwrap();
    Arc::new(set)
}

fn v4(a: u8, b: u8, c: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, a, b, c))
}

fn with_extensions(scenario: &Arc<Scenario>, set: &Arc<ExtensionSet>) -> FlowRef {
    let mut flow = FlowRef::new(Arc::clone(scenario), 0);
    flow.extensions = Some(Arc::clone(set));
    flow
}

/// Sessions of `snapshot` that failed with `reason`.
fn failed_with(snapshot: &Snapshot, reason: &str) -> u64 {
    snapshot
        .iter()
        .filter(|(k, _)| {
            k.metric() == metric::SESSIONS_FAILED
                && k.labels()
                    .iter()
                    .any(|(l, v)| *l == "reason" && v == reason)
        })
        .map(|(_, v)| match v {
            SeriesValue::Count(c) => *c,
            _ => 0,
        })
        .sum()
}

fn exit(report: &RunReport, worker: u32) -> &Snapshot {
    &report.workers[&worker].as_ref().unwrap().sessions
}

/// The acceptance test. A good scenario and a looping one run in the same
/// runtimes and share one compiled extension. Every looping session fails
/// with `extension_error` at its fuel limit, and every good session on the
/// same threads and the same extension succeeds.
#[test]
fn a_looping_extension_fails_only_its_own_sessions() {
    let (good, looping) = (compile(GOOD), compile(LOOPING));
    let ext = set(small_fuel());
    let servers = Runtime::start(RuntimeConfig::new(
        SEED,
        [(10u32, &good, 0u8), (11, &looping, 1)]
            .into_iter()
            .map(|(id, scenario, n)| WorkerConfig {
                id,
                core: None,
                role: Role::Server(ServerConfig {
                    flow: with_extensions(scenario, &ext),
                    listen: vec![SocketAddr::new(v4(60, 0, 1 + n), 0)],
                    config: ListenConfig::default(),
                }),
            })
            .collect(),
    ))
    .unwrap();
    let addrs = servers.listen_addrs();
    assert_eq!(addrs.len(), 2);

    let prefix = "127.61.0.0/24";
    let hosts = HostPool::parse(&PoolSpec {
        name: "clients",
        ipv4: Some(prefix),
        count: 16,
        ..PoolSpec::default()
    })
    .unwrap();
    let ranges = hosts.range().split_even(2).unwrap();
    // The test component refuses session id 13 in `on-open`, which is local
    // session 13 of worker 0, so the client workers use ids 20 and 21.
    let workers = [(GOOD_CLIENT, &good), (LOOPING_CLIENT, &looping)]
        .into_iter()
        .zip(ranges)
        .enumerate()
        .map(|(n, ((id, scenario), range))| {
            let profile = LoadProfile::new(vec![Phase::new(
                "steady",
                Duration::from_secs(1),
                Shape::Hold,
                1.0,
                1.0,
            )])
            .unwrap();
            let mut schedule =
                ScheduleConfig::new(Objective::SessionsPerSecond(SESSIONS as f64), profile, SEED);
            schedule.arrival = Arrival::Constant;
            schedule.stream = stream_id(id, 0);
            let server = addrs[n];
            WorkerConfig {
                id,
                core: None,
                role: Role::Client(Box::new(ClientConfig {
                    flow: with_extensions(scenario, &ext),
                    schedule,
                    servers: vec![server.ip()],
                    port: Some(server.port()),
                    sources: SourceHosts {
                        selector: HostSelector::new(
                            "clients",
                            range,
                            Selection::RoundRobin,
                            SEED,
                            id,
                        )
                        .unwrap(),
                        pool: hosts.clone(),
                        ipv6: false,
                    },
                    connect: ConnectConfig::default(),
                })),
            }
        })
        .collect();
    let client = Runtime::start(RuntimeConfig::new(SEED, workers)).unwrap();
    let client = client.join();
    servers.stop(Duration::from_secs(1));
    let server = servers.join();

    for report in [&client, &server] {
        for (id, r) in &report.workers {
            assert!(r.is_ok(), "worker {id}: {r:?}");
        }
    }
    // The good client and server: every session succeeded.
    let g = exit(&client, GOOD_CLIENT);
    assert_eq!(g.counter_total(metric::SESSIONS_ATTEMPTED), SESSIONS);
    assert_eq!(g.counter_total(metric::SESSIONS_SUCCEEDED), SESSIONS);
    assert_eq!(g.counter_total(metric::SESSIONS_FAILED), 0);
    let gs = exit(&server, 10);
    assert_eq!(gs.counter_total(metric::SESSIONS_SUCCEEDED), SESSIONS);
    assert_eq!(gs.counter_total(metric::SESSIONS_FAILED), 0);
    // The looping client: every session failed with extension_error, and no
    // other reason shows up.
    let l = exit(&client, LOOPING_CLIENT);
    assert_eq!(l.counter_total(metric::SESSIONS_ATTEMPTED), SESSIONS);
    assert_eq!(l.counter_total(metric::SESSIONS_SUCCEEDED), 0);
    assert_eq!(l.counter_total(metric::SESSIONS_FAILED), SESSIONS);
    assert_eq!(failed_with(l, "extension_error"), SESSIONS);
}

fn call(function: &str) -> FnCall<'_> {
    FnCall {
        module: "t",
        function,
        args: fps_flow::CallArgs {
            positional: &[],
            named: &[],
        },
    }
}

fn key(seed: u64, session_id: u64, scenario: &Arc<Scenario>) -> SessionKey {
    SessionKey {
        seed,
        session_id,
        side: Side::Client,
        scenario: Arc::clone(scenario),
        flow: 0,
        transport: Transport::Tcp,
    }
}

/// Renders `rand` once in a fresh session and returns the eight bytes.
fn draw(ext: &Arc<ExtensionSet>, k: SessionKey) -> Vec<u8> {
    let mut f = ext.session_functions(k, Box::new(NoFunctions));
    let mut out = Vec::new();
    f.render(
        &call("rand"),
        &mut rand::rngs::mock::StepRng::new(0, 1),
        &mut out,
    )
    .unwrap();
    assert_eq!(out.len(), 8);
    out
}

#[test]
fn extension_randomness_derives_from_the_seed_and_the_session_id() {
    let scenario = compile(GOOD);
    let ext = set(Limits::default());
    let a = draw(&ext, key(SEED, 7, &scenario));
    // The same seed and id give the same bytes in a new instance.
    assert_eq!(a, draw(&ext, key(SEED, 7, &scenario)));
    // Another id, another seed, or another side gives other bytes.
    assert_ne!(a, draw(&ext, key(SEED, 8, &scenario)));
    assert_ne!(a, draw(&ext, key(SEED + 1, 7, &scenario)));
    let mut server_key = key(SEED, 7, &scenario);
    server_key.side = Side::Server;
    assert_ne!(a, draw(&ext, server_key));
    // The bytes are the first draw of the documented generator.
    let mut rng = extension_rng(SEED, "t", 7, Side::Client);
    assert_eq!(a, rng.next_u64().to_le_bytes());
    // The generator differs per module and from the template stream.
    assert_ne!(
        rng.next_u64(),
        extension_rng(SEED, "u", 7, Side::Client).next_u64()
    );
    assert_ne!(
        extension_rng(SEED, "t", 7, Side::Client).next_u64(),
        fps_interp::session_rng(SEED, 7, Side::Client).next_u64()
    );
}

#[test]
fn a_fuel_limit_fails_the_session_and_leaves_other_sessions_alone() {
    let scenario = compile(GOOD);
    let ext = set(small_fuel());
    let mut victim = ext.session_functions(key(SEED, 1, &scenario), Box::new(NoFunctions));
    let mut bystander = ext.session_functions(key(SEED, 2, &scenario), Box::new(NoFunctions));
    let mut rng = rand::rngs::mock::StepRng::new(0, 1);
    let mut out = Vec::new();

    // The bystander opens its instance first and keeps it across the failure.
    bystander.render(&call("rand"), &mut rng, &mut out).unwrap();
    let err = victim
        .render(&call("spin"), &mut rng, &mut out)
        .unwrap_err();
    assert_eq!(err.kind, FnErrorKind::Internal);
    let failure = victim.take_failure().unwrap();
    assert!(matches!(failure, WasmFailure::OutOfFuel), "{failure}");
    assert_eq!(failure.reason(), fps_stats::FailureReason::ExtensionError);
    bystander.render(&call("rand"), &mut rng, &mut out).unwrap();

    // A hook error keeps its kind, and a function that the catalog lacks is
    // unsupported without opening an instance.
    let mut s = ext.session_functions(key(SEED, 3, &scenario), Box::new(NoFunctions));
    let err = s.render(&call("fail"), &mut rng, &mut out).unwrap_err();
    assert_eq!(err.kind, FnErrorKind::Internal);
    assert!(matches!(s.take_failure(), Some(WasmFailure::Hook(_))));
    let err = s.render(&call("nope"), &mut rng, &mut out).unwrap_err();
    assert_eq!(err.kind, FnErrorKind::Unsupported);

    // Sessions end with the on-close hook. A failed session skips it.
    assert!(bystander.finish(CloseReason::Finished).is_none());
    assert!(victim.finish(CloseReason::Failed).is_none());
}

#[test]
fn modules_outside_the_set_use_the_native_functions() {
    let scenario = compile(GOOD);
    let ext = set(Limits::default());
    let mut f = ext.session_functions(key(SEED, 1, &scenario), Box::new(NoFunctions));
    let mut out = Vec::new();
    let other = FnCall {
        module: "other",
        function: "x",
        args: fps_flow::CallArgs {
            positional: &[],
            named: &[],
        },
    };
    let err = f
        .render(&other, &mut rand::rngs::mock::StepRng::new(0, 1), &mut out)
        .unwrap_err();
    assert_eq!(err.kind, FnErrorKind::Unsupported);
    assert!(f.take_failure().is_none());
    assert!(!ext.contains("other") && ext.contains("t") && !ext.is_empty());
}

#[test]
fn the_catalog_must_match_the_manifest() {
    let manifest = Manifest::parse(MANIFEST).unwrap();
    let mut ok = ExtensionSet::new().unwrap();
    ok.load(
        "t",
        &component_bytes(),
        Limits::default(),
        Some(&manifest.functions),
    )
    .unwrap();
    assert!(matches!(
        ok.load("t", &component_bytes(), Limits::default(), None),
        Err(ExtensionError::Duplicate { .. })
    ));

    let wrong = Manifest::parse(&MANIFEST.replace("type: bytes", "type: int")).unwrap();
    let mut bad = ExtensionSet::new().unwrap();
    let err = bad
        .load(
            "t",
            &component_bytes(),
            Limits::default(),
            Some(&wrong.functions),
        )
        .unwrap_err();
    assert!(matches!(err, ExtensionError::Catalog { .. }), "{err}");
    let missing = Manifest::parse(&MANIFEST.replace("  - name: fail\n", "")).unwrap();
    let err = bad
        .load(
            "t",
            &component_bytes(),
            Limits::default(),
            Some(&missing.functions),
        )
        .unwrap_err();
    assert!(err.to_string().contains("'fail'"), "{err}");
    assert!(bad.is_empty());

    let err = bad
        .load("t", b"not a component", Limits::default(), None)
        .unwrap_err();
    assert!(matches!(err, ExtensionError::Load { .. }), "{err}");
}

#[test]
fn an_extension_loads_from_the_module_directory() {
    let dir: PathBuf = [env!("CARGO_TARGET_TMPDIR"), "eng17-module"]
        .iter()
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = Manifest::parse(MANIFEST).unwrap();
    let mut set = ExtensionSet::new().unwrap();
    // A module directory without extension.wasm has no extension.
    assert!(!set
        .load_module_dir(&manifest, &dir, Limits::default())
        .unwrap());
    assert!(set.is_empty());
    std::fs::write(dir.join("extension.wasm"), component_bytes()).unwrap();
    assert!(set
        .load_module_dir(&manifest, &dir, Limits::default())
        .unwrap());
    assert!(set.contains("t"));
}

/// The test component with an `on-open` that refuses a UDP session, so a
/// session shows the transport that its extension saw.
fn udp_refusing_component() -> Vec<u8> {
    let wat = component_wat();
    let patched = wat.replace(
        "(if (i64.eq (local.get 0) (i64.const 13))",
        "(if (i32.or (i64.eq (local.get 0) (i64.const 13)) \
         (i32.eq (local.get 4) (i32.const 1)))",
    );
    assert_ne!(wat, patched, "the on-open guard moved, update this test");
    let mut core = wat::parse_str(patched).unwrap();
    let mut resolve = wit_parser::Resolve::default();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/extension");
    let (pkg, _) = resolve.push_path(dir).unwrap();
    let world = resolve.select_world(&[pkg], Some("extension")).unwrap();
    wit_component::embed_component_metadata(
        &mut core,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
    )
    .unwrap();
    wit_component::ComponentEncoder::default()
        .module(&core)
        .unwrap()
        .validate(true)
        .encode()
        .unwrap()
}

#[test]
fn the_extension_sees_the_transport_of_its_session() {
    let mut set = ExtensionSet::new().unwrap();
    set.load("t", &udp_refusing_component(), Limits::default(), None)
        .unwrap();
    let set = Arc::new(set);
    let scenario = compile(GOOD);
    let render = |transport: Transport| {
        let mut k = key(SEED, 1, &scenario);
        k.transport = transport;
        let mut f = set.session_functions(k, Box::new(NoFunctions));
        let mut out = Vec::new();
        let r = f.render(
            &call("rand"),
            &mut rand::rngs::mock::StepRng::new(0, 1),
            &mut out,
        );
        (r.is_ok(), f.take_failure().is_some())
    };
    assert_eq!(render(Transport::Tcp), (true, false));
    // The extension refused the open because it saw transport udp.
    assert_eq!(render(Transport::Udp), (false, true));
}

#[test]
fn the_compiled_extension_is_the_bytes_the_caller_passes() {
    let dir: PathBuf = [env!("CARGO_TARGET_TMPDIR"), "eng21-module"]
        .iter()
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = Manifest::parse(MANIFEST).unwrap();
    let good = component_bytes();

    // The file on disk differs from the digested bytes, as after a swap:
    // the component that loads is the passed one.
    std::fs::write(dir.join("extension.wasm"), b"swapped after the digest").unwrap();
    let mut set = ExtensionSet::new().unwrap();
    set.load_module_bytes(&manifest, &good, Limits::default())
        .unwrap();
    assert!(set.contains("t"));
    let set = Arc::new(set);
    let scenario = compile(GOOD);
    assert_eq!(draw(&set, key(SEED, 1, &scenario)).len(), 8);

    // Bad bytes fail even when the file on disk is good.
    std::fs::write(dir.join("extension.wasm"), &good).unwrap();
    let mut set = ExtensionSet::new().unwrap();
    let err = set
        .load_module_bytes(&manifest, b"not a component", Limits::default())
        .unwrap_err();
    assert!(matches!(err, ExtensionError::Load { .. }), "{err}");
    assert!(set.is_empty());

    // The bytes still meet the catalog check against the manifest.
    let other = Manifest::parse("name: t\ntype: app\nfunctions:\n  - name: other\n").unwrap();
    let err = set
        .load_module_bytes(&other, &good, Limits::default())
        .unwrap_err();
    assert!(matches!(err, ExtensionError::Catalog { .. }), "{err}");

    // load_module_dir still reads the file and loads the same way.
    assert!(set
        .load_module_dir(&manifest, &dir, Limits::default())
        .unwrap());
    assert!(set.contains("t"));
}

#[test]
fn close_reason_maps_outcomes_for_every_runner() {
    use fps_interp::{Failure, FailureKind, Reason};
    assert_eq!(fps_runtime::close_reason(&Ok(())), CloseReason::Finished);
    let failed = |reason| {
        fps_runtime::close_reason(&Err(Failure {
            flow: "f".to_owned(),
            side: Side::Client,
            action_index: 0,
            action_type: "connect",
            reason,
            kind: Box::new(FailureKind::OpenTimeout { timeout_ms: 1 }),
        }))
    };
    assert_eq!(failed(Reason::Aborted), CloseReason::Reset);
    assert_eq!(failed(Reason::PeerClosed), CloseReason::PeerClosed);
    assert_eq!(failed(Reason::ExpectTimeout), CloseReason::Timeout);
    assert_eq!(failed(Reason::ExtensionError), CloseReason::Failed);
}

#[test]
fn a_flow_has_no_extensions_by_default() {
    let flow = FlowRef::new(compile(GOOD), 0);
    assert!(flow.extensions.is_none());
}
