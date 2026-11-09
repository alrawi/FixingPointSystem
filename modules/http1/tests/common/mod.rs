// SPDX-License-Identifier: GPL-3.0-only
//! Shared helpers that load the http1 scenarios and run both sides of a flow
//! against each other over the in memory duplex of `fps-io`.
#![allow(dead_code)] // Each test file uses a different subset of the helpers.

pub mod http;
pub mod tls;

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use fps_flow::program::{Op, TlsRole};
use fps_flow::template::Filler;
use fps_flow::{Manifest, ModuleSet, Scenario, Value};
use fps_interp::{
    session_rng, BoxedIo, Failure, Limits, OpenRequest, Opener, Session, SessionCtx, Side,
    SideStats, TlsUpgrader, UpgradeFuture, UpgradeRequest,
};
use fps_io::memory::duplex;
use fps_io::SessionIo;
use fps_module_http1::Http1;
use fps_module_tls::{HandshakeReport, NssKeyLog};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// The run seed of every transcript.
pub const SEED: u64 = 42;

/// The session id of every transcript.
pub const SESSION_ID: u64 = 1;

/// The module directory, `modules/http1`.
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
    let ctx = set.context("http1").unwrap();
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
}

impl<S> Tap<S> {
    pub fn new(inner: S) -> (Self, Arc<Mutex<Vec<u8>>>) {
        let written = Arc::new(Mutex::new(Vec::new()));
        (
            Tap {
                inner,
                written: Arc::clone(&written),
            },
            written,
        )
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

/// Hands out one prepared transport.
pub struct OneShot<S>(Option<S>);

impl<S: SessionIo> Opener for OneShot<S> {
    type Io = S;

    fn open(&mut self, _request: OpenRequest<'_>) -> impl Future<Output = io::Result<S>> {
        let io = self.0.take();
        async move { io.ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "used")) }
    }
}

/// What a session over TLS adds to its [`Run`].
#[derive(Debug)]
pub struct TlsRun {
    /// What the client negotiated.
    pub client: HandshakeReport,
    /// What the server negotiated.
    pub server: HandshakeReport,
    /// The bytes the client wrote on the transport, which are TLS records.
    pub wire_c2s: Vec<u8>,
    /// The bytes the server wrote on the transport, which are TLS records.
    pub wire_s2c: Vec<u8>,
    /// The NSS key log of the client, which holds the secrets of the session.
    pub key_log: String,
}

/// The outcome of both sides of one session and the bytes each side wrote.
///
/// `c2s` and `s2c` hold what the application wrote, so the plaintext of a
/// session over TLS, and `tls` holds the records on the wire.
#[derive(Debug)]
pub struct Run {
    pub client: Result<(), Failure>,
    pub server: Result<(), Failure>,
    pub client_stats: SideStats,
    pub server_stats: SideStats,
    pub c2s: Vec<u8>,
    pub s2c: Vec<u8>,
    pub tls: Option<TlsRun>,
}

/// True when a side of the first flow upgrades its transport to TLS.
pub fn uses_tls(s: &Scenario) -> bool {
    let upgrades = |ops: &[Op]| ops.iter().any(|op| matches!(op, Op::UpgradeTls(_)));
    s.flows
        .iter()
        .any(|f| upgrades(&f.client.ops) || f.server.as_ref().is_some_and(|p| upgrades(&p.ops)))
}

type Taps = Arc<Mutex<Vec<(TlsRole, Arc<Mutex<Vec<u8>>>)>>>;

/// Serves the `tls_upgrade` actions with the profiles of [`tls`] and taps the
/// plaintext above the TLS layer of each side.
struct PlaintextTap<'a> {
    inner: &'a tls::Profiles,
    taps: Taps,
}

impl PlaintextTap<'_> {
    fn plaintext(&self, role: TlsRole) -> Vec<u8> {
        let taps = self.taps.lock().unwrap();
        taps.iter()
            .find(|(r, _)| *r == role)
            .map(|(_, written)| std::mem::take(&mut *written.lock().unwrap()))
            .unwrap_or_default()
    }
}

impl TlsUpgrader for PlaintextTap<'_> {
    fn upgrade<'io>(&self, io: BoxedIo<'io>, request: UpgradeRequest<'_>) -> UpgradeFuture<'io> {
        let secured = self.inner.upgrade(io, request);
        let role = request.role;
        let taps = Arc::clone(&self.taps);
        Box::pin(async move {
            let session = secured.await?;
            let (tapped, written) = Tap::new(session);
            taps.lock().unwrap().push((role, written));
            Ok(Box::new(tapped) as BoxedIo<'io>)
        })
    }
}

/// Runs the first flow of `s` with both sides over one duplex, with the http1
/// functions on both sides. A flow with `tls_upgrade` runs with the TLS
/// profiles of [`tls`] and a fresh authority.
pub async fn run(s: &Scenario, seed: u64, session_id: u64) -> Run {
    let filler = Filler::new(seed);
    let flow = &s.flows[0];
    let (a, b) = duplex(64 * 1024);
    let (a, c2s) = Tap::new(a);
    let (b, s2c) = Tap::new(b);
    let mut client_fns = Http1;
    let mut server_fns = Http1;
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
    let key_log = Arc::new(NssKeyLog::in_memory());
    let profiles = if uses_tls(s) {
        let authority = tls::authority().unwrap();
        let identity = tls::identity(&authority).unwrap();
        let roots = authority.root_store().unwrap();
        Some(
            tls::Profiles::default()
                .with_clients(&roots, &key_log)
                .unwrap()
                .with_servers(&identity)
                .unwrap(),
        )
    } else {
        None
    };
    let tap = profiles.as_ref().map(|p| PlaintextTap {
        inner: p,
        taps: Taps::default(),
    });
    if let Some(tap) = &tap {
        client.set_tls_upgrader(tap);
        server.set_tls_upgrader(tap);
    }
    let server_program = flow.server.as_ref().unwrap();
    let (mut client_open, mut server_open) = (OneShot(Some(a)), OneShot(Some(b)));
    let (c, sv) = tokio::join!(
        client.run_program(&mut client_open, &flow.client, std::future::pending()),
        server.run_program(&mut server_open, server_program, std::future::pending()),
    );
    let wire_c2s = std::mem::take(&mut *c2s.lock().unwrap());
    let wire_s2c = std::mem::take(&mut *s2c.lock().unwrap());
    let (client_stats, server_stats) = (client.stats(), server.stats());
    let (c2s, s2c, tls_run) = match (&tap, &profiles) {
        (Some(tap), Some(profiles)) => {
            let reports = profiles.reports();
            let by_side = |side| {
                reports
                    .iter()
                    .find(|r| r.side == side)
                    .cloned()
                    .unwrap_or_else(|| panic!("no {side:?} handshake report"))
            };
            let tls_run = c.is_ok().then(|| TlsRun {
                client: by_side(fps_module_tls::Side::Client),
                server: by_side(fps_module_tls::Side::Server),
                wire_c2s: wire_c2s.clone(),
                wire_s2c: wire_s2c.clone(),
                key_log: key_log.contents(),
            });
            (
                tap.plaintext(TlsRole::Client),
                tap.plaintext(TlsRole::Server),
                tls_run,
            )
        }
        _ => (wire_c2s, wire_s2c, None),
    };
    Run {
        client: c,
        server: sv,
        client_stats,
        server_stats,
        c2s,
        s2c,
        tls: tls_run,
    }
}
