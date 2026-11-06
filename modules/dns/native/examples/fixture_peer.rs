// SPDX-License-Identifier: GPL-3.0-only
//! Runs one side of every dns scenario over kernel sockets.
//!
//! `modules/dns/fixtures/capture.sh` starts the server side in the server
//! namespace of the QA-01 harness and the client side in the client
//! namespace, and records the traffic between them as the module fixture.
//! Both sides walk the scenarios of the manifest in the same order, run the
//! flows of each scenario in order, and run the sessions one after another,
//! so the n-th session of one side meets the n-th session of the other and
//! both sides draw from the same session generators.
//!
//! TCP flows run over `fps-backend-kernel`. UDP flows run over Tokio UDP
//! sockets, because the kernel UDP backend (TRN-02) has not merged yet. The
//! client connects one socket per session from an ephemeral port, as a stub
//! resolver does. The server binds its UDP socket and TCP listener once at
//! start, so a query never meets a closed port, and binds each UDP session
//! to the source address of its first datagram.
//!
//! ```text
//! fixture_peer server --listen 10.202.0.2 [options]
//! fixture_peer client --connect 10.202.0.2 --source 10.201.0.2 [options]
//!
//! options
//!   --sessions N      sessions per scenario, default 10
//!   --seed S          run seed, default 42
//!   --port P          server port of every flow, default 53
//!   --settings JSON   module settings as `modules.dns.settings` holds them, for example
//!                     '{"zone_file":"zones/example.test.zone"}'. A relative zone_file
//!                     resolves against the module directory. Default: no settings, so
//!                     the server answers from the bundled zone.
//!   --zone FILE       shorthand for --settings with that zone_file, resolved against the
//!                     current directory
//! ```

use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::task::{Context, Poll};

use fps_backend_kernel::{ConnectConfig, Connector, KernelListener, KernelStream, ListenConfig};
use fps_flow::program::L4;
use fps_flow::template::Filler;
use fps_flow::{Manifest, ModuleSet, Scenario};
use fps_interp::{
    session_rng, DatagramOpener, Limits, OpenKind, OpenRequest, Opener, Session, SessionCtx, Side,
};
use fps_io::DatagramIo;
use fps_module_dns::Dns;
use tokio::io::ReadBuf;
use tokio::net::UdpSocket;

/// Opens kernel TCP sessions. The server listener binds at start.
struct StreamOpener {
    server: IpAddr,
    source: Option<IpAddr>,
    connector: Connector,
    listener: Option<KernelListener>,
}

impl StreamOpener {
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
                let l = self.listener.as_ref().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotConnected, "no tcp listener")
                })?;
                if l.local_addr().port() != port {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("flow port {port} differs from --port"),
                    ));
                }
                l.accept().await.map(|(s, _)| s).map_err(to_io)
            }
        }
    }
}

impl Opener for StreamOpener {
    type Io = KernelStream;

    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<KernelStream>> {
        self.open_inner(request.kind, request.port)
    }
}

/// One UDP session. A client session owns a connected socket. A server
/// session shares the bound socket, answers its peer, and ignores
/// datagrams from other sources.
struct UdpSession {
    socket: Arc<UdpSocket>,
    /// The peer of a server session, `None` for a connected client socket.
    peer: Option<SocketAddr>,
    /// The first datagram of a server session, which `accept` read.
    pending: Option<Vec<u8>>,
    closed: bool,
}

fn not_connected() -> io::Error {
    io::Error::new(io::ErrorKind::NotConnected, "the udp session was closed")
}

impl DatagramIo for UdpSession {
    fn poll_send(&mut self, cx: &mut Context<'_>, datagram: &[u8]) -> Poll<io::Result<()>> {
        if self.closed {
            return Poll::Ready(Err(not_connected()));
        }
        let sent = match self.peer {
            Some(peer) => self.socket.poll_send_to(cx, datagram, peer),
            None => self.socket.poll_send(cx, datagram),
        };
        sent.map_ok(|_| ())
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        if self.closed {
            return Poll::Ready(Err(not_connected()));
        }
        if let Some(d) = self.pending.take() {
            let n = d.len().min(buf.len());
            buf[..n].copy_from_slice(&d[..n]);
            return Poll::Ready(Ok(d.len()));
        }
        loop {
            let mut rb = ReadBuf::new(buf);
            match self.peer {
                None => {
                    return self
                        .socket
                        .poll_recv(cx, &mut rb)
                        .map_ok(|()| rb.filled().len());
                }
                Some(peer) => match self.socket.poll_recv_from(cx, &mut rb) {
                    Poll::Ready(Ok(from)) if from == peer => {
                        return Poll::Ready(Ok(rb.filled().len()))
                    }
                    Poll::Ready(Ok(_)) => continue,
                    other => return other.map_ok(|_| 0),
                },
            }
        }
    }

    fn close(&mut self) {
        self.closed = true;
    }
}

/// Opens UDP sessions. The server socket binds at start.
struct UdpOpener {
    server: IpAddr,
    source: Option<IpAddr>,
    bound: Option<Arc<UdpSocket>>,
}

