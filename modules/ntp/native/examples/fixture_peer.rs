// SPDX-License-Identifier: GPL-3.0-only
//! Runs one side of every ntp scenario over UDP sockets.
//!
//! `modules/ntp/fixtures/capture.sh` starts the server side in the server
//! namespace of the QA-01 harness and the client side in the client
//! namespace, and records the traffic between them as the module fixture.
//! Both sides walk the scenarios of the manifest in the same order and run
//! the sessions one after another, so the n-th session of one side meets the
//! n-th session of the other and both sides draw from the same session
//! generators. Timestamps come from the wall clock of each host.
//!
//! UDP flows run over Tokio UDP sockets, because the kernel UDP backend
//! (TRN-02) has not merged yet. The server binds its socket once at start,
//! and binds each session to the source address of its first datagram. Every
//! client session gets its own source port, one after another from
//! `FIRST_SOURCE_PORT` (MOD-21). The kernel would pick a port at random, and a
//! port that an earlier session released can come up again for the same
//! destination, which puts two sessions on one 5-tuple.
//!
//! ```text
//! fixture_peer server --listen 10.202.0.2 [options]
//! fixture_peer client --connect 10.202.0.2 --source 10.201.0.2 [options]
//!
//! options
//!   --sessions N       sessions per scenario, default 10
//!   --seed S           run seed, default 42
//!   --port P           server port of every flow, default 123
//!   --burst-gap-ms MS  pause between the packets of the iburst scenario,
//!                      default 2000
//!   --poll-ms MS       pause that stands for a poll exponent of 6, default 64000.
//!                      The pause of the exponent 7 is twice as long.
//! ```

use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::task::{Context, Poll};

use fps_flow::template::Filler;
use fps_flow::{Manifest, ModuleSet, Scenario, Value};
use fps_interp::{
    session_rng, DatagramOpener, Limits, OpenKind, OpenRequest, Session, SessionCtx, Side,
};
use fps_io::DatagramIo;
use fps_module_ntp::Ntp;
use tokio::io::ReadBuf;
use tokio::net::UdpSocket;

/// The first source port of the client sessions. It lies inside the default
/// ephemeral range of Linux, `net.ipv4.ip_local_port_range`.
const FIRST_SOURCE_PORT: u16 = 40_000;
/// The number of source ports that the client sessions use before they start over.
const SOURCE_PORTS: u16 = 20_000;

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
    connected: u16,
    bound: Option<Arc<UdpSocket>>,
}

impl UdpOpener {
    async fn open_inner(&mut self, kind: OpenKind, port: u16) -> io::Result<UdpSession> {
        match kind {
            OpenKind::Connect => {
                let source = self.source.unwrap_or(IpAddr::from([0, 0, 0, 0]));
                let local = FIRST_SOURCE_PORT + self.connected;
                self.connected = (self.connected + 1) % SOURCE_PORTS;
                let socket = UdpSocket::bind(SocketAddr::new(source, local)).await?;
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
    burst_gap_ms: u64,
    poll_ms: u64,
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
        port: 123,
        burst_gap_ms: 2_000,
        poll_ms: 64_000,
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
            "--burst-gap-ms" => o.burst_gap_ms = num(args.next())?,
            "--poll-ms" => o.poll_ms = num(args.next())?,
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(o)
}

fn load(o: &Options) -> Result<Vec<(String, Scenario)>, String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let m = Manifest::load(&dir.join("manifest.yaml")).map_err(|e| e.to_string())?;
    let mut set = ModuleSet::default();
    set.insert(m.clone());
    let ctx = set.context("ntp").ok_or("no ntp context")?;
    let int = |v: u64| Value::Int(i64::try_from(v).unwrap_or(i64::MAX));
    let vars = [
        ("server_port", Value::Int(o.port.into())),
        ("burst_gap_ms", int(o.burst_gap_ms)),
        ("poll_interval_ms", int(o.poll_ms)),
        ("poll_interval_long_ms", int(o.poll_ms.saturating_mul(2))),
    ];
    let mut out = Vec::new();
    for rel in &m.scenarios {
        let mut s = Scenario::load_with(&dir.join(rel), Some(&ctx)).map_err(|e| e.to_string())?;
        // A scenario takes only the variables that it declares.
        let own: Vec<(String, Value)> = vars
            .iter()
            .filter(|(k, _)| s.vars.contains_key(*k))
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect();
        s.override_vars(&own).map_err(|e| e.to_string())?;
        s.fold_constants().map_err(|e| e.to_string())?;
        out.push((rel.display().to_string(), s));
    }
    Ok(out)
}

async fn run(o: Options) -> Result<(), String> {
    let scenarios = load(&o)?;
    let filler = Filler::new(o.seed);
    let mut datagrams = UdpOpener {
        server: o.server,
        source: o.source,
        connected: 0,
        bound: None,
    };
    if o.side == Side::Server {
        let addr = SocketAddr::new(o.server, o.port);
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
                let mut fns = Ntp;
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
                let done = session
                    .run_datagram_program(&mut datagrams, program, std::future::pending())
                    .await;
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
            return ExitCode::FAILURE;
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
