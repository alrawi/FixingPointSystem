// SPDX-License-Identifier: GPL-3.0-only
//! Runs one side of every ftp scenario over kernel sockets.
//!
//! `modules/ftp/fixtures/capture.sh` starts the server side in the server
//! namespace of the QA-01 harness and the client side in the client
//! namespace, and records the traffic between them as the module fixture.
//! Both sides walk the scenarios of the manifest in the same order and run
//! the sessions one after another, so the n-th control connection on the
//! server always belongs to the n-th client session.
//!
//! Every session is a scenario instance, because the data connections come
//! from `spawn`. The server listens on the control port of every scenario
//! for the whole run and binds the passive data port of a transfer when its
//! spawned data flow starts, accepts one connection, and releases the port.
//! The client dials the data port that it read out of the 227 or 229 reply,
//! and it retries a refused connection for a short time, because it may dial
//! before the server side of the spawned flow has bound the port.
//!
//! ```text
//! fixture_peer server --listen 10.202.0.2 [options]
//! fixture_peer client --connect 10.202.0.2 --source 10.201.0.2 [options]
//!
//! options
//!   --sessions N      sessions per scenario, default 10
//!   --seed S          run seed, default 42
//!   --var NAME=VALUE  replace a variable in every scenario that declares it
//! ```
//!
//! The server address also becomes the variable `server_ip` of every
//! scenario that declares it, unless `--var server_ip=...` names another one,
//! so the 227 replies announce the address that the data connections reach.

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
use fps_module_content::Content;
use fps_module_ftp::Ftp;
use rand::RngCore;

/// Routes the calls of the two modules that the scenarios use.
#[derive(Default)]
struct Dispatch {
    ftp: Ftp,
    content: Content,
}

impl TemplateFn for Dispatch {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        match call.module {
            "ftp" => self.ftp.render(call, rng, out),
            "content" => self.content.render(call, rng, out),
            other => Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("no module provides '{other}.{}'", call.function),
            )),
        }
    }
}

/// The listeners of the control ports, which stay open for the whole run.
type Control = Rc<RefCell<Vec<Rc<KernelListener>>>>;

/// Opens kernel TCP sessions for the interpreter.
struct KernelOpener {
    server: IpAddr,
    source: Option<IpAddr>,
    control: Control,
}

fn to_io(e: fps_backend_kernel::BackendError) -> io::Error {
    io::Error::other(e.to_string())
}

impl KernelOpener {
    fn bind_control(&self, port: u16) -> io::Result<()> {
        let addr = SocketAddr::new(self.server, port);
        let l = KernelListener::bind(addr, &ListenConfig::default()).map_err(to_io)?;
        self.control.borrow_mut().push(Rc::new(l));
        Ok(())
    }
}

impl Opener for KernelOpener {
    type Io = KernelStream;

    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<KernelStream>> {
        let (kind, port) = (request.kind, request.port);
        let addr = SocketAddr::new(self.server, port);
        let source = self.source.unwrap_or(IpAddr::from([0, 0, 0, 0]));
        let persistent = self
            .control
            .borrow()
            .iter()
            .find(|l| l.local_addr() == addr)
            .cloned();
        async move {
            match kind {
                OpenKind::Connect => {
                    let connector = Connector::new(ConnectConfig::default());
                    // A spawned data flow of the server may still be binding.
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
                    if let Some(l) = persistent {
                        return l.accept().await.map(|(s, _)| s).map_err(to_io);
                    }
                    // A passive data port: bind, accept one connection, and
                    // release the port when the listener drops.
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
        Err(io::Error::other("the ftp module has no UDP flow"))
    }
}

struct Env {
    server: IpAddr,
    source: Option<IpAddr>,
    control: Control,
}

impl FlowEnv for Env {
    type Stream = KernelOpener;
    type Datagram = NoDatagrams;

    fn stream(&mut self, _flow: &Flow, _side: Side) -> KernelOpener {
        KernelOpener {
            server: self.server,
            source: self.source,
            control: Rc::clone(&self.control),
        }
    }

    fn datagram(&mut self, _flow: &Flow, _side: Side) -> NoDatagrams {
        NoDatagrams
    }

    fn functions(&mut self, _flow: &Flow, _side: Side) -> Box<dyn TemplateFn> {
        Box::new(Dispatch::default())
    }
}

struct Options {
    side: Side,
    server: IpAddr,
    source: Option<IpAddr>,
    sessions: u64,
    seed: u64,
    vars: Vec<(String, Value)>,
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

fn load(vars: &[(String, Value)]) -> Result<Vec<(String, Scenario)>, String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let m = Manifest::load(&dir.join("manifest.yaml")).map_err(|e| e.to_string())?;
    let mut set = ModuleSet::default();
    set.insert(m.clone());
    // The scenarios call the content module, which the manifest requires.
    set.insert(
        Manifest::load(&dir.join("..").join("content").join("manifest.yaml"))
            .map_err(|e| e.to_string())?,
    );
    let ctx = set.context("ftp").ok_or("no ftp context")?;
    let mut out = Vec::new();
    for rel in &m.scenarios {
        let mut s =
            Scenario::load_instance(&dir.join(rel), Some(&ctx)).map_err(|e| e.to_string())?;
        let own: Vec<(String, Value)> = vars
            .iter()
            .filter(|(k, _)| s.vars.contains_key(k))
            .cloned()
            .collect();
        s.override_vars(&own).map_err(|e| e.to_string())?;
        s.fold_constants().map_err(|e| e.to_string())?;
        out.push((rel.display().to_string(), s));
    }
    Ok(out)
}

async fn run(mut o: Options) -> Result<(), String> {
    if !o.vars.iter().any(|(k, _)| k == "server_ip") {
        o.vars
            .push(("server_ip".into(), Value::Str(o.server.to_string())));
    }
    let scenarios = load(&o.vars)?;
    let filler = Filler::new(o.seed);
    let control: Control = Rc::default();
    let mut env = Env {
        server: o.server,
        source: o.source,
        control: Rc::clone(&control),
    };
    if o.side == Side::Server {
        // The server binds the control port of every scenario before its
        // first session, so a client never meets a closed port.
        let opener = env.stream(&scenarios[0].1.flows[0], Side::Server);
        let mut bound = Vec::new();
        for (_, s) in &scenarios {
            let port = s
                .vars
                .get("server_port")
                .and_then(Value::as_i64)
                .and_then(|p| u16::try_from(p).ok())
                .ok_or("a scenario has no valid server_port")?;
            if !bound.contains(&port) {
                opener
                    .bind_control(port)
                    .map_err(|e| format!("bind {port}: {e}"))?;
                bound.push(port);
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
