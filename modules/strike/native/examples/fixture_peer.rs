// SPDX-License-Identifier: GPL-3.0-only
//! Runs one side of the strike scenarios over kernel sockets.
//!
//! `modules/strike/fixtures/capture.sh` starts the server side in the server
//! namespace of the QA-01 harness and the client side in the client
//! namespace, and records the traffic between them as the module fixture.
//! Both sides walk the selected scenarios in the same order and run the
//! sessions one after another, so the n-th connection of a port on the
//! server always belongs to the n-th client session.
//!
//! Every session is a scenario instance. The server binds every listen port
//! of its scenarios before the first session and keeps the listeners open
//! for the whole run, so the churn children of `conn_churn` are accepted
//! one by one from the backlog of a single listener, and a child that dials
//! a moment early meets an open port instead of a refusal. The client also
//! retries a refused connection for a short time.
//!
//! ```text
//! fixture_peer server --listen 10.202.0.2 [options]
//! fixture_peer client --connect 10.202.0.2 --source 10.201.0.2 [options]
//!
//! options
//!   --sessions N      sessions per scenario, default 10
//!   --seed S          run seed, default 42
//!   --var NAME=VALUE  replace a variable in every scenario that declares it
//!   --scenario NAME   only run this scenario, repeatable; the name is the
//!                     file stem, the manifest path, or the path with the
//!                     extension, and the scenarios keep the manifest order
//! ```
//!
//! A listen port is a variable whose name ends in `_port`, so a run can move
//! a scenario to its own ports with `--var server_port=... --var
//! churn_port=...` and the phases of a capture never share a port.

use std::cell::RefCell;
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use fps_backend_kernel::{ConnectConfig, Connector, KernelListener, KernelStream, ListenConfig};
use fps_flow::program::Flow;
use fps_flow::template::Filler;
use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use fps_flow::{Manifest, ModuleSet, Scenario, Value};
use fps_interp::{
    DatagramOpener, FlowEnv, InstanceConfig, Limits, OpenKind, OpenRequest, Opener,
    ScenarioInstance, Side, Sides,
};
use fps_io::memory::MemoryDatagram;
use rand::RngCore;

/// The scenarios call no module function, and `rand_int` is a builtin of the
/// template engine, so this resolver only rejects.
struct NoFunctions;

impl TemplateFn for NoFunctions {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        _rng: &mut dyn RngCore,
        _out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        Err(FnError::new(
            FnErrorKind::Unsupported,
            format!("no module provides '{}.{}'", call.module, call.function),
        ))
    }
}

/// The listeners that stay open for the whole run.
type Held = Rc<RefCell<Vec<Rc<KernelListener>>>>;

/// Opens kernel TCP sessions for the interpreter.
struct KernelOpener {
    server: IpAddr,
    source: Option<IpAddr>,
    held: Held,
}

fn to_io(e: fps_backend_kernel::BackendError) -> io::Error {
    io::Error::other(e.to_string())
}

impl KernelOpener {
    fn bind_port(&self, port: u16) -> io::Result<()> {
        let addr = SocketAddr::new(self.server, port);
        let l = KernelListener::bind(addr, &ListenConfig::default()).map_err(to_io)?;
        self.held.borrow_mut().push(Rc::new(l));
        Ok(())
    }
}

