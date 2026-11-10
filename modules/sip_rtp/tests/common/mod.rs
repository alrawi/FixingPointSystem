// SPDX-License-Identifier: GPL-3.0-only
//! Shared helpers that load the sip_rtp scenarios and run every flow of a
//! scenario instance, both sides against each other, over an in memory UDP
//! network.
//!
//! A scenario instance spawns the media flow, so these helpers use
//! `ScenarioInstance` of `fps-interp`. The network connects a `connect` to the
//! `accept` of the same port. It keeps every datagram that each side sent,
//! with the time of the send on the Tokio clock, so a test that runs on the
//! paused clock reads the exact pacing of a flow.
#![allow(dead_code)] // Each test file uses a different subset of the helpers.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use fps_flow::program::Flow;
use fps_flow::template::Filler;
use fps_flow::{Manifest, ModuleSet, Scenario, TemplateFn, Value};
use fps_interp::{
    DatagramOpener, FlowEnv, FlowState, InstanceConfig, InstanceError, Limits, OpenKind,
    OpenRequest, Opener, ScenarioInstance, Side, SideRecord, Sides,
};
use fps_io::memory::{datagram_pair, MemoryDatagram, MemoryIo};
use fps_io::DatagramIo;
use fps_module_sip_rtp::SipRtp;
use tokio::sync::oneshot;
use tokio::time::Instant;

/// The run seed of every transcript.
pub const SEED: u64 = 42;

/// The session id of the first flow session of every transcript.
pub const SESSION_ID: u64 = 1;

/// The module directory, `modules/sip_rtp`.
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
    let ctx = set.context("sip_rtp").unwrap();
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

/// One datagram that a side sent and the Tokio clock at that moment.
#[derive(Clone, Debug)]
pub struct Sent {
    pub at: Instant,
    pub bytes: Vec<u8>,
}

type Log = Arc<Mutex<Vec<Sent>>>;

/// One UDP session of the network, in the order the clients opened them.
#[derive(Clone)]
pub struct Conn {
    /// Id of the flow that opened it.
    pub flow: String,
    /// The port that the client connected to.
    pub port: u16,
    c2s: Log,
    s2c: Log,
}

impl Conn {
    /// The datagrams that the client sent.
    pub fn c2s(&self) -> Vec<Sent> {
        self.c2s.lock().unwrap().clone()
    }

    /// The datagrams that the server sent.
    pub fn s2c(&self) -> Vec<Sent> {
        self.s2c.lock().unwrap().clone()
    }

    /// The bytes of the client datagrams, without times.
    pub fn c2s_bytes(&self) -> Vec<Vec<u8>> {
        self.c2s().into_iter().map(|s| s.bytes).collect()
    }

    /// The bytes of the server datagrams, without times.
    pub fn s2c_bytes(&self) -> Vec<Vec<u8>> {
        self.s2c().into_iter().map(|s| s.bytes).collect()
    }
}

/// A datagram end that records what it sends, and that can lose one datagram
/// on the way, as a network does.
pub struct Tap {
    inner: MemoryDatagram,
    sent: Log,
    /// The 1 based number of the datagram that this end loses.
    lose: Option<usize>,
    count: usize,
}

impl Tap {
    fn new(inner: MemoryDatagram, sent: Log, lose: Option<usize>) -> Tap {
        Tap {
            inner,
            sent,
            lose,
            count: 0,
        }
    }
}

