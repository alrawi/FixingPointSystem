// SPDX-License-Identifier: GPL-3.0-only
//! Shared helpers that load the ssh scenarios and run both sides of a
//! scenario against each other over an in memory network.
//!
//! The network connects a `connect` to the `accept` of the same port and
//! records the bytes that each connection carried in each direction.
#![allow(dead_code)] // Each test file uses a different subset of the helpers.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use fps_flow::program::Flow;
use fps_flow::template::Filler;
use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use fps_flow::{Manifest, ModuleSet, Scenario, Value};
use fps_interp::{
    DatagramOpener, FlowEnv, FlowState, InstanceConfig, InstanceError, Limits, OpenKind,
    OpenRequest, Opener, ScenarioInstance, Side, SideRecord, Sides,
};
use fps_io::memory::{duplex, MemoryDatagram, MemoryIo};
use fps_io::SessionIo;
use fps_module_ssh::Ssh;
use rand::RngCore;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::oneshot;

/// Routes the calls of the module that the scenarios use, as the agent does
/// with the native functions of every installed module. One value serves one
/// session, because the SSH functions hold the keys of the session.
#[derive(Default)]
pub struct Dispatch {
    ssh: Ssh,
}

impl TemplateFn for Dispatch {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        match call.module {
            "ssh" => self.ssh.render(call, rng, out),
            other => Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("no module provides '{other}.{}'", call.function),
            )),
        }
    }
}

/// The run seed of every transcript.
pub const SEED: u64 = 42;

/// The session id of the first flow session of every transcript.
pub const SESSION_ID: u64 = 1;

/// The module directory, `modules/ssh`.
pub fn module_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Reads the module manifest.
pub fn manifest() -> Manifest {
    Manifest::load(&module_dir().join("manifest.yaml")).unwrap()
}

/// Loads every scenario that the manifest lists for a scenario instance,
/// checking module calls against the manifest catalog, and folds constants as
/// the engine does once per run. Each entry carries the scenario file stem.
pub fn scenarios() -> Vec<(String, Scenario)> {
    scenarios_with(&[])
}

