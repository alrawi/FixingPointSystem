// SPDX-License-Identifier: GPL-3.0-only
//! Shared helpers that load the dns scenarios and run both sides of every
//! flow against each other, UDP flows over the in memory datagram pair and
//! TCP flows over the in memory duplex of `fps-io`.
#![allow(dead_code)] // Each test file uses a different subset of the helpers.

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use fps_flow::program::L4;
use fps_flow::template::Filler;
use fps_flow::{Manifest, ModuleSet, Scenario, Value, Vars};
use fps_interp::{
    session_rng, DatagramOpener, Failure, Limits, OpenKind, OpenRequest, Opener, Session,
    SessionCtx, Side, SideStats,
};
use fps_io::memory::{datagram_pair, duplex, MemoryDatagram};
use fps_io::{DatagramIo, SessionIo};
use fps_module_dns::Dns;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// The run seed of every transcript.
pub const SEED: u64 = 42;

/// The session id of the first flow of every transcript.
pub const SESSION_ID: u64 = 1;

/// The module directory, `modules/dns`.
pub fn module_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Reads the module manifest.
pub fn manifest() -> Manifest {
    Manifest::load(&module_dir().join("manifest.yaml")).unwrap()
}

/// Loads every scenario that the manifest lists, checking module calls
/// against the manifest catalog, and folds constants as the engine does once
/// per run. Each entry carries the scenario file stem.
pub fn scenarios() -> Vec<(String, Scenario)> {
    let m = manifest();
    let mut set = ModuleSet::default();
    set.insert(m.clone());
    let ctx = set.context("dns").unwrap();
    m.scenarios
        .iter()
        .map(|rel| {
            let path = module_dir().join(rel);
            let mut s = Scenario::load_with(&path, Some(&ctx))
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            s.fold_constants().unwrap();
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            (stem, s)
        })
        .collect()
}

/// A stream wrapper that records the bytes written through it.
pub struct Tap<S> {
    inner: S,
    written: Arc<Mutex<Vec<u8>>>,
}

impl<S: SessionIo> AsyncRead for Tap<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<S: SessionIo> AsyncWrite for Tap<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let r = Pin::new(&mut this.inner).poll_write(cx, data);
        if let Poll::Ready(Ok(n)) = r {
            this.written.lock().unwrap().extend_from_slice(&data[..n]);
        }
        r
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

impl<S: SessionIo> SessionIo for Tap<S> {
    fn abort(&mut self) {
        self.inner.abort();
    }
}

/// A datagram wrapper that records every datagram sent through it.
pub struct DatagramTap {
    inner: MemoryDatagram,
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl DatagramIo for DatagramTap {
    fn poll_send(&mut self, cx: &mut Context<'_>, datagram: &[u8]) -> Poll<io::Result<()>> {
        let r = self.inner.poll_send(cx, datagram);
        if let Poll::Ready(Ok(())) = r {
            self.sent.lock().unwrap().push(datagram.to_vec());
        }
        r
    }

    fn poll_recv(&mut self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        self.inner.poll_recv(cx, buf)
    }

    fn close(&mut self) {
        self.inner.close();
    }
}

/// Hands out one prepared stream.
pub struct OneShot<S>(Option<S>);

impl<S: SessionIo> Opener for OneShot<S> {
    type Io = S;

    fn open(&mut self, _request: OpenRequest<'_>) -> impl Future<Output = io::Result<S>> {
        let io = self.0.take();
        async move { io.ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "used")) }
    }
}

/// Hands out one end of the datagram pair. For `accept` it waits for the
/// first datagram and leaves it queued, as the datagram rules require.
pub struct OneDatagram(Option<DatagramTap>);

impl DatagramOpener for OneDatagram {
    type Io = DatagramTap;

    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<DatagramTap>> {
        let io = self.0.take();
        let accept = request.kind == OpenKind::Accept;
        async move {
            let io = io.ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "used"))?;
            if accept {
                io.inner.readable().await?;
            }
            Ok(io)
        }
    }
}

/// The messages that one side of a flow wrote: the datagrams of a UDP flow,
/// or the byte stream of a TCP flow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Wire {
    /// One entry per datagram.
    Datagrams(Vec<Vec<u8>>),
    /// The stream bytes.
    Stream(Vec<u8>),
}

impl Wire {
    /// Returns the DNS messages, splitting a TCP stream at its length
    /// prefixes.
    pub fn messages(&self) -> Vec<Vec<u8>> {
        match self {
            Wire::Datagrams(d) => d.clone(),
            Wire::Stream(stream) => {
                let mut s: &[u8] = stream;
                let mut out = Vec::new();
                while !s.is_empty() {
                    let n = usize::from(u16::from_be_bytes([s[0], s[1]]));
                    out.push(s[2..2 + n].to_vec());
                    s = &s[2 + n..];
                }
                out
            }
        }
    }

