// SPDX-License-Identifier: GPL-3.0-only
//! Shared helpers that load the ntp scenarios and run both sides of every
//! flow against each other over the in memory datagram pair of `fps-io`, with
//! the clock of each side pinned, because the packets carry timestamps.
#![allow(dead_code)] // Each test file uses a different subset of the helpers.

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use fps_flow::template::Filler;
use fps_flow::{Manifest, ModuleSet, PinnedClock, Scenario, Value, Vars};
use fps_interp::{
    session_rng, DatagramOpener, Failure, Limits, OpenKind, OpenRequest, Session, SessionCtx, Side,
    SideStats,
};
use fps_io::memory::{datagram_pair, MemoryDatagram};
use fps_io::DatagramIo;
use fps_module_ntp::Ntp;

/// The run seed of every transcript.
pub const SEED: u64 = 42;

/// The session id of the first flow of every transcript.
pub const SESSION_ID: u64 = 1;

/// The instant of the first read of the client clock, 2026-11-10T12:00:00Z in
/// milliseconds since the Unix epoch.
pub const CLIENT_START_MS: i64 = 1_794_312_000_000;

/// The time that the pinned clock of each side advances by with every read.
pub const STEP_MS: i64 = 1_000;

/// How far the server clock runs ahead of the client clock, which stands for
/// the delay of the path in this test.
pub const SERVER_AHEAD_MS: i64 = 23;

/// The module directory, `modules/ntp`.
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
    let ctx = set.context("ntp").unwrap();
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

/// The outcome of both sides of one flow and what each side wrote, one
/// entry per datagram.
#[derive(Debug)]
pub struct FlowRun {
    pub flow: String,
    pub client: Result<(), Failure>,
    pub server: Result<(), Failure>,
    pub client_stats: SideStats,
    pub server_stats: SideStats,
    pub client_vars: Vars,
    pub c2s: Vec<Vec<u8>>,
    pub s2c: Vec<Vec<u8>>,
}

impl FlowRun {
    /// Returns the transcript of one direction: each datagram with its length
    /// as two big endian bytes in front, so the file keeps the boundaries.
    pub fn transcript(datagrams: &[Vec<u8>]) -> Vec<u8> {
        datagrams
            .iter()
            .flat_map(|g| (g.len() as u16).to_be_bytes().into_iter().chain(g.clone()))
            .collect()
    }
}

fn ctx<'a>(
    s: &'a Scenario,
    filler: &'a Filler,
    flow: &'a str,
    side: Side,
    seed: u64,
    session_id: u64,
    functions: &'a mut Ntp,
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
/// `first_id + n`, and the clocks of both sides pinned at the instants above.
pub async fn run(s: &Scenario, seed: u64, first_id: u64) -> Vec<FlowRun> {
    run_at(s, seed, first_id, CLIENT_START_MS).await
}

/// Like [`run`], with the client clock pinned at `start_ms`. The server clock
/// starts [`SERVER_AHEAD_MS`] later.
pub async fn run_at(s: &Scenario, seed: u64, first_id: u64, start_ms: i64) -> Vec<FlowRun> {
    let filler = Filler::new(seed);
    let mut out = Vec::new();
    for (n, flow) in s.flows.iter().enumerate() {
        let id = first_id + n as u64;
        let client_clock = PinnedClock::new(start_ms).with_step_ms(STEP_MS);
        let server_clock = PinnedClock::new(start_ms + SERVER_AHEAD_MS).with_step_ms(STEP_MS);
        let (mut client_fns, mut server_fns) = (Ntp, Ntp);
        let mut client = Session::new(ctx(
            s,
            &filler,
            &flow.id,
            Side::Client,
            seed,
            id,
            &mut client_fns,
        ));
        client.set_clock(&client_clock);
        let mut server = Session::new(ctx(
            s,
            &filler,
            &flow.id,
            Side::Server,
            seed,
            id,
            &mut server_fns,
        ));
        server.set_clock(&server_clock);
        let server_program = flow.server.as_ref().unwrap();
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
        let c2s = std::mem::take(&mut *ca.lock().unwrap());
        let s2c = std::mem::take(&mut *cb.lock().unwrap());
        out.push(FlowRun {
            flow: flow.id.clone(),
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

/// Returns the integer that a `set` action bound.
pub fn int(vars: &Vars, name: &str) -> Option<i64> {
    vars.get(name).and_then(|v| match v {
        Value::Int(i) => Some(*i),
        _ => None,
    })
}
