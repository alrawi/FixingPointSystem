// SPDX-License-Identifier: GPL-3.0-only
//! Runs one side of every hls scenario over kernel sockets.
//!
//! `modules/hls/fixtures/capture.sh` starts the server side in the server
//! namespace of the QA-01 harness and the client side in the client
//! namespace, and records the traffic between them as the module fixture.
//! Both sides walk the scenarios of the manifest in the same order and run
//! the sessions one after another, so the n-th playlist connection on the
//! server always belongs to the n-th client session.
//!
//! Every session is a scenario instance, because `vod_segments` spawns a
//! segment flow for every segment. The server listens on the playlist port and
//! on the segment port of every scenario for the whole run. The client gives
//! every connection of a session its own source port, one after another from
//! 40000, so that no two sessions of a capture share a 5-tuple (MOD-21). The
//! spawned segment flows dial at the instants of the segment schedule, and a
//! client that dials before the server side of the spawned flow waits at the
//! listener, which is bound already, so it retries nothing.
//!
//! ```text
//! fixture_peer server --listen 10.202.0.2 [options]
//! fixture_peer client --connect 10.202.0.2 --source 10.201.0.2 [options]
//!
//! options
//!   --sessions N              sessions per scenario, default 10
//!   --sessions-for NAME=N     sessions of one scenario
//!   --seed S                  run seed, default 42
//!   --var [NAME:]VAR=VALUE    replace a variable in every scenario that declares
//!                             it, or only in the scenario NAME
//! ```
//!
//! The client prints `connections=N` at the end, which is the number of TCP
//! connections that it opened, so `capture.sh` can check that the capture holds
//! one session for each.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Duration;

use fps_backend_kernel::{
    ConnectConfig, Connector, KernelListener, KernelStream, ListenConfig, PortRange,
};
use fps_flow::program::Flow;
use fps_flow::template::Filler;
use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use fps_flow::{Manifest, ModuleSet, Scenario, Value};
use fps_interp::{
    DatagramOpener, FlowEnv, InstanceConfig, Limits, OpenKind, OpenRequest, Opener,
    ScenarioInstance, Side, Sides,
};
use fps_io::memory::MemoryDatagram;
use fps_module_hls::Hls;
use fps_module_http1::Http1;
use rand::RngCore;

/// Routes the calls of the two modules that the scenarios use. Every flow side
/// gets its own value, because `hls.next` and `hls.pace` keep state.
#[derive(Default)]
struct Dispatch {
    hls: Hls,
    http1: Http1,
}

impl TemplateFn for Dispatch {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        match call.module {
            "hls" => self.hls.render(call, rng, out),
            "http1" => self.http1.render(call, rng, out),
            other => Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("no module provides '{other}.{}'", call.function),
            )),
        }
    }
}

/// The first source port of the client connections. It lies inside the default
/// ephemeral range of Linux, `net.ipv4.ip_local_port_range`.
const FIRST_SOURCE_PORT: u16 = 40_000;
/// The number of source ports that the client connections use before they start over.
const SOURCE_PORTS: u16 = 20_000;

/// The listeners of the server ports, which stay open for the whole run.
type Listeners = Rc<RefCell<Vec<Rc<KernelListener>>>>;

/// Opens kernel TCP sessions for the interpreter.
///
/// Every client connection gets its own source port, one after another from
/// [`FIRST_SOURCE_PORT`], from a counter that all flow sides of the process
/// share. The kernel would pick a port at random, and a port that an earlier
/// connection used and released can come up again for the same destination.
/// The capture then holds two sessions with one 5-tuple, which the session
/// table and the reference parsers see as one connection that restarts (MOD-21).
struct KernelOpener {
    server: IpAddr,
    source: Option<IpAddr>,
    listeners: Listeners,
    next_port: Rc<Cell<u16>>,
    opened: Rc<Cell<u64>>,
}

fn to_io(e: fps_backend_kernel::BackendError) -> io::Error {
    io::Error::other(e.to_string())
}