/// Like [`scenarios`], and first replaces each listed variable in every
/// scenario that declares it.
pub fn scenarios_with(overrides: &[(&str, Value)]) -> Vec<(String, Scenario)> {
    let m = manifest();
    let mut set = ModuleSet::default();
    set.insert(m.clone());
    let ctx = set.context("ssh").unwrap();
    m.scenarios
        .iter()
        .map(|rel| {
            let path = module_dir().join(rel);
            let mut s = Scenario::load_instance(&path, Some(&ctx))
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
    /// Offset of a written byte whose lowest bit this tap flips, if any.
    flip_at: Option<usize>,
    sent: usize,
}

impl<S> Tap<S> {
    pub fn new(inner: S) -> (Self, Arc<Mutex<Vec<u8>>>) {
        let written = Arc::new(Mutex::new(Vec::new()));
        (
            Tap {
                inner,
                written: Arc::clone(&written),
                flip_at: None,
                sent: 0,
            },
            written,
        )
    }

    /// Flips the lowest bit of the byte at `offset` of what this tap writes.
    pub fn flipping(mut self, offset: Option<usize>) -> Self {
        self.flip_at = offset;
        self
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
        let mut copy;
        let mut data = data;
        if let Some(at) = this.flip_at {
            if (this.sent..this.sent + data.len()).contains(&at) {
                copy = data.to_vec();
                copy[at - this.sent] ^= 1;
                data = &copy;
            }
        }
        let r = Pin::new(&mut this.inner).poll_write(cx, data);
        if let Poll::Ready(Ok(n)) = r {
            this.written.lock().unwrap().extend_from_slice(&data[..n]);
            this.sent += n;
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

/// One TCP connection of the network, in the order the clients opened them.
#[derive(Clone)]
pub struct Conn {
    /// Id of the flow that opened it.
    pub flow: String,
    /// The port that the client connected to.
    pub port: u16,
    c2s: Arc<Mutex<Vec<u8>>>,
    s2c: Arc<Mutex<Vec<u8>>>,
}

impl Conn {
    /// The bytes that the client wrote.
    pub fn c2s(&self) -> Vec<u8> {
        self.c2s.lock().unwrap().clone()
    }

    /// The bytes that the server wrote.
    pub fn s2c(&self) -> Vec<u8> {
        self.s2c.lock().unwrap().clone()
    }
}

type Server = Tap<MemoryIo>;

struct Rendezvous {
    backlog: HashMap<u16, VecDeque<Server>>,
    waiting: HashMap<u16, VecDeque<oneshot::Sender<Server>>>,
}

/// An in memory network on which `connect` reaches the `accept` of the same
/// port, and which keeps every connection for the tests.
#[derive(Clone)]
pub struct MemNet {
    table: Arc<Mutex<Rendezvous>>,
    conns: Arc<Mutex<Vec<Conn>>>,
}

impl Default for MemNet {
    fn default() -> Self {
        MemNet {
            table: Arc::new(Mutex::new(Rendezvous {
                backlog: HashMap::new(),
                waiting: HashMap::new(),
            })),
            conns: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl MemNet {
    /// The connections opened so far, in the order of their `connect`.
    pub fn conns(&self) -> Vec<Conn> {
        self.conns.lock().unwrap().clone()
    }

    fn deliver(&self, port: u16, end: Server) {
        let mut end = end;
        let mut table = self.table.lock().unwrap();
        while let Some(tx) = table.waiting.get_mut(&port).and_then(VecDeque::pop_front) {
            match tx.send(end) {
                Ok(()) => return,
                Err(back) => end = back,
            }
        }
        table.backlog.entry(port).or_default().push_back(end);
    }

    fn take(&self, port: u16) -> Result<Server, oneshot::Receiver<Server>> {
        let mut table = self.table.lock().unwrap();
        if let Some(end) = table.backlog.get_mut(&port).and_then(VecDeque::pop_front) {
            return Ok(end);
        }
        let (tx, rx) = oneshot::channel();
        table.waiting.entry(port).or_default().push_back(tx);
        Err(rx)
    }
}

/// Opens the transports of one flow side.
pub struct StreamEnd {
    net: MemNet,
    flow: String,
    flip_client_at: Option<usize>,
}

impl Opener for StreamEnd {
    type Io = Tap<MemoryIo>;

    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<Self::Io>> {
        let net = self.net.clone();
        let flow = self.flow.clone();
        let flip = self.flip_client_at;
        let (kind, port) = (request.kind, request.port);
        async move {
            match kind {
                OpenKind::Connect => {
                    let (client, server) = duplex(64 << 10);
                    let (client, c2s) = Tap::new(client);
                    let client = client.flipping(flip);
                    let (server, s2c) = Tap::new(server);
                    net.conns.lock().unwrap().push(Conn {
                        flow,
                        port,
                        c2s,
                        s2c,
                    });
                    net.deliver(port, server);
                    Ok(client)
                }
                OpenKind::Accept => {
                    let taken = net.take(port);
                    match taken {
                        Ok(end) => Ok(end),
                        Err(rx) => rx.await.map_err(|_| {
                            io::Error::new(io::ErrorKind::NotConnected, "network dropped")
                        }),
                    }
                }
            }
        }
    }
}

/// The scenarios have no UDP flow, so this opener refuses every open.
pub struct NoDatagrams;

impl DatagramOpener for NoDatagrams {
    type Io = MemoryDatagram;

    async fn open(&mut self, _request: OpenRequest<'_>) -> io::Result<MemoryDatagram> {
        Err(io::Error::other("the ssh module has no UDP flow"))
    }
}

struct Env {
    net: MemNet,
    flip_client_at: Option<usize>,
}

impl FlowEnv for Env {
    type Stream = StreamEnd;
    type Datagram = NoDatagrams;

    fn stream(&mut self, flow: &Flow, _side: Side) -> StreamEnd {
        StreamEnd {
            net: self.net.clone(),
            flow: flow.id.clone(),
            flip_client_at: self.flip_client_at,
        }
    }

    fn datagram(&mut self, _flow: &Flow, _side: Side) -> NoDatagrams {
        NoDatagrams
    }

    fn functions(&mut self, _flow: &Flow, _side: Side) -> Box<dyn TemplateFn> {
        Box::new(Dispatch::default())
    }
}

/// The outcome of one scenario instance with both sides of every flow.
pub struct Run {
    pub result: Result<(), InstanceError>,
    pub records: Vec<SideRecord>,
    pub states: Vec<(String, Option<FlowState>)>,
    pub net: MemNet,
}

impl Run {
    /// The connections of the flow `id`, in the order of their `connect`.
    pub fn conns_of(&self, id: &str) -> Vec<Conn> {
        self.net
            .conns()
            .into_iter()
            .filter(|c| c.flow == id)
            .collect()
    }
}

/// Runs every flow of `s`, both sides, with the first flow session id
/// `first_session_id`.
pub async fn run(s: &Scenario, seed: u64, first_session_id: u64) -> Run {
    run_flipping(s, seed, first_session_id, None).await
}

/// Like [`run`], and flips the lowest bit of the byte at `flip_client_at` in
/// what the client writes, which models a fault on the wire.
pub async fn run_flipping(
    s: &Scenario,
    seed: u64,
    first_session_id: u64,
    flip_client_at: Option<usize>,
) -> Run {
    let filler = Filler::new(seed);
    let net = MemNet::default();
    let mut env = Env {
        net: net.clone(),
        flip_client_at,
    };
    let mut instance = ScenarioInstance::new(
        s,
        InstanceConfig {
            sides: Sides::Both,
            run_seed: seed,
            first_session_id,
            limits: Limits::default(),
        },
    );
    let result = instance
        .run(&mut env, &filler, std::future::pending())
        .await;
    let states = s
        .flows
        .iter()
        .map(|f| (f.id.clone(), instance.state(&f.id)))
        .collect();
    Run {
        result,
        records: instance.records().to_vec(),
        states,
        net,
    }
}
