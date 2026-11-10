// SPDX-License-Identifier: GPL-3.0-only
//! Runs one side of every smb2 scenario over kernel sockets.
//!
//! `modules/smb2/fixtures/capture.sh` starts the server side in the server
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

use fps_backend_kernel::{
    ConnectConfig, Connector, KernelListener, KernelStream, ListenConfig, PortRange,
};
use fps_flow::template::Filler;
use fps_flow::{Manifest, ModuleSet, Scenario, Value};
use fps_interp::{session_rng, Limits, OpenKind, OpenRequest, Opener, Session, SessionCtx, Side};
use fps_module_smb2::Smb2;

/// The first source port of the client sessions. It lies inside the default
/// ephemeral range of Linux, `net.ipv4.ip_local_port_range`.
const FIRST_SOURCE_PORT: u16 = 40_000;
/// The number of source ports that the client sessions use before they start over.
const SOURCE_PORTS: u16 = 20_000;

/// Opens kernel TCP sessions for the interpreter. The listener binds on the
/// first `accept` of each port and stays open for later sessions.
///
/// Every client session gets its own source port, one after another from
/// [`FIRST_SOURCE_PORT`]. The kernel would pick a port at random, and a port
/// that an earlier session used and released can come up again for the same
/// destination. The capture then holds two sessions with one 5-tuple, which
/// the session table and the reference parsers see as one connection that
/// restarts, and Zeek reports it as a parser error (MOD-21). Own ports keep
/// every 5-tuple of a capture unique.
struct KernelOpener {
    server: IpAddr,
    source: Option<IpAddr>,
    connected: u16,
    listeners: Vec<KernelListener>,
}

impl KernelOpener {
    async fn open_inner(&mut self, kind: OpenKind, port: u16) -> io::Result<KernelStream> {
        let to_io = |e: fps_backend_kernel::BackendError| io::Error::other(e.to_string());
        match kind {
            OpenKind::Connect => {
                let source = self.source.unwrap_or(IpAddr::from([0, 0, 0, 0]));
                let local = FIRST_SOURCE_PORT + self.connected;
                self.connected = (self.connected + 1) % SOURCE_PORTS;
                let config = ConnectConfig {
                    local_ports: Some(PortRange::new(local, local).map_err(to_io)?),
                    ..ConnectConfig::default()
                };
                Connector::new(config)
                    .connect(source, SocketAddr::new(self.server, port))
                    .await
                    .map_err(to_io)
            }
            OpenKind::Accept => {
                let i = self.listen(port)?;
                self.listeners[i]
                    .accept()
                    .await
                    .map(|(s, _)| s)
                    .map_err(to_io)
            }
        }
    }

    /// Binds the listener of `port` unless it exists, and returns its index.
    /// The server binds every port of every scenario before the first
    /// session, because the client starts the next session as soon as the
    /// session before it ends, and the server may not have reached its
    /// `accept` yet.
    fn listen(&mut self, port: u16) -> io::Result<usize> {
        let to_io = |e: fps_backend_kernel::BackendError| io::Error::other(e.to_string());
        let addr = SocketAddr::new(self.server, port);
        match self.listeners.iter().position(|l| l.local_addr() == addr) {
            Some(i) => Ok(i),
            None => {
                let l = KernelListener::bind(addr, &ListenConfig::default()).map_err(to_io)?;
                self.listeners.push(l);
                Ok(self.listeners.len() - 1)
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
    let ctx = set.context("smb2").ok_or("no smb2 context")?;
    let mut out = Vec::new();
    for rel in &m.scenarios {
        let mut s = Scenario::load_with(&dir.join(rel), Some(&ctx)).map_err(|e| e.to_string())?;
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
        connected: 0,
        listeners: Vec::new(),
    };
    if o.side == Side::Server {
        for (_, s) in &scenarios {
            if let Some(Value::Int(port)) = s.vars.get("server_port") {
                let port = u16::try_from(*port).map_err(|e| format!("server_port: {e}"))?;
                opener
                    .listen(port)
                    .map_err(|e| format!("listen on {port}: {e}"))?;
            }
        }
    }
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
            let mut functions = Smb2;
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