impl KernelOpener {
    fn bind(&self, port: u16) -> io::Result<()> {
        let addr = SocketAddr::new(self.server, port);
        let mut all = self.listeners.borrow_mut();
        if !all.iter().any(|l| l.local_addr() == addr) {
            let l = KernelListener::bind(addr, &ListenConfig::default()).map_err(to_io)?;
            all.push(Rc::new(l));
        }
        Ok(())
    }
}

impl Opener for KernelOpener {
    type Io = KernelStream;

    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<KernelStream>> {
        let (kind, port) = (request.kind, request.port);
        let addr = SocketAddr::new(self.server, port);
        let source = self.source.unwrap_or(IpAddr::from([0, 0, 0, 0]));
        let listener = self
            .listeners
            .borrow()
            .iter()
            .find(|l| l.local_addr() == addr)
            .cloned();
        let local = FIRST_SOURCE_PORT + self.next_port.get();
        if kind == OpenKind::Connect {
            self.next_port
                .set((self.next_port.get() + 1) % SOURCE_PORTS);
            self.opened.set(self.opened.get() + 1);
        }
        async move {
            match kind {
                OpenKind::Connect => {
                    let config = ConnectConfig {
                        local_ports: Some(PortRange::new(local, local).map_err(to_io)?),
                        ..ConnectConfig::default()
                    };
                    let connector = Connector::new(config);
                    // The server binds every port before its first session, but a refused
                    // connection costs nothing to try again, and the port stays the same.
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
                OpenKind::Accept => match listener {
                    Some(l) => l.accept().await.map(|(s, _)| s).map_err(to_io),
                    None => Err(io::Error::other(format!("port {port} is not bound"))),
                },
            }
        }
    }
}

/// The scenarios have no UDP flow, so this opener refuses every open.
struct NoDatagrams;

impl DatagramOpener for NoDatagrams {
    type Io = MemoryDatagram;

    async fn open(&mut self, _request: OpenRequest<'_>) -> io::Result<MemoryDatagram> {
        Err(io::Error::other("the hls module has no UDP flow"))
    }
}

struct Env {
    server: IpAddr,
    source: Option<IpAddr>,
    listeners: Listeners,
    next_port: Rc<Cell<u16>>,
    opened: Rc<Cell<u64>>,
}

impl FlowEnv for Env {
    type Stream = KernelOpener;
    type Datagram = NoDatagrams;

    fn stream(&mut self, _flow: &Flow, _side: Side) -> KernelOpener {
        KernelOpener {
            server: self.server,
            source: self.source,
            listeners: Rc::clone(&self.listeners),
            next_port: Rc::clone(&self.next_port),
            opened: Rc::clone(&self.opened),
        }
    }

    fn datagram(&mut self, _flow: &Flow, _side: Side) -> NoDatagrams {
        NoDatagrams
    }

