// SPDX-License-Identifier: GPL-3.0-only
//! Runs one side of every http1 scenario over kernel sockets.
//!
//! `modules/http1/fixtures/capture.sh` starts the server side in the server
//! namespace of the QA-01 harness and the client side in the client
//! namespace, and records the traffic between them as the module fixture.
//! Both sides walk the scenarios of the manifest in the same order and run
//! the sessions one after another, so the n-th accepted connection always
//! belongs to the n-th client session and both sides draw from the same
//! session generators.
//!
//! ```text
//! fixture_peer server --listen 10.202.0.2 [options]
//! fixture_peer client --connect 10.202.0.2 --source 10.201.0.2 [options]
//!
//! options
//!   --sessions N      sessions per scenario, default 10
//!   --seed S          run seed, default 42
//!   --var NAME=INT    replace an integer variable in every scenario that declares it
//! ```

use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::process::ExitCode;

use fps_backend_kernel::{ConnectConfig, Connector, KernelListener, KernelStream, ListenConfig};
use fps_flow::template::Filler;
use fps_flow::{Manifest, ModuleSet, Scenario, Value};
use fps_interp::{session_rng, Limits, OpenKind, OpenRequest, Opener, Session, SessionCtx, Side};
use fps_module_http1::Http1;

/// Opens kernel TCP sessions for the interpreter. The listener binds on the
/// first `accept` of each port and stays open for later sessions.
struct KernelOpener {
    server: IpAddr,
    source: Option<IpAddr>,
    connector: Connector,
    listeners: Vec<KernelListener>,
}

impl KernelOpener {
    async fn open_inner(&mut self, kind: OpenKind, port: u16) -> io::Result<KernelStream> {
        let to_io = |e: fps_backend_kernel::BackendError| io::Error::other(e.to_string());
        match kind {
            OpenKind::Connect => {
                let source = self.source.unwrap_or(IpAddr::from([0, 0, 0, 0]));
                self.connector
                    .connect(source, SocketAddr::new(self.server, port))
                    .await
                    .map_err(to_io)
            }
            OpenKind::Accept => {
                let addr = SocketAddr::new(self.server, port);
                let i = match self.listeners.iter().position(|l| l.local_addr() == addr) {
                    Some(i) => i,
                    None => {
                        let l = KernelListener::bind(addr, &ListenConfig::default())
                            .map_err(to_io)?;
                        self.listeners.push(l);
                        self.listeners.len() - 1
                    }
                };
                self.listeners[i].accept().await.map(|(s, _)| s).map_err(to_io)
            }
        }
    }
}

impl Opener for KernelOpener {
    type Io = KernelStream;

    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<KernelStream>> {
        let (kind, port) = (request.kind, request.port);
        self.open_inner(kind, port)
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
                let kv = args.next().ok_or("missing NAME=INT")?;
                let (k, v) = kv.split_once('=').ok_or("expected NAME=INT")?;
                let v: i64 = v.parse().map_err(|e| format!("bad value of {k}: {e}"))?;
                o.vars.push((k.to_string(), Value::Int(v)));
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
    let ctx = set.context("http1").ok_or("no http1 context")?;
    let mut out = Vec::new();
    for rel in &m.scenarios {
        let mut s =
            Scenario::load_with(&dir.join(rel), Some(&ctx)).map_err(|e| e.to_string())?;
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

async fn run(o: Options) -> Result<(), String> {
    let scenarios = load(&o.vars)?;
    let filler = Filler::new(o.seed);
    let mut opener = KernelOpener {
        server: o.server,
        source: o.source,
        connector: Connector::new(ConnectConfig::default()),
        listeners: Vec::new(),
    };
    let mut session_id = 0;
    let mut failures = 0;
    for (name, s) in &scenarios {
        let flow = &s.flows[0];
        let program = match o.side {
            Side::Client => &flow.client,
            Side::Server => flow.server.as_ref().ok_or("flow without server side")?,
        };
        for _ in 0..o.sessions {
            session_id += 1;
            let mut functions = Http1;
            let mut session = Session::new(SessionCtx {
                vars: &s.vars,
                filler: &filler,
                flow: &flow.id,
                side: o.side,
                session_id,
                rng: session_rng(o.seed, session_id, o.side),
                functions: &mut functions,
                limits: Limits::default(),
            });
            if let Err(f) = session
                .run_program(&mut opener, program, std::future::pending())
                .await
            {
                failures += 1;
                eprintln!("{name} session {session_id}: {f:?}");
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
