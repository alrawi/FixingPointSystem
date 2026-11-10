// SPDX-License-Identifier: GPL-3.0-only
//! Shared helpers that load the smtp scenarios and run both sides of a flow
//! against each other over the in memory duplex of `fps-io`.
//!
//! A flow with `tls_upgrade` (the STARTTLS scenarios of MOD-20) runs with the
//! TLS profiles of [`tls`] and a fresh authority. [`run`] then returns the
//! logical SMTP dialogue in `c2s` and `s2c`, which is what each side wrote
//! before the upgrade followed by what it wrote above the TLS layer, and
//! [`TlsRun`] returns the TLS records that crossed the transport.
#![allow(dead_code)] // Each test file uses a different subset of the helpers.

pub mod tls;

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use fps_flow::program::{Op, TlsRole};
use fps_flow::template::Filler;
use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use fps_flow::{Manifest, ModuleSet, Scenario, Value};
use fps_interp::{
    session_rng, BoxedIo, Failure, Limits, OpenRequest, Opener, Session, SessionCtx, Side,
    SideStats, TlsUpgrader, UpgradeFuture, UpgradeRequest,
};
use fps_io::memory::duplex;
use fps_io::SessionIo;
use fps_module_content::Content;
use fps_module_smtp::Smtp;
use fps_module_tls::{HandshakeReport, NssKeyLog};
use rand::RngCore;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Routes the calls of the two modules that the scenarios use, as the agent
/// does with the native functions of every installed module.
#[derive(Default)]
pub struct Dispatch {
    smtp: Smtp,
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
            "smtp" => self.smtp.render(call, rng, out),
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

/// The module directory, `modules/smtp`.
pub fn module_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Reads the module manifest.
pub fn manifest() -> Manifest {
    Manifest::load(&module_dir().join("manifest.yaml")).unwrap()
}

/// True when a side of a flow of `s` upgrades its transport to TLS.
pub fn uses_tls(s: &Scenario) -> bool {
    let upgrades = |ops: &[Op]| ops.iter().any(|op| matches!(op, Op::UpgradeTls(_)));
    s.flows
        .iter()
        .any(|f| upgrades(&f.client.ops) || f.server.as_ref().is_some_and(|p| upgrades(&p.ops)))
}

/// Loads every scenario of the manifest that stays in plain text, checking
/// module calls against the manifest catalog, and folds constants as the
/// engine does once per run. Each entry carries the scenario file stem. The
/// STARTTLS scenarios are left out, because the tests of `scenarios.rs` parse
/// one plain dialogue. [`all_scenarios_with`] has them.
pub fn scenarios() -> Vec<(String, Scenario)> {
    scenarios_with(&[])
}

/// Like [`scenarios`], and first replaces each listed variable in every
/// scenario that declares it.
pub fn scenarios_with(overrides: &[(&str, Value)]) -> Vec<(String, Scenario)> {
    all_scenarios_with(overrides)
        .into_iter()
        .filter(|(_, s)| !uses_tls(s))
        .collect()
}

/// Like [`scenarios_with`], and keeps the scenarios that upgrade to TLS.
pub fn all_scenarios_with(overrides: &[(&str, Value)]) -> Vec<(String, Scenario)> {
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
    let ctx = set.context("smtp").unwrap();
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

/// What a session over TLS adds to its [`Run`].
#[derive(Debug)]
pub struct TlsRun {
    /// What the client negotiated.
    pub client: HandshakeReport,
    /// What the server negotiated.
    pub server: HandshakeReport,
    /// The bytes the client wrote on the transport after the upgrade, which
    /// are TLS records. The plaintext before the upgrade is not in them.
    pub wire_c2s: Vec<u8>,
    /// The bytes the server wrote on the transport after the upgrade.
    pub wire_s2c: Vec<u8>,
    /// How many bytes of `Run::c2s` the client wrote before the upgrade.
    pub plain_before_c2s: usize,
    /// How many bytes of `Run::s2c` the server wrote before the upgrade.
    pub plain_before_s2c: usize,
    /// The NSS key log of the client, which holds the secrets of the session.
    pub key_log: String,
}

/// The outcome of both sides of one session and the bytes each side wrote.
///
/// `c2s` and `s2c` hold the dialogue as the application wrote it. For a
/// session that upgrades to TLS that is the plaintext before the upgrade
/// followed by the plaintext above the TLS layer, so the STARTTLS command and
/// its 220 reply are in it, and `tls` holds the records on the wire.
#[derive(Debug)]
pub struct Run {
    pub client: Result<(), Failure>,
    pub server: Result<(), Failure>,
    pub client_stats: SideStats,
    pub server_stats: SideStats,
    pub c2s: Vec<u8>,
    pub s2c: Vec<u8>,
    /// Stream offsets after each write of the client, in the offsets of `c2s`.
    pub c2s_ends: Vec<usize>,
    /// Stream offsets after each write of the server, in the offsets of `s2c`.
    pub s2c_ends: Vec<usize>,
    pub tls: Option<TlsRun>,
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

/// What one side wrote above its TLS layer.
struct Above {
    role: TlsRole,
    /// Bytes the side had written on the transport when it started the upgrade.
    before: usize,
    written: Arc<Mutex<Vec<u8>>>,
    ends: Arc<Mutex<Vec<usize>>>,
}

/// Serves the `tls_upgrade` actions with the profiles of [`tls`], notes how
/// much each side had written when it upgraded, and taps the plaintext above
/// the TLS layer of each side.
struct Upgrades<'a> {
    inner: &'a tls::Profiles,
    client_wire: Arc<Mutex<Vec<u8>>>,
    server_wire: Arc<Mutex<Vec<u8>>>,
    above: Arc<Mutex<Vec<Above>>>,
}

impl TlsUpgrader for Upgrades<'_> {
    fn upgrade<'io>(&self, io: BoxedIo<'io>, request: UpgradeRequest<'_>) -> UpgradeFuture<'io> {
        let role = request.role;
        let wire = match role {
            TlsRole::Client => &self.client_wire,
            TlsRole::Server => &self.server_wire,
        };
        let before = wire.lock().unwrap().len();
        let secured = self.inner.upgrade(io, request);
        let above = Arc::clone(&self.above);
        Box::pin(async move {
            let session = secured.await?;
            let (tapped, written) = Tap::new(session);
            above.lock().unwrap().push(Above {
                role,
                before,
                written,
                ends: tapped.ends(),
            });
            Ok(Box::new(tapped) as BoxedIo<'io>)
        })
    }
}

/// What one side wrote, split at its upgrade.
struct Joined {
    /// The plaintext before the upgrade followed by the plaintext above TLS.
    stream: Vec<u8>,
    /// Offsets after each write, in the offsets of `stream`.
    ends: Vec<usize>,
    /// The records on the transport after the upgrade.
    records: Vec<u8>,
    /// Bytes of `stream` that came before the upgrade.
    before: usize,
}

/// Joins what a side wrote before its upgrade, the first `before` bytes of the
/// transport, with what it wrote above the TLS layer.
fn join(wire: &[u8], wire_ends: &[usize], above: Option<&Above>) -> Joined {
    let Some(a) = above else {
        return Joined {
            stream: wire.to_vec(),
            ends: wire_ends.to_vec(),
            records: Vec::new(),
            before: wire.len(),
        };
    };
    let mut stream = wire[..a.before].to_vec();
    let mut ends: Vec<usize> = wire_ends
        .iter()
        .copied()
        .filter(|&e| e <= a.before)
        .collect();
    stream.extend_from_slice(&a.written.lock().unwrap());
    ends.extend(a.ends.lock().unwrap().iter().map(|e| e + a.before));
    Joined {
        stream,
        ends,
        records: wire[a.before..].to_vec(),
        before: a.before,
    }
}

/// Runs the first flow of `s` with both sides over one duplex, with the smtp
/// functions on both sides. A flow with `tls_upgrade` runs with the TLS
/// profiles of [`tls`] and a fresh authority.
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
    let key_log = Arc::new(NssKeyLog::in_memory());
    let profiles = uses_tls(s).then(|| {
        let authority = tls::authority().unwrap();
        let identity = tls::identity(&authority).unwrap();
        let roots = authority.root_store().unwrap();
        tls::Profiles::default()
            .with_clients(&roots, &key_log)
            .unwrap()
            .with_servers(&identity)
            .unwrap()
    });
    let upgrades = profiles.as_ref().map(|p| Upgrades {
        inner: p,
        client_wire: Arc::clone(&c2s),
        server_wire: Arc::clone(&s2c),
        above: Arc::default(),
    });
    if let Some(u) = &upgrades {
        client.set_tls_upgrader(u);
        server.set_tls_upgrader(u);
    }
    let server_program = flow.server.as_ref().unwrap();
    let (mut client_open, mut server_open) = (OneShot(Some(a)), OneShot(Some(b)));
    let (c, sv) = tokio::join!(
        client.run_program(&mut client_open, &flow.client, std::future::pending()),
        server.run_program(&mut server_open, server_program, std::future::pending()),
    );
    let wire_c2s = std::mem::take(&mut *c2s.lock().unwrap());
    let wire_s2c = std::mem::take(&mut *s2c.lock().unwrap());
    let wire_c2s_ends = std::mem::take(&mut *c2s_ends.lock().unwrap());
    let wire_s2c_ends = std::mem::take(&mut *s2c_ends.lock().unwrap());
    let (client_stats, server_stats) = (client.stats(), server.stats());
    let (mut reports, mut above_c, mut above_s) = (None, None, None);
    if let (Some(u), Some(p)) = (&upgrades, &profiles) {
        let mut above = u.above.lock().unwrap();
        above_c = above
            .iter()
            .position(|a| a.role == TlsRole::Client)
            .map(|i| above.swap_remove(i));
        above_s = above
            .iter()
            .position(|a| a.role == TlsRole::Server)
            .map(|i| above.swap_remove(i));
        let all = p.reports();
        let by_side = |side| all.iter().find(|r| r.side == side).cloned();
        if c.is_ok() {
            reports =
                by_side(fps_module_tls::Side::Client).zip(by_side(fps_module_tls::Side::Server));
        }
    }
    let client_side = join(&wire_c2s, &wire_c2s_ends, above_c.as_ref());
    let server_side = join(&wire_s2c, &wire_s2c_ends, above_s.as_ref());
    let tls = reports.map(|(client, server)| TlsRun {
        client,
        server,
        wire_c2s: client_side.records.clone(),
        wire_s2c: server_side.records.clone(),
        plain_before_c2s: client_side.before,
        plain_before_s2c: server_side.before,
        key_log: key_log.contents(),
    });
    Run {
        client: c,
        server: sv,
        client_stats,
        server_stats,
        c2s: client_side.stream,
        s2c: server_side.stream,
        c2s_ends: client_side.ends,
        s2c_ends: server_side.ends,
        tls,
    }
}