    fn functions(&mut self, _flow: &Flow, _side: Side) -> Box<dyn TemplateFn> {
        Box::new(Dispatch::default())
    }
}

/// A variable override, for every scenario or for one.
struct Override {
    scenario: Option<String>,
    name: String,
    value: Value,
}

struct Options {
    side: Side,
    server: IpAddr,
    source: Option<IpAddr>,
    sessions: u64,
    sessions_for: Vec<(String, u64)>,
    seed: u64,
    vars: Vec<Override>,
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
        sessions_for: Vec::new(),
        seed: 42,
        vars: Vec::new(),
    };
    let ip = |s: Option<String>| -> Result<IpAddr, String> {
        s.ok_or("missing address")?
            .parse()
            .map_err(|e| format!("bad address: {e}"))
    };
    let number = |s: Option<String>| -> Result<u64, String> {
        s.ok_or("missing number")?
            .parse()
            .map_err(|e| format!("bad number: {e}"))
    };
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--listen" | "--connect" => o.server = ip(args.next())?,
            "--source" => o.source = Some(ip(args.next())?),
            "--sessions" => o.sessions = number(args.next())?,
            "--seed" => o.seed = number(args.next())?,
            "--sessions-for" => {
                let kv = args.next().ok_or("missing NAME=N")?;
                let (k, v) = kv.split_once('=').ok_or("expected NAME=N")?;
                o.sessions_for
                    .push((k.to_string(), number(Some(v.to_string()))?));
            }
            "--var" => {
                let kv = args.next().ok_or("missing [NAME:]VAR=VALUE")?;
                let (k, v) = kv.split_once('=').ok_or("expected [NAME:]VAR=VALUE")?;
                let (scenario, name) = match k.split_once(':') {
                    Some((s, n)) => (Some(s.to_string()), n.to_string()),
                    None => (None, k.to_string()),
                };
                let value = v
                    .parse::<i64>()
                    .map_or_else(|_| Value::Str(v.to_string()), Value::Int);
                o.vars.push(Override {
                    scenario,
                    name,
                    value,
                });
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(o)
}

fn load(vars: &[Override]) -> Result<Vec<(String, Scenario)>, String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let m = Manifest::load(&dir.join("manifest.yaml")).map_err(|e| e.to_string())?;
    let mut set = ModuleSet::default();
    set.insert(m.clone());
    // The scenarios call the http1 module, which the manifest requires.
    set.insert(
        Manifest::load(&dir.join("..").join("http1").join("manifest.yaml"))
            .map_err(|e| e.to_string())?,
    );
    let ctx = set.context("hls").ok_or("no hls context")?;
    let mut out = Vec::new();
    for rel in &m.scenarios {
        let mut s =
            Scenario::load_instance(&dir.join(rel), Some(&ctx)).map_err(|e| e.to_string())?;
        let stem = rel
            .file_stem()
            .map_or_else(String::new, |x| x.to_string_lossy().into_owned());
        let own: Vec<(String, Value)> = vars
            .iter()
            .filter(|v| s.vars.contains_key(&v.name))
            .filter(|v| v.scenario.as_deref().is_none_or(|n| n == stem))
            .map(|v| (v.name.clone(), v.value.clone()))
            .collect();
        s.override_vars(&own).map_err(|e| e.to_string())?;
        s.fold_constants().map_err(|e| e.to_string())?;
        out.push((stem, s));
    }
    Ok(out)
}

fn port_of(s: &Scenario, name: &str) -> Option<u16> {
    s.vars
        .get(name)
        .and_then(Value::as_i64)
        .and_then(|p| u16::try_from(p).ok())
}

async fn run(o: Options) -> Result<(), String> {
    let scenarios = load(&o.vars)?;
    let filler = Filler::new(o.seed);
    let listeners: Listeners = Rc::default();
    let opened = Rc::new(Cell::new(0));
    let mut env = Env {
        server: o.server,
        source: o.source,
        listeners: Rc::clone(&listeners),
        next_port: Rc::default(),
        opened: Rc::clone(&opened),
    };
    if o.side == Side::Server {
        // The server binds the playlist port and the segment port of every
        // scenario before its first session, so a client never meets a closed
        // port, and a spawned segment flow of the server finds its listener.
        let opener = env.stream(&scenarios[0].1.flows[0], Side::Server);
        for (_, s) in &scenarios {
            let port = port_of(s, "server_port").ok_or("a scenario has no valid server_port")?;
            opener.bind(port).map_err(|e| format!("bind {port}: {e}"))?;
            if let Some(port) = port_of(s, "segment_port") {
                opener.bind(port).map_err(|e| format!("bind {port}: {e}"))?;
            }
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
        let sessions = o
            .sessions_for
            .iter()
            .find(|(n, _)| n == name)
            .map_or(o.sessions, |(_, n)| *n);
        for _ in 0..sessions {
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
            first_session_id += 1000;
            if let Err(e) = instance
                .run(&mut env, &filler, std::future::pending())
                .await
            {
                failures += 1;
                eprintln!("{name} session {}: {e}", first_session_id - 1000);
            }
        }
        println!("{name}: {sessions} sessions");
    }
    println!("connections={}", opened.get());
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