    /// Returns the transcript bytes. Datagrams carry a two byte length
    /// prefix each, so the file keeps their boundaries.
    pub fn transcript(&self) -> Vec<u8> {
        match self {
            Wire::Datagrams(d) => d
                .iter()
                .flat_map(|g| (g.len() as u16).to_be_bytes().into_iter().chain(g.clone()))
                .collect(),
            Wire::Stream(s) => s.clone(),
        }
    }
}

/// The outcome of both sides of one flow and what each side wrote.
#[derive(Debug)]
pub struct FlowRun {
    pub flow: String,
    pub l4: L4,
    pub client: Result<(), Failure>,
    pub server: Result<(), Failure>,
    pub client_stats: SideStats,
    pub server_stats: SideStats,
    pub client_vars: Vars,
    pub c2s: Wire,
    pub s2c: Wire,
}

fn ctx<'a>(
    s: &'a Scenario,
    filler: &'a Filler,
    flow: &'a str,
    side: Side,
    seed: u64,
    session_id: u64,
    functions: &'a mut Dns,
) -> SessionCtx<'a> {
    SessionCtx {
        vars: &s.vars,
        filler,
        flow,
        side,
        session_id,
        rng: session_rng(seed, session_id, side),
        functions,
        limits: Limits::default(),
    }
}

/// Runs every flow of `s` in order, the n-th flow with session id
/// `first_id + n`, with the bundled zone on the server side.
pub async fn run(s: &Scenario, seed: u64, first_id: u64) -> Vec<FlowRun> {
    run_with(s, seed, first_id, &Dns::bundled().unwrap()).await
}

/// Like [`run`], with the functions that `dns` holds, so a test can serve a
/// zone that the module settings configured.
pub async fn run_with(s: &Scenario, seed: u64, first_id: u64, dns: &Dns) -> Vec<FlowRun> {
    let filler = Filler::new(seed);
    let mut out = Vec::new();
    for (n, flow) in s.flows.iter().enumerate() {
        let id = first_id + n as u64;
        let mut client_fns = dns.clone();
        let mut server_fns = client_fns.clone();
        let mut client = Session::new(ctx(
            s,
            &filler,
            &flow.id,
            Side::Client,
            seed,
            id,
            &mut client_fns,
        ));
        let mut server = Session::new(ctx(
            s,
            &filler,
            &flow.id,
            Side::Server,
            seed,
            id,
            &mut server_fns,
        ));
        let server_program = flow.server.as_ref().unwrap();
        let (c, sv, c2s, s2c) = match flow.l4 {
            L4::Udp => {
                let (a, b) = datagram_pair(16);
                let (ca, cb) = (Arc::default(), Arc::default());
                let mut co = OneDatagram(Some(DatagramTap {
                    inner: a,
                    sent: Arc::clone(&ca),
                }));
                let mut so = OneDatagram(Some(DatagramTap {
                    inner: b,
                    sent: Arc::clone(&cb),
                }));
                let (c, sv) = tokio::join!(
                    client.run_datagram_program(&mut co, &flow.client, std::future::pending()),
                    server.run_datagram_program(&mut so, server_program, std::future::pending()),
                );
                let c2s = Wire::Datagrams(std::mem::take(&mut *ca.lock().unwrap()));
                let s2c = Wire::Datagrams(std::mem::take(&mut *cb.lock().unwrap()));
                (c, sv, c2s, s2c)
            }
            _ => {
                let (a, b) = duplex(64 * 1024);
                let (ca, cb) = (Arc::default(), Arc::default());
                let mut co = OneShot(Some(Tap {
                    inner: a,
                    written: Arc::clone(&ca),
                }));
                let mut so = OneShot(Some(Tap {
                    inner: b,
                    written: Arc::clone(&cb),
                }));
                let (c, sv) = tokio::join!(
                    client.run_program(&mut co, &flow.client, std::future::pending()),
                    server.run_program(&mut so, server_program, std::future::pending()),
                );
                let c2s = Wire::Stream(std::mem::take(&mut *ca.lock().unwrap()));
                let s2c = Wire::Stream(std::mem::take(&mut *cb.lock().unwrap()));
                (c, sv, c2s, s2c)
            }
        };
        out.push(FlowRun {
            flow: flow.id.clone(),
            l4: flow.l4,
            client: c,
            server: sv,
            client_stats: client.stats(),
            server_stats: server.stats(),
            client_vars: client.overlay().clone(),
            c2s,
            s2c,
        });
    }
    out
}

/// Returns the text of a variable that a `set` action bound.
pub fn text(vars: &Vars, name: &str) -> Option<String> {
    vars.get(name).map(|v| match v {
        Value::Str(t) => t.clone(),
        other => format!("{other:?}"),
    })
}