impl Opener for KernelOpener {
    type Io = KernelStream;

    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<KernelStream>> {
        let (kind, port) = (request.kind, request.port);
        let addr = SocketAddr::new(self.server, port);
        let source = self.source.unwrap_or(IpAddr::from([0, 0, 0, 0]));
        let held = self
            .held
            .borrow()
            .iter()
            .find(|l| l.local_addr() == addr)
            .cloned();
        async move {
            match kind {
                OpenKind::Connect => {
                    let connector = Connector::new(ConnectConfig::default());
                    // The server binds its ports before the first session,
                    // so a refusal here means the run started too early.
                    let mut last = None;
                    for _ in 0..100 {
                        match connector.connect(source, addr).await {
                            Ok(s) => return Ok(s),
                            Err(e) if e.io_kind() == Some(io::ErrorKind::ConnectionRefused) => {
                                last = Some(e);
                                tokio::time::sleep(Duration::from_millis(10)).await;
                            }
                            Err(e) => return Err(to_io(e)),
                        }
                    }
                    Err(last.map_or_else(|| io::Error::other("no attempt"), to_io))
                }
                OpenKind::Accept => {
                    if let Some(l) = held {
                        return l.accept().await.map(|(s, _)| s).map_err(to_io);
                    }
                    // A port that the run did not pre-bind: accept one
                    // connection and release the port when the listener
                    // drops.
                    let l = KernelListener::bind(addr, &ListenConfig::default()).map_err(to_io)?;
                    l.accept().await.map(|(s, _)| s).map_err(to_io)
                }
            }
        }
    }
}

/// The scenarios have no UDP flow, so this opener refuses every open.
struct NoDatagrams;

impl DatagramOpener for NoDatagrams {
    type Io = MemoryDatagram;

    async fn open(&mut self, _request: OpenRequest<'_>) -> io::Result<MemoryDatagram> {
        Err(io::Error::other("the strike module has no UDP flow"))
    }
}

struct Env {
    server: IpAddr,
    source: Option<IpAddr>,
    held: Held,
}

impl FlowEnv for Env {
    type Stream = KernelOpener;
    type Datagram = NoDatagrams;

    fn stream(&mut self, _flow: &Flow, _side: Side) -> KernelOpener {
        KernelOpener {
            server: self.server,
            source: self.source,
            held: Rc::clone(&self.held),
        }
    }

    fn datagram(&mut self, _flow: &Flow, _side: Side) -> NoDatagrams {
        NoDatagrams
    }

    fn functions(&mut self, _flow: &Flow, _side: Side) -> Box<dyn TemplateFn> {
        Box::new(NoFunctions)
    }
}

struct Options {
    side: Side,
    server: IpAddr,
    source: Option<IpAddr>,
    sessions: u64,
    seed: u64,
    vars: Vec<(String, Value)>,
    only: Vec<String>,
}

fn parse_args() -> Result<Options, String> {
    let mut args = std::env::args().skip(1);
    let side = match args.next().as_deref() {
        Some("client") => Side::Client,
        Some("server") => Side::Server,
        _ => return Err("the first argument must be client or server".into()),
    };
    let mut o = Options {
        side,
        server: IpAddr::from([127, 0, 0, 1]),
        source: None,
        sessions: 10,
        seed: 42,
        vars: Vec::new(),
        only: Vec::new(),
    };
    let ip = |s: Option<String>| -> Result<IpAddr, String> {
        s.ok_or("missing address")?
            .parse()
            .map_err(|e| format!("bad address: {e}"))
    };
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--listen" | "--connect" => o.server = ip(args.next())?,
            "--source" => o.source = Some(ip(args.next())?),
            "--sessions" | "--seed" => {
                let n: u64 = args
                    .next()
                    .ok_or("missing number")?
                    .parse()
                    .map_err(|e| format!("bad number: {e}"))?;
                if flag == "--seed" {
                    o.seed = n;
                } else {
                    o.sessions = n;
                }
            }
            "--scenario" => o.only.push(args.next().ok_or("missing scenario name")?),
            "--var" => {
                let kv = args.next().ok_or("missing NAME=VALUE")?;
                let (k, v) = kv.split_once('=').ok_or("expected NAME=VALUE")?;
                let value = v
                    .parse::<i64>()
                    .map_or_else(|_| Value::Str(v.to_string()), Value::Int);
                o.vars.push((k.to_string(), value));
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(o)
}

/// Whether a manifest path names the requested scenario.
fn scenario_matches(rel: &str, want: &str) -> bool {
    let stem = Path::new(rel)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    rel == want || stem == want || rel == format!("{want}.yaml")
}

fn load(vars: &[(String, Value)], only: &[String]) -> Result<Vec<(String, Scenario)>, String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let m = Manifest::load(&dir.join("manifest.yaml")).map_err(|e| e.to_string())?;
    let mut set = ModuleSet::default();
    set.insert(m.clone());
    let ctx = set.context("strike").ok_or("no strike context")?;
    let mut out = Vec::new();
    for rel in &m.scenarios {
        let path = rel.display().to_string();
        if !only.is_empty() && !only.iter().any(|want| scenario_matches(&path, want)) {
            continue;
        }
        let mut s =
            Scenario::load_instance(&dir.join(rel), Some(&ctx)).map_err(|e| e.to_string())?;
        let own: Vec<(String, Value)> = vars
            .iter()
            .filter(|(k, _)| s.vars.contains_key(k))
            .cloned()
            .collect();
        s.override_vars(&own).map_err(|e| e.to_string())?;
        s.fold_constants().map_err(|e| e.to_string())?;
        out.push((path, s));
    }
    if out.is_empty() {
        return Err("no scenario of the manifest was selected".into());
    }
    for want in only {
        if !out.iter().any(|(path, _)| scenario_matches(path, want)) {
            return Err(format!("the manifest has no scenario named {want}"));
        }
    }
    Ok(out)
}

