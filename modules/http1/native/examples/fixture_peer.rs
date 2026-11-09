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
//! The https scenarios upgrade their sessions to TLS (MOD-04). The agent builds
//! no TLS profiles yet (AGT-14), so the peers build them with
//! `tests/common/tls.rs`. The server generates a run authority, issues the
//! certificate for `www.example.test`, and writes the authority certificate to
//! the file of `--ca`. The client waits for that file, trusts only that
//! authority, and appends the secrets of every handshake to the file of
//! `--keylog` in the NSS format, which `capture.sh` injects into the capture.
//!
//! ```text
//! fixture_peer server --listen 10.202.0.2 --ca ca.pem [options]
//! fixture_peer client --connect 10.202.0.2 --source 10.201.0.2 --ca ca.pem --keylog keys.log [options]
//!
//! options
//!   --sessions N      sessions per scenario, default 10
//!   --seed S          run seed, default 42
//!   --var NAME=INT    replace an integer variable in every scenario that declares it
//!   --ca FILE         the authority certificate in PEM form, which the server writes and the client reads
//!   --keylog FILE     the key log of the client, default none (the secrets stay in memory)
//! ```

use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use fps_backend_kernel::{ConnectConfig, Connector, KernelListener, KernelStream, ListenConfig};
use fps_flow::template::Filler;
use fps_flow::{Manifest, ModuleSet, Scenario, Value};
use fps_interp::{session_rng, Limits, OpenKind, OpenRequest, Opener, Session, SessionCtx, Side};
use fps_module_http1::Http1;
use fps_module_tls::NssKeyLog;

#[path = "../../tests/common/tls.rs"]
mod tls;

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
    /// session, because the client starts a scenario on its own port as soon as
    /// the scenario before it ends, and the server may not have reached it yet.
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
    ca: Option<PathBuf>,
    keylog: Option<PathBuf>,
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
        ca: None,
        keylog: None,
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
            "--ca" | "--keylog" => {
                let path = PathBuf::from(args.next().ok_or("missing file name")?);
                if flag == "--ca" {
                    o.ca = Some(path);
                } else {
                    o.keylog = Some(path);
                }
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

/// Builds the TLS profiles of this side, which the https scenarios name.
async fn tls_profiles(o: &Options) -> Result<tls::Profiles, String> {
    let profiles = tls::Profiles::default();
    match o.side {
        Side::Server => {
            let authority = tls::authority().map_err(|e| e.to_string())?;
            let identity = tls::identity(&authority).map_err(|e| e.to_string())?;
            if let Some(ca) = &o.ca {
                // Write and rename, so the client never reads half a certificate.
                let partial = ca.with_extension("partial");
                std::fs::write(&partial, authority.ca_pem())
                    .and_then(|()| std::fs::rename(&partial, ca))
                    .map_err(|e| format!("write {}: {e}", ca.display()))?;
            }
            profiles.with_servers(&identity).map_err(|e| e.to_string())
        }
        Side::Client => {
            let ca =
                o.ca.as_ref()
                    .ok_or("the client needs --ca for the https scenarios")?;
            let mut pem = None;
            for _ in 0..100 {
                if let Ok(bytes) = std::fs::read(ca) {
                    pem = Some(bytes);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let pem = pem.ok_or_else(|| format!("{} did not appear in 10 s", ca.display()))?;
            let roots = tls::roots_from_pem(&pem)?;
            let key_log = match &o.keylog {
                Some(path) => {
                    NssKeyLog::to_file(path).map_err(|e| format!("open {}: {e}", path.display()))?
                }
                None => NssKeyLog::in_memory(),
            };
            profiles
                .with_clients(&roots, &Arc::new(key_log))
                .map_err(|e| e.to_string())
        }
    }
}

async fn run(o: Options) -> Result<(), String> {
    let scenarios = load(&o.vars)?;
    let profiles = tls_profiles(&o).await?;
    let filler = Filler::new(o.seed);
    let mut opener = KernelOpener {
        server: o.server,
        source: o.source,
        connector: Connector::new(ConnectConfig::default()),
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
            session.set_tls_upgrader(&profiles);
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
