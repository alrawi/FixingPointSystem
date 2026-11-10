// SPDX-License-Identifier: GPL-3.0-only
//! Runs one side of every sip_rtp scenario over kernel UDP sockets.
//!
//! `modules/sip_rtp/fixtures/capture.sh` starts the server side in the server
//! namespace of the QA-01 harness and the client side in the client
//! namespace, and records the traffic between them as the module fixture.
//! Both sides walk the scenarios of the manifest in the same order and run
//! the calls one after another, so the n-th INVITE on the server always
//! belongs to the n-th call of the client.
//!
//! Every call is a scenario instance, because the media flow comes from
//! `spawn`. The agent cannot run such an instance yet, because its runtime
//! opens no datagram sockets for the flows of an instance (AGT-16), and this
//! peer is the runner that stands in for it. UDP flows run over Tokio UDP
//! sockets, as the dns fixture peer does, because the kernel UDP backend
//! (TRN-02) cannot serve an instance either.
//!
//! The server binds the SIP port once at start. It binds the RTP port of a call
//! when the spawned media flow of its side starts, which happens before the 200
//! OK that announces the port, and it releases the port when the call ends. The
//! client opens a new socket for every flow, from a source port that counts up
//! from 40000, so no two sessions of a capture share a 5-tuple (the MOD-21
//! lesson), and it connects the socket to the server.
//!
//! ```text
//! fixture_peer server --listen 10.202.0.2 [options]
//! fixture_peer client --connect 10.202.0.2 --source 10.201.0.2 [options]
//!
//! options
//!   --sessions N      calls per scenario, default 10
//!   --seed S          run seed, default 42
//!   --var NAME=VALUE  replace a variable in every scenario that declares it
//! ```
//!
//! The server address becomes the variable `server_ip` of every scenario, and
//! the client address becomes `client_ip`, unless `--var` names another one,
//! so the SDP lines announce the hosts that carry the media.

use std::cell::Cell;
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use fps_flow::program::Flow;
use fps_flow::template::Filler;
use fps_flow::{Manifest, ModuleSet, Scenario, TemplateFn, Value};
use fps_interp::{
    DatagramOpener, FlowEnv, InstanceConfig, Limits, OpenKind, OpenRequest, Opener,
    ScenarioInstance, Side, Sides,
};
use fps_io::memory::MemoryIo;
use fps_io::DatagramIo;
use fps_module_sip_rtp::SipRtp;
use tokio::io::ReadBuf;
use tokio::net::UdpSocket;

/// One UDP session. A client session owns a connected socket. A server
/// session shares the bound SIP socket or owns the socket of its RTP port, answers
/// its peer, and ignores datagrams from other sources.
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

/// What every opener of the run shares.
struct Shared {
    server: IpAddr,
    source: Option<IpAddr>,
    /// The SIP socket of the server, bound once.
    sip: Option<Arc<UdpSocket>>,
    sip_port: u16,
    /// The next client source port.
    next_port: Cell<u16>,
}

/// Opens UDP sessions for one flow side.
struct UdpOpener {
    shared: Rc<Shared>,
}

fn is_busy(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::AddrInUse
}

impl UdpOpener {
    async fn open_inner(&mut self, kind: OpenKind, port: u16) -> io::Result<UdpSession> {
        let sh = &self.shared;
        match kind {
            OpenKind::Connect => {
                let source = sh.source.unwrap_or(IpAddr::from([0, 0, 0, 0]));
                let socket = loop {
                    let p = sh.next_port.get();
                    sh.next_port.set(if p >= 60000 { 40000 } else { p + 1 });
                    match UdpSocket::bind(SocketAddr::new(source, p)).await {
                        Ok(s) => break s,
                        Err(e) if is_busy(&e) => continue,
                        Err(e) => return Err(e),
                    }
                };
                socket.connect(SocketAddr::new(sh.server, port)).await?;
                Ok(UdpSession {
                    socket: Arc::new(socket),
                    peer: None,
                    pending: None,
                    closed: false,
                })
            }
            OpenKind::Accept => {
                let socket = if port == sh.sip_port {
                    sh.sip.clone().ok_or_else(|| {
                        io::Error::new(io::ErrorKind::NotConnected, "no sip socket")
                    })?
                } else {
                    // The RTP port of a call. The previous call may still hold it for a moment.
                    let addr = SocketAddr::new(sh.server, port);
                    let mut tries = 0;
                    loop {
                        match UdpSocket::bind(addr).await {
                            Ok(s) => break Arc::new(s),
                            Err(e) if is_busy(&e) && tries < 100 => {
                                tries += 1;
                                tokio::time::sleep(Duration::from_millis(20)).await;
                            }
                            Err(e) => return Err(e),
                        }
                    }
                };
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

/// The scenarios have no TCP flow, so this opener refuses every open.
struct NoStreams;

impl Opener for NoStreams {
    type Io = MemoryIo;

    async fn open(&mut self, _request: OpenRequest<'_>) -> io::Result<MemoryIo> {
        Err(io::Error::other("the sip_rtp module has no TCP flow"))
    }
}

struct Env {
    shared: Rc<Shared>,
}

impl FlowEnv for Env {
    type Stream = NoStreams;
    type Datagram = UdpOpener;

    fn stream(&mut self, _flow: &Flow, _side: Side) -> NoStreams {
        NoStreams
    }

    fn datagram(&mut self, _flow: &Flow, _side: Side) -> UdpOpener {
        UdpOpener {
            shared: Rc::clone(&self.shared),
        }
    }

    /// Every flow side gets its own functions, because the RTP stream keeps
    /// its sequence number in them.
    fn functions(&mut self, _flow: &Flow, _side: Side) -> Box<dyn TemplateFn> {
        Box::new(SipRtp::default())
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
    let ctx = set.context("sip_rtp").ok_or("no sip_rtp context")?;
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
    if let (Some(source), false) = (o.source, o.vars.iter().any(|(k, _)| k == "client_ip")) {
        o.vars
            .push(("client_ip".into(), Value::Str(source.to_string())));
    }
    let scenarios = load(&o.vars)?;
    let sip_port = scenarios
        .first()
        .and_then(|(_, s)| s.vars.get("sip_port"))
        .and_then(Value::as_i64)
        .and_then(|p| u16::try_from(p).ok())
        .ok_or("the scenarios have no valid sip_port")?;
    if scenarios
        .iter()
        .any(|(_, s)| s.vars.get("sip_port").and_then(Value::as_i64) != Some(i64::from(sip_port)))
    {
        return Err("the scenarios disagree on sip_port".into());
    }
    let sip = if o.side == Side::Server {
        let addr = SocketAddr::new(o.server, sip_port);
        let socket = UdpSocket::bind(addr)
            .await
            .map_err(|e| format!("bind {addr}: {e}"))?;
        println!("listening on {addr}");
        Some(Arc::new(socket))
    } else {
        None
    };
    let shared = Rc::new(Shared {
        server: o.server,
        source: o.source,
        sip,
        sip_port,
        next_port: Cell::new(40000),
    });
    let mut env = Env { shared };
    let filler = Filler::new(o.seed);
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
            // Both sides step the ids by the same amount, so the n-th call
            // has the same ids on the client and the server.
            first_session_id += 100;
            if let Err(e) = instance
                .run(&mut env, &filler, std::future::pending())
                .await
            {
                failures += 1;
                eprintln!("{name} call {}: {e}", first_session_id / 100);
            }
        }
        println!("{name}: {} calls", o.sessions);
    }
    if failures > 0 {
        return Err(format!("{failures} calls failed"));
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