/// The listen ports of the scenarios, the variables that end in `_port`.
fn listen_ports(scenarios: &[(String, Scenario)]) -> Result<Vec<u16>, String> {
    let mut ports = Vec::new();
    for (name, s) in scenarios {
        let mut found = false;
        for (key, value) in &s.vars {
            if key.ends_with("_port") {
                let port = value
                    .as_i64()
                    .and_then(|p| u16::try_from(p).ok())
                    .ok_or_else(|| format!("{name} has no valid {key}"))?;
                if !ports.contains(&port) {
                    ports.push(port);
                }
                found = true;
            }
        }
        if !found {
            return Err(format!("{name} declares no *_port variable"));
        }
    }
    ports.sort_unstable();
    Ok(ports)
}

async fn run(o: Options) -> Result<(), String> {
    let scenarios = load(&o.vars, &o.only)?;
    let filler = Filler::new(o.seed);
    let held: Held = Rc::default();
    let mut env = Env {
        server: o.server,
        source: o.source,
        held: Rc::clone(&held),
    };
    if o.side == Side::Server {
        // The server binds every listen port before its first session, so a
        // client and its spawned children never meet a closed port.
        let opener = env.stream(&scenarios[0].1.flows[0], Side::Server);
        for port in listen_ports(&scenarios)? {
            opener
                .bind_port(port)
                .map_err(|e| format!("bind {port}: {e}"))?;
        }
    }
    let sides = if o.side == Side::Client {
        Sides::Client
    } else {
        Sides::Server
    };
    let mut first_session_id = 1;
    let mut failures = 0;
    for (name, s) in &scenarios {
        for _ in 0..o.sessions {
            let mut instance = ScenarioInstance::new(
                s,
                InstanceConfig {
                    sides,
                    run_seed: o.seed,
                    first_session_id,
                    limits: Limits::default(),
                },
            );
            // Both sides step the ids by the same amount, so the n-th
            // session has the same ids on the client and the server.
            first_session_id += 100;
            if let Err(e) = instance
                .run(&mut env, &filler, std::future::pending())
                .await
            {
                failures += 1;
                eprintln!("{name} session {}: {e}", first_session_id - 100);
            }
        }
        println!("{name}: {} sessions", o.sessions);
    }
    if failures > 0 {
        return Err(format!("{failures} sessions failed"));
    }
    Ok(())
}

fn main() -> ExitCode {
    let o = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("fixture_peer: {e}");
            return ExitCode::from(2);
        }
    };
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("fixture_peer: {e}");
            return ExitCode::from(2);
        }
    };
    match rt.block_on(run(o)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fixture_peer: {e}");
            ExitCode::FAILURE
        }
    }
}
