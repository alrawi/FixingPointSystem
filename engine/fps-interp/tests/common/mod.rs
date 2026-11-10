// SPDX-License-Identifier: GPL-3.0-only
//! Shared helpers that run a client and a server program against each other
//! over the in memory duplex of `fps-io`.
#![allow(dead_code)] // Each test file uses a different subset of the helpers.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use fps_flow::function::NoFunctions;
use fps_flow::template::Filler;
use fps_flow::{FnCall, FnError, Scenario, TemplateFn};
use fps_interp::{
    session_rng, Failure, Limits, OpenRequest, Opener, Session, SessionCtx, Side, SideStats,
};
use fps_io::memory::{duplex, MemoryIo};
use fps_io::SessionIo;
use rand::RngCore;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Compiles scenario text and folds its constants, as the engine does once
/// per run.
pub fn compile(text: &str) -> Scenario {
    let mut s = Scenario::parse(text).unwrap();
    s.fold_constants().unwrap();
    s
}

/// A transport wrapper that records written bytes and can fail reads.
#[derive(Debug)]
pub struct Tap<S> {
    inner: S,
    written: Arc<Mutex<Vec<u8>>>,
    fail_reads: Option<io::ErrorKind>,
}

impl<S> Tap<S> {
    pub fn new(inner: S) -> (Self, Arc<Mutex<Vec<u8>>>) {
        let written = Arc::new(Mutex::new(Vec::new()));
        (
            Tap {
                inner,
                written: Arc::clone(&written),
                fail_reads: None,
            },
            written,
        )
    }

    pub fn failing_reads(inner: S, kind: io::ErrorKind) -> Self {
        Tap {
            inner,
            written: Arc::default(),
            fail_reads: Some(kind),
        }
    }
}

impl<S: SessionIo> AsyncRead for Tap<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let Some(kind) = this.fail_reads {
            return Poll::Ready(Err(io::Error::new(kind, "tls alert handshake_failure")));
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
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

/// Hands out one prepared transport and records the request.
pub struct OneShot<S> {
    io: Option<S>,
    pub requests: Vec<(fps_interp::OpenKind, Option<Vec<u8>>, u16)>,
}

impl<S> OneShot<S> {
    pub fn new(io: S) -> Self {
        OneShot {
            io: Some(io),
            requests: Vec::new(),
        }
    }
}

impl<S: SessionIo> Opener for OneShot<S> {
    type Io = S;

    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<S>> {
        self.requests
            .push((request.kind, request.host.map(<[u8]>::to_vec), request.port));
        let io = self.io.take();
        async move { io.ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "used")) }
    }
}

/// An opener that fails with the given error kind.
pub struct Failing(pub io::ErrorKind);

impl Opener for Failing {
    type Io = MemoryIo;

    fn open(&mut self, _request: OpenRequest<'_>) -> impl Future<Output = io::Result<MemoryIo>> {
        let kind = self.0;
        async move { Err(io::Error::new(kind, "open failed")) }
    }
}

/// An opener whose open never completes, like a dial to a silent address.
pub struct Hanging;

impl Opener for Hanging {
    type Io = MemoryIo;

    fn open(&mut self, _request: OpenRequest<'_>) -> impl Future<Output = io::Result<MemoryIo>> {
        std::future::pending()
    }
}

/// A module `text` with one function `upper`, which renders its first
/// argument in upper case.
pub struct Upper;

impl TemplateFn for Upper {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        _rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        match (call.module, call.function, call.args.positional) {
            ("text", "upper", [fps_flow::ArgValue::Bytes(b)]) => {
                out.extend(b.iter().map(u8::to_ascii_uppercase));
                Ok(())
            }
            ("text", "upper", [fps_flow::ArgValue::Text(t)]) => {
                out.extend(t.bytes().map(|b| b.to_ascii_uppercase()));
                Ok(())
            }
            _ => Err(FnError::new(
                fps_flow::FnErrorKind::Unsupported,
                "unknown function",
            )),
        }
    }
}

/// Options of one paired run.
pub struct Opts {
    pub capacity: usize,
    pub seed: u64,
    pub session_id: u64,
    pub client_limits: Limits,
    pub server_limits: Limits,
    pub client_stop: Option<Duration>,
    pub client_functions: Box<dyn TemplateFn>,
    /// The clock that both sides read for `now()`, the wall clock by default.
    pub clock: Option<Arc<dyn fps_flow::Clock>>,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            capacity: 8192,
            seed: 42,
            session_id: 7,
            client_limits: Limits::default(),
            server_limits: Limits::default(),
            client_stop: None,
            client_functions: Box::new(NoFunctions),
            clock: None,
        }
    }
}

/// The outcome of both sides.
#[derive(Debug)]
pub struct Pair {
    pub client: Result<(), Failure>,
    pub server: Result<(), Failure>,
    pub client_stats: SideStats,
    pub server_stats: SideStats,
}

async fn stop_after(d: Option<Duration>) {
    match d {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending().await,
    }
}

/// Runs the first flow of `s` with both sides and the given openers.
pub async fn run_with<C: Opener, S: Opener>(
    s: &Scenario,
    client_opener: &mut C,
    server_opener: &mut S,
    mut opts: Opts,
) -> Pair {
    let filler = Filler::new(opts.seed);
    let flow = &s.flows[0];
    let mut server_functions = NoFunctions;
    let mut client = Session::new(SessionCtx {
        vars: &s.vars,
        filler: &filler,
        flow: &flow.id,
        side: Side::Client,
        session_id: opts.session_id,
        rng: session_rng(opts.seed, opts.session_id, Side::Client),
        functions: opts.client_functions.as_mut(),
        limits: opts.client_limits,
    });
    let mut server = Session::new(SessionCtx {
        vars: &s.vars,
        filler: &filler,
        flow: &flow.id,
        side: Side::Server,
        session_id: opts.session_id,
        rng: session_rng(opts.seed, opts.session_id, Side::Server),
        functions: &mut server_functions,
        limits: opts.server_limits,
    });
    if let Some(clock) = opts.clock.as_deref() {
        client.set_clock(clock);
        server.set_clock(clock);
    }
    let server_program = flow.server.as_ref().unwrap();
    let (c, sv) = tokio::join!(
        client.run_program(client_opener, &flow.client, stop_after(opts.client_stop)),
        server.run_program(server_opener, server_program, std::future::pending()),
    );
    Pair {
        client: c,
        server: sv,
        client_stats: client.stats(),
        server_stats: server.stats(),
    }
}

/// Runs the first flow of the scenario text with both sides over one duplex.
pub async fn run_pair(text: &str, opts: Opts) -> Pair {
    let s = compile(text);
    let (a, b) = duplex(opts.capacity);
    run_with(&s, &mut OneShot::new(a), &mut OneShot::new(b), opts).await
}

/// Like [`run_pair`], and also returns the bytes that each side wrote.
pub async fn run_recorded(text: &str, opts: Opts) -> (Pair, Vec<u8>, Vec<u8>) {
    let s = compile(text);
    let (a, b) = duplex(opts.capacity);
    let (a, c2s) = Tap::new(a);
    let (b, s2c) = Tap::new(b);
    let pair = run_with(&s, &mut OneShot::new(a), &mut OneShot::new(b), opts).await;
    let c2s = c2s.lock().unwrap().clone();
    let s2c = s2c.lock().unwrap().clone();
    (pair, c2s, s2c)
}
