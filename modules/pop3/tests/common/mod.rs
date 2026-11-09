// SPDX-License-Identifier: GPL-3.0-only
//! Shared helpers that load the pop3 scenarios and run both sides of a flow
//! against each other over the in memory duplex of `fps-io`.
#![allow(dead_code)] // Each test file uses a different subset of the helpers.

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use fps_flow::template::Filler;
use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use fps_flow::{Manifest, ModuleSet, Scenario, Value};
use fps_interp::{
    session_rng, Failure, Limits, OpenRequest, Opener, Session, SessionCtx, Side, SideStats,
};
use fps_io::memory::duplex;
use fps_io::SessionIo;
use fps_module_content::Content;
use fps_module_pop3::Pop3;
use rand::RngCore;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Routes the calls of the two modules that the scenarios use, as the agent
/// does with the native functions of every installed module.
#[derive(Default)]
pub struct Dispatch {
    pop3: Pop3,
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
            "pop3" => self.pop3.render(call, rng, out),
            "content" => self.content.render(call, rng, out),
            other => Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("no module provides '{other}.{}'", call.function),
            )),
        }
    }
}

/// The run seed of every transcript.
pub const SEED: u64 = 42;

/// The session id of every transcript.
pub const SESSION_ID: u64 = 1;

/// The module directory, `modules/pop3`.
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
    scenarios_with(&[])
}

/// Like [`scenarios`], and first replaces each listed variable in every
/// scenario that declares it.
pub fn scenarios_with(overrides: &[(&str, Value)]) -> Vec<(String, Scenario)> {
    let m = manifest();
    let mut set = ModuleSet::default();
    set.insert(m.clone());
    // The scenarios call the content module, which the manifest requires.
    set.insert(
        Manifest::load(
            &module_dir()
                .join("..")
                .join("content")
                .join("manifest.yaml"),
        )
        .unwrap(),
    );
    let ctx = set.context("pop3").unwrap();
    m.scenarios
        .iter()
        .map(|rel| {
            let path = module_dir().join(rel);
            let mut s = Scenario::load_with(&path, Some(&ctx))
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let own: Vec<(String, Value)> = overrides
                .iter()
                .filter(|(k, _)| s.vars.contains_key(*k))
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect();
            s.override_vars(&own).unwrap();
            s.fold_constants().unwrap();
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            (stem, s)
        })
        .collect()
}

/// A transport wrapper that records the bytes written through it.
pub struct Tap<S> {
    inner: S,
    written: Arc<Mutex<Vec<u8>>>,
    /// The stream offset after each write, which shows how a side grouped
    /// its output into writes, so a test can tell pipelined commands from
    /// commands that wait for an answer.
    ends: Arc<Mutex<Vec<usize>>>,
}

impl<S> Tap<S> {
    pub fn new(inner: S) -> (Self, Arc<Mutex<Vec<u8>>>) {
        let written = Arc::new(Mutex::new(Vec::new()));
        (
            Tap {
                inner,
                written: Arc::clone(&written),
                ends: Arc::new(Mutex::new(Vec::new())),
            },
            written,
        )
    }

    pub fn ends(&self) -> Arc<Mutex<Vec<usize>>> {
        Arc::clone(&self.ends)
    }
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
            let mut w = this.written.lock().unwrap();
            w.extend_from_slice(&data[..n]);
            this.ends.lock().unwrap().push(w.len());
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

/// Hands out one prepared transport.
pub struct OneShot<S>(Option<S>);

impl<S: SessionIo> Opener for OneShot<S> {
    type Io = S;

    fn open(&mut self, _request: OpenRequest<'_>) -> impl Future<Output = io::Result<S>> {
        let io = self.0.take();
        async move { io.ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "used")) }
    }
}

/// The outcome of both sides of one session and the bytes each side wrote.
#[derive(Debug)]
pub struct Run {
    pub client: Result<(), Failure>,
    pub server: Result<(), Failure>,
    pub client_stats: SideStats,
    pub server_stats: SideStats,
    pub c2s: Vec<u8>,
    pub s2c: Vec<u8>,
    /// Stream offsets after each write of the client.
    pub c2s_ends: Vec<usize>,
    /// Stream offsets after each write of the server.
    pub s2c_ends: Vec<usize>,
}

/// Returns true when one write carried the bytes `from..to` of a stream whose
/// writes end at the offsets `ends`.
pub fn in_one_write(ends: &[usize], from: usize, to: usize) -> bool {
    let mut start = 0;
    for &end in ends {
        if start <= from && to <= end {
            return true;
        }
        start = end;
    }
    false
}

/// Runs the first flow of `s` with both sides over one duplex, with the pop3
/// functions on both sides.
pub async fn run(s: &Scenario, seed: u64, session_id: u64) -> Run {
    let filler = Filler::new(seed);
    let flow = &s.flows[0];
    let (a, b) = duplex(64 * 1024);
    let (a, c2s) = Tap::new(a);
    let (b, s2c) = Tap::new(b);
    let (c2s_ends, s2c_ends) = (a.ends(), b.ends());
    let mut client_fns = Dispatch::default();
    let mut server_fns = Dispatch::default();
    let mut client = Session::new(SessionCtx {
        vars: &s.vars,
        filler: &filler,
        flow: &flow.id,
        side: Side::Client,
        session_id,
        rng: session_rng(seed, session_id, Side::Client),
        functions: &mut client_fns,
        limits: Limits::default(),
    });
    let mut server = Session::new(SessionCtx {
        vars: &s.vars,
        filler: &filler,
        flow: &flow.id,
        side: Side::Server,
        session_id,
        rng: session_rng(seed, session_id, Side::Server),
        functions: &mut server_fns,
        limits: Limits::default(),
    });
    let server_program = flow.server.as_ref().unwrap();
    let (mut client_open, mut server_open) = (OneShot(Some(a)), OneShot(Some(b)));
    let (c, sv) = tokio::join!(
        client.run_program(&mut client_open, &flow.client, std::future::pending()),
        server.run_program(&mut server_open, server_program, std::future::pending()),
    );
    let c2s = std::mem::take(&mut *c2s.lock().unwrap());
    let s2c = std::mem::take(&mut *s2c.lock().unwrap());
    let c2s_ends_taken = std::mem::take(&mut *c2s_ends.lock().unwrap());
    let s2c_ends_taken = std::mem::take(&mut *s2c_ends.lock().unwrap());
    Run {
        client: c,
        server: sv,
        client_stats: client.stats(),
        server_stats: server.stats(),
        c2s,
        s2c,
        c2s_ends: c2s_ends_taken,
        s2c_ends: s2c_ends_taken,
    }
}