impl DatagramIo for Tap {
    fn poll_send(&mut self, cx: &mut Context<'_>, datagram: &[u8]) -> Poll<io::Result<()>> {
        self.count += 1;
        if self.lose == Some(self.count) {
            return Poll::Ready(Ok(()));
        }
        let r = self.inner.poll_send(cx, datagram);
        if let Poll::Ready(Ok(())) = r {
            self.sent.lock().unwrap().push(Sent {
                at: Instant::now(),
                bytes: datagram.to_vec(),
            });
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

struct Rendezvous {
    backlog: HashMap<u16, VecDeque<Tap>>,
    waiting: HashMap<u16, VecDeque<oneshot::Sender<Tap>>>,
}

/// An in memory network on which `connect` reaches the `accept` of the same
/// port, and which keeps every session for the tests.
#[derive(Clone)]
pub struct MemNet {
    table: Arc<Mutex<Rendezvous>>,
    conns: Arc<Mutex<Vec<Conn>>>,
    /// The number of the RTP packet that the client loses on its way to the server.
    lose_rtp: Option<usize>,
}

impl Default for MemNet {
    fn default() -> Self {
        MemNet {
            table: Arc::new(Mutex::new(Rendezvous {
                backlog: HashMap::new(),
                waiting: HashMap::new(),
            })),
            conns: Arc::new(Mutex::new(Vec::new())),
            lose_rtp: None,
        }
    }
}

impl MemNet {
    /// The sessions opened so far, in the order of their `connect`.
    pub fn conns(&self) -> Vec<Conn> {
        self.conns.lock().unwrap().clone()
    }

    fn deliver(&self, port: u16, end: Tap) {
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

    fn take(&self, port: u16) -> Result<Tap, oneshot::Receiver<Tap>> {
        let mut table = self.table.lock().unwrap();
        if let Some(end) = table.backlog.get_mut(&port).and_then(VecDeque::pop_front) {
            return Ok(end);
        }
        let (tx, rx) = oneshot::channel();
        table.waiting.entry(port).or_default().push_back(tx);
        Err(rx)
    }
}

/// Opens the sockets of one flow side.
pub struct DatagramEnd {
    net: MemNet,
    flow: String,
}

impl DatagramOpener for DatagramEnd {
    type Io = Tap;

    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<Tap>> {
        let net = self.net.clone();
        let flow = self.flow.clone();
        let (kind, port) = (request.kind, request.port);
        async move {
            match kind {
                OpenKind::Connect => {
                    let (client, server) = datagram_pair(64);
                    let (c2s, s2c): (Log, Log) = (Arc::default(), Arc::default());
                    let lose = if flow == "media" { net.lose_rtp } else { None };
                    net.conns.lock().unwrap().push(Conn {
                        flow,
                        port,
                        c2s: Arc::clone(&c2s),
                        s2c: Arc::clone(&s2c),
                    });
                    net.deliver(port, Tap::new(server, s2c, None));
                    Ok(Tap::new(client, c2s, lose))
                }
                OpenKind::Accept => {
                    let end = match net.take(port) {
                        Ok(end) => end,
                        Err(rx) => rx.await.map_err(|_| {
                            io::Error::new(io::ErrorKind::NotConnected, "network dropped")
                        })?,
                    };
                    // As the datagram rules require, `accept` waits for the
                    // first datagram and leaves it queued.
                    end.inner.readable().await?;
                    Ok(end)
                }
            }
        }
    }
}

/// The scenarios have no TCP flow, so this opener refuses every open.
pub struct NoStreams;

impl Opener for NoStreams {
    type Io = MemoryIo;

    async fn open(&mut self, _request: OpenRequest<'_>) -> io::Result<MemoryIo> {
        Err(io::Error::other("the sip_rtp module has no TCP flow"))
    }
}

struct Env {
    net: MemNet,
}

impl FlowEnv for Env {
    type Stream = NoStreams;
    type Datagram = DatagramEnd;

    fn stream(&mut self, _flow: &Flow, _side: Side) -> NoStreams {
        NoStreams
    }

    fn datagram(&mut self, flow: &Flow, _side: Side) -> DatagramEnd {
        DatagramEnd {
            net: self.net.clone(),
            flow: flow.id.clone(),
        }
    }

    /// Every flow side gets its own functions, because the stream of `rtp`
    /// keeps its sequence number in them.
    fn functions(&mut self, _flow: &Flow, _side: Side) -> Box<dyn TemplateFn> {
        Box::new(SipRtp::default())
    }
}

/// The outcome of one scenario instance with both sides of every flow.
pub struct Run {
    pub result: Result<(), InstanceError>,
    pub records: Vec<SideRecord>,
    pub states: Vec<(String, Option<FlowState>)>,
    pub net: MemNet,
    /// The Tokio clock when the instance started.
    pub started: Instant,
}

impl Run {
    /// The sessions of the flow `id`, in the order of their `connect`.
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
    run_on(s, seed, first_session_id, MemNet::default()).await
}

/// Like [`run`], and the client loses its `n`-th RTP packet (counted from 1).
pub async fn run_losing_rtp(s: &Scenario, seed: u64, first_session_id: u64, n: usize) -> Run {
    let net = MemNet {
        lose_rtp: Some(n),
        ..MemNet::default()
    };
    run_on(s, seed, first_session_id, net).await
}

async fn run_on(s: &Scenario, seed: u64, first_session_id: u64, net: MemNet) -> Run {
    let filler = Filler::new(seed);
    let mut env = Env { net: net.clone() };
    let mut instance = ScenarioInstance::new(
        s,
        InstanceConfig {
            sides: Sides::Both,
            run_seed: seed,
            first_session_id,
            limits: Limits::default(),
        },
    );
    let started = Instant::now();
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
        started,
    }
}

/// A SIP message split into its parts, for the tests that check the dialog.
#[derive(Debug)]
pub struct Sip {
    pub start: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Sip {
    /// Splits a datagram and checks its `Content-Length`.
    pub fn parse(bytes: &[u8]) -> Sip {
        let text = std::str::from_utf8(bytes).expect("SIP is text");
        let (head, body) = text.split_once("\r\n\r\n").expect("empty line");
        let mut lines = head.split("\r\n");
        let start = lines.next().unwrap().to_string();
        let headers: Vec<(String, String)> = lines
            .map(|l| {
                let (n, v) = l.split_once(": ").expect("header");
                (n.to_string(), v.to_string())
            })
            .collect();
        let sip = Sip {
            start,
            headers,
            body: body.to_string(),
        };
        assert_eq!(
            sip.header("Content-Length")
                .unwrap()
                .parse::<usize>()
                .unwrap(),
            sip.body.len(),
            "Content-Length of {}",
            sip.start
        );
        sip
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The branch parameter of the top `Via`.
    pub fn branch(&self) -> String {
        let via = self.header("Via").unwrap();
        via.split(';')
            .find_map(|p| p.strip_prefix("branch="))
            .unwrap()
            .to_string()
    }

    /// The `tag` parameter of a header.
    pub fn tag(&self, name: &str) -> Option<String> {
        self.header(name)?
            .split(';')
            .find_map(|p| p.strip_prefix("tag="))
            .map(str::to_string)
    }

    /// The first token after `SIP/2.0 ` of a response or the method of a
    /// request.
    pub fn code_or_method(&self) -> String {
        match self.start.strip_prefix("SIP/2.0 ") {
            Some(r) => r.split(' ').next().unwrap().to_string(),
            None => self.start.split(' ').next().unwrap().to_string(),
        }
    }

    /// The value of an SDP line that starts with `prefix`.
    pub fn sdp(&self, prefix: &str) -> Vec<&str> {
        self.body
            .split("\r\n")
            .filter_map(|l| l.strip_prefix(prefix))
            .collect()
    }
}

/// The fixed header of an RTP packet.
#[derive(Debug, PartialEq, Eq)]
pub struct Rtp {
    pub marker: bool,
    pub pt: u8,
    pub seq: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub payload: usize,
}

/// Parses an RTP packet of RFC 3550 with no CSRC list and no extension.
pub fn rtp(packet: &[u8]) -> Rtp {
    assert!(packet.len() >= 12, "short RTP packet");
    assert_eq!(packet[0], 0x80, "version 2, no padding, extension, or CSRC");
    Rtp {
        marker: packet[1] & 0x80 != 0,
        pt: packet[1] & 0x7F,
        seq: u16::from_be_bytes([packet[2], packet[3]]),
        timestamp: u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]),
        ssrc: u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]),
        payload: packet.len() - 12,
    }
}