impl UdpOpener {
    async fn open_inner(&mut self, kind: OpenKind, port: u16) -> io::Result<UdpSession> {
        match kind {
            OpenKind::Connect => {
                let source = self.source.unwrap_or(IpAddr::from([0, 0, 0, 0]));
                let socket = UdpSocket::bind(SocketAddr::new(source, 0)).await?;
                socket.connect(SocketAddr::new(self.server, port)).await?;
                Ok(UdpSession {
                    socket: Arc::new(socket),
                    peer: None,
                    pending: None,
                    closed: false,
                })
            }
            OpenKind::Accept => {
                let socket = self
                    .bound
                    .clone()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "no udp socket"))?;
                if socket.local_addr()?.port() != port {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("flow port {port} differs from --port"),
                    ));
                }
                let mut buf = vec![0u8; fps_io::MAX_DATAGRAM];
                let (n, peer) = socket.recv_from(&mut buf).await?;
                buf.truncate(n);
                Ok(UdpSession {
                    socket,
                    peer: Some(peer),
                    pending: Some(buf),
                    closed: false,
                })
            }
        }
    }
}

impl DatagramOpener for UdpOpener {
    type Io = UdpSession;

    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<UdpSession>> {
        self.open_inner(request.kind, request.port)
    }
}

struct Options {
    side: Side,
    server: IpAddr,
    source: Option<IpAddr>,
    sessions: u64,
    seed: u64,
    port: u16,
    settings: serde_json::Value,
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
        port: 53,
        settings: serde_json::json!({}),
    };
    let ip = |s: Option<String>| -> Result<IpAddr, String> {
        s.ok_or("missing address")?
            .parse()
            .map_err(|e| format!("bad address: {e}"))
    };
    let num = |s: Option<String>| -> Result<u64, String> {
        s.ok_or("missing number")?
            .parse()
            .map_err(|e| format!("bad number: {e}"))
    };
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--listen" | "--connect" => o.server = ip(args.next())?,
            "--source" => o.source = Some(ip(args.next())?),
            "--sessions" => o.sessions = num(args.next())?,
            "--seed" => o.seed = num(args.next())?,
            "--port" => {
                o.port = u16::try_from(num(args.next())?).map_err(|_| "port above 65535")?;
            }
            "--settings" => {
                let text = args.next().ok_or("missing settings")?;
                o.settings = serde_json::from_str(&text)
                    .map_err(|e| format!("--settings is not valid JSON: {e}"))?;
            }
            "--zone" => {
                let file: PathBuf = args.next().ok_or("missing zone file")?.into();
                let file = std::path::absolute(&file).map_err(|e| e.to_string())?;
                o.settings = serde_json::json!({ "zone_file": file });
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(o)
}

fn load(port: u16) -> Result<Vec<(String, Scenario)>, String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let m = Manifest::load(&dir.join("manifest.yaml")).map_err(|e| e.to_string())?;
    let mut set = ModuleSet::default();
    set.insert(m.clone());
    let ctx = set.context("dns").ok_or("no dns context")?;
    let mut out = Vec::new();
    for rel in &m.scenarios {
        let mut s = Scenario::load_with(&dir.join(rel), Some(&ctx)).map_err(|e| e.to_string())?;
        let port = vec![("server_port".to_string(), fps_flow::Value::Int(port.into()))];
        s.override_vars(&port).map_err(|e| e.to_string())?;
        s.fold_constants().map_err(|e| e.to_string())?;
        out.push((rel.display().to_string(), s));
    }
    Ok(out)
}

async fn run(o: Options) -> Result<(), String> {
    let scenarios = load(o.port)?;
    let module_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let functions = Dns::from_settings(&o.settings, &module_dir).map_err(|e| e.to_string())?;
    let filler = Filler::new(o.seed);
    let mut streams = StreamOpener {
        server: o.server,
        source: o.source,
        connector: Connector::new(ConnectConfig::default()),
        listener: None,
    };
    let mut datagrams = UdpOpener {
        server: o.server,
        source: o.source,
        bound: None,
    };
    if o.side == Side::Server {
        let addr = SocketAddr::new(o.server, o.port);
        streams.listener =
            Some(KernelListener::bind(addr, &ListenConfig::default()).map_err(|e| e.to_string())?);
        datagrams.bound = Some(Arc::new(
            UdpSocket::bind(addr).await.map_err(|e| e.to_string())?,
        ));
        println!("listening on {addr}");
    }
    let mut session_id = 0;
    let mut failures = 0;
    for (name, s) in &scenarios {
        for _ in 0..o.sessions {
            for flow in &s.flows {
                session_id += 1;
                let program = match o.side {
                    Side::Client => &flow.client,
                    Side::Server => flow.server.as_ref().ok_or("flow without server side")?,
                };
                let mut fns = functions.clone();
                let mut session = Session::new(SessionCtx {
                    vars: &s.vars,
                    filler: &filler,
                    flow: &flow.id,
                    side: o.side,
                    session_id,
                    rng: session_rng(o.seed, session_id, o.side),
                    functions: &mut fns,
                    limits: Limits::default(),
                });
                let done = match flow.l4 {
                    L4::Udp => {
                        session
                            .run_datagram_program(&mut datagrams, program, std::future::pending())
                            .await
                    }
                    _ => {
                        session
                            .run_program(&mut streams, program, std::future::pending())
                            .await
                    }
                };
                if let Err(f) = done {
                    failures += 1;
                    eprintln!("{name} session {session_id}: {f:?}");
                }
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
