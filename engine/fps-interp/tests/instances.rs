// SPDX-License-Identifier: GPL-3.0-only
//! Scenario instances with `after` ordering, `spawn`, shared variables, and
//! failure propagation, with both sides of every flow running against each
//! other over an in memory network.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use fps_flow::function::NoFunctions;
use fps_flow::program::Flow;
use fps_flow::template::Filler;
use fps_flow::{ArgValue, FnCall, FnError, FnErrorKind, Scenario, TemplateFn, Value};
use fps_interp::{
    session_rng, DatagramOpener, FailureKind, FlowEnv, FlowState, InstanceConfig, Limits, OpenKind,
    OpenRequest, Opener, Reason, ScenarioInstance, Session, SessionCtx, Side, Sides,
};
use fps_io::memory::{datagram_pair, duplex, MemoryDatagram, MemoryIo};
use rand::RngCore;
use tokio::io::AsyncWriteExt;
use tokio::sync::oneshot;

fn filler() -> &'static Filler {
    static FILLER: OnceLock<Filler> = OnceLock::new();
    FILLER.get_or_init(|| Filler::new(9))
}

fn compile(text: &str) -> Scenario {
    let mut s = Scenario::parse_instance(text, None).unwrap();
    s.fold_constants().unwrap();
    s
}

fn config(sides: Sides) -> InstanceConfig {
    InstanceConfig {
        sides,
        run_seed: 42,
        first_session_id: 100,
        limits: Limits::default(),
    }
}

/// One open in the order the sides asked for it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Open {
    flow: String,
    kind: OpenKind,
    host: Option<Vec<u8>>,
    port: u16,
}

/// Connections that wait for the other side of one port.
struct Rendezvous<T> {
    backlog: HashMap<u16, VecDeque<T>>,
    waiting: HashMap<u16, VecDeque<oneshot::Sender<T>>>,
}

impl<T> Default for Rendezvous<T> {
    fn default() -> Self {
        Rendezvous {
            backlog: HashMap::new(),
            waiting: HashMap::new(),
        }
    }
}

impl<T> Rendezvous<T> {
    /// Hands the server end of a new connection to an accept on `port`.
    fn deliver(&mut self, port: u16, end: T) {
        let mut end = end;
        while let Some(tx) = self.waiting.get_mut(&port).and_then(VecDeque::pop_front) {
            match tx.send(end) {
                Ok(()) => return,
                Err(back) => end = back,
            }
        }
        self.backlog.entry(port).or_default().push_back(end);
    }

    /// Takes a queued connection or registers an accept on `port`.
    fn take(&mut self, port: u16) -> Result<T, oneshot::Receiver<T>> {
        if let Some(end) = self.backlog.get_mut(&port).and_then(VecDeque::pop_front) {
            return Ok(end);
        }
        let (tx, rx) = oneshot::channel();
        self.waiting.entry(port).or_default().push_back(tx);
        Err(rx)
    }
}

/// An in memory network on which `connect` reaches the `accept` of the same
/// port, and which logs every open.
#[derive(Clone, Default)]
struct MemNet {
    streams: Arc<Mutex<Rendezvous<MemoryIo>>>,
    datagrams: Arc<Mutex<Rendezvous<MemoryDatagram>>>,
    opens: Arc<Mutex<Vec<Open>>>,
}

impl MemNet {
    fn log(&self, flow: &str, request: &OpenRequest<'_>) {
        self.opens.lock().unwrap().push(Open {
            flow: flow.to_string(),
            kind: request.kind,
            host: request.host.map(<[u8]>::to_vec),
            port: request.port,
        });
    }

    fn opens(&self) -> Vec<Open> {
        self.opens.lock().unwrap().clone()
    }
}

async fn accept<T>(table: &Mutex<Rendezvous<T>>, port: u16) -> io::Result<T> {
    let taken = table.lock().unwrap().take(port);
    match taken {
        Ok(end) => Ok(end),
        Err(rx) => rx
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::NotConnected, "network dropped")),
    }
}

struct StreamEnd {
    net: MemNet,
    flow: String,
}

impl Opener for StreamEnd {
    type Io = MemoryIo;

    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<MemoryIo>> {
        self.net.log(&self.flow, &request);
        let net = self.net.clone();
        let (kind, port) = (request.kind, request.port);
        async move {
            match kind {
                OpenKind::Connect => {
                    let (client, server) = duplex(64 << 10);
                    net.streams.lock().unwrap().deliver(port, server);
                    Ok(client)
                }
                OpenKind::Accept => accept(&net.streams, port).await,
            }
        }
    }
}

struct DatagramEnd {
    net: MemNet,
    flow: String,
}

impl DatagramOpener for DatagramEnd {
    type Io = MemoryDatagram;

    fn open(
        &mut self,
        request: OpenRequest<'_>,
    ) -> impl Future<Output = io::Result<MemoryDatagram>> {
        self.net.log(&self.flow, &request);
        let net = self.net.clone();
        let (kind, port) = (request.kind, request.port);
        async move {
            match kind {
                OpenKind::Connect => {
                    let (client, server) = datagram_pair(16);
                    net.datagrams.lock().unwrap().deliver(port, server);
                    Ok(client)
                }
                OpenKind::Accept => {
                    let io = accept(&net.datagrams, port).await?;
                    // The first datagram stays queued for the first expect.
                    io.readable().await?;
                    Ok(io)
                }
            }
        }
    }
}

/// Test doubles of the `dns` and `ftp` module functions.
struct Modules;

fn bytes_arg<'a>(call: &FnCall<'a>) -> Option<&'a [u8]> {
    match call.args.positional.first()? {
        ArgValue::Bytes(b) => Some(b),
        ArgValue::Text(t) => Some(t.as_bytes()),
        _ => None,
    }
}

impl TemplateFn for Modules {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        _rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        let bad = |m: &str| FnError::new(FnErrorKind::InvalidArgument, m);
        match (call.module, call.function) {
            // A query with a fixed id for the name in `name`, type A only.
            ("dns", "query") => {
                let name = call
                    .args
                    .named
                    .iter()
                    .find(|a| a.name == "name")
                    .and_then(|a| match a.value {
                        ArgValue::Text(t) => Some(t),
                        _ => None,
                    })
                    .ok_or_else(|| bad("name"))?;
                out.extend_from_slice(&[0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
                for label in name.split('.') {
                    out.push(u8::try_from(label.len()).map_err(|_| bad("label"))?);
                    out.extend_from_slice(label.as_bytes());
                }
                out.extend_from_slice(&[0, 0, 1, 0, 1]);
                Ok(())
            }
            // The address of the last answer, which ends the response.
            ("dns", "answer_ip") => {
                let msg = bytes_arg(call).ok_or_else(|| bad("message"))?;
                let ip = msg
                    .get(msg.len().saturating_sub(4)..)
                    .ok_or_else(|| bad("short"))?;
                let text = format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
                out.extend_from_slice(text.as_bytes());
                Ok(())
            }
            // The port of a reply such as "227 2121".
            ("ftp", "pasv_port") => {
                let reply = bytes_arg(call).ok_or_else(|| bad("reply"))?;
                let digits: Vec<u8> = reply
                    .iter()
                    .skip(4)
                    .copied()
                    .take_while(u8::is_ascii_digit)
                    .collect();
                out.extend_from_slice(&digits);
                Ok(())
            }
            _ => Err(FnError::new(FnErrorKind::Unsupported, "unknown function")),
        }
    }
}

struct Env {
    net: MemNet,
}

impl FlowEnv for Env {
    type Stream = StreamEnd;
    type Datagram = DatagramEnd;

    fn stream(&mut self, flow: &Flow, _side: Side) -> StreamEnd {
        StreamEnd {
            net: self.net.clone(),
            flow: flow.id.clone(),
        }
    }

    fn datagram(&mut self, flow: &Flow, _side: Side) -> DatagramEnd {
        DatagramEnd {
            net: self.net.clone(),
            flow: flow.id.clone(),
        }
    }

    fn functions(&mut self, _flow: &Flow, _side: Side) -> Box<dyn TemplateFn> {
        Box::new(Modules)
    }
}

/// The web_browse scenario of the specification, which resolves a name and
/// then fetches a page from the address that the lookup bound. The engine
/// runs no transport modules yet, so the page flow is plain HTTP, and the
/// `dns` functions are the test doubles above.
const WEB_BROWSE: &str = r#"
scenario: web_browse
vars:
  host: www.example.test
  dns_port: 53
  server_port: 8080
  page_size: 18000
flows:
  - id: page
    after: lookup
    module: http1
    client:
      - connect: { host: "{{ vars.server_ip }}", port: "{{ vars.server_port }}" }
      - send: "GET / HTTP/1.1\r\nHost: {{ vars.host }}\r\n\r\n"
      - expect: { pattern: "\r\n\r\n", timeout_ms: 5000 }
      - expect: { bytes: "{{ vars.page_size }}", timeout_ms: 5000 }
      - close: fin
    server:
      - accept: { port: "{{ vars.server_port }}" }
      - expect: { pattern: "\r\n\r\n", timeout_ms: 5000 }
      - send: "HTTP/1.1 200 OK\r\nContent-Length: {{ vars.page_size }}\r\n\r\n{{ fill(vars.page_size) }}"
      - close: fin
  - id: lookup
    module: dns
    l4: udp
    client:
      - connect: { port: "{{ vars.dns_port }}" }
      - send: "{{ dns.query(name=vars.host, qtype='A') }}"
      - expect: { min_bytes: 12, timeout_ms: 2000 }
      - set: { server_ip: "{{ dns.answer_ip(last) }}" }
      - close: fin
    server:
      - accept: { port: "{{ vars.dns_port }}" }
      - expect: { min_bytes: 12, timeout_ms: 2000 }
      - send: "{{ hex('12348180000100010000000003777777076578616d706c6504746573740000010001c00c000100010000012c0004c0000201') }}"
      - close: fin
"#;

#[tokio::test]
async fn web_browse_runs_lookup_before_page() {
    let scenario = compile(WEB_BROWSE);
    let net = MemNet::default();
    let mut env = Env { net: net.clone() };
    let mut instance = ScenarioInstance::new(&scenario, config(Sides::Both));
    instance
        .run(&mut env, filler(), std::future::pending())
        .await
        .unwrap();

    let opens = net.opens();
    let flows: Vec<&str> = opens.iter().map(|o| o.flow.as_str()).collect();
    assert_eq!(flows, ["lookup", "lookup", "page", "page"], "{opens:?}");
    let records: Vec<(&str, Side)> = instance
        .records()
        .iter()
        .map(|r| (r.flow.as_str(), r.side))
        .collect();
    let first_page = records.iter().position(|r| r.0 == "page").unwrap();
    assert!(records[..first_page].iter().all(|r| r.0 == "lookup"));
    assert_eq!(first_page, 2, "both lookup sides finish before page starts");

    // The page dials the address that the lookup bound with set.
    let dial = opens
        .iter()
        .find(|o| o.flow == "page" && o.kind == OpenKind::Connect)
        .unwrap();
    assert_eq!(dial.host.as_deref(), Some(&b"192.0.2.1"[..]));
    assert_eq!(dial.port, 8080);
    assert_eq!(
        instance.vars().get("server_ip"),
        Some(&Value::Str("192.0.2.1".into()))
    );
    assert_eq!(instance.state("lookup"), Some(FlowState::Done));
    assert_eq!(instance.state("page"), Some(FlowState::Done));

    let page_client = instance
        .records()
        .iter()
        .find(|r| r.flow == "page" && r.side == Side::Client)
        .unwrap();
    assert_eq!(page_client.failed, None);
    assert!(page_client.stats.bytes_rx >= 18000);
    // Both sides of one flow session share an id, and flows take ids in
    // start order.
    let ids: Vec<(&str, u64)> = instance
        .records()
        .iter()
        .map(|r| (r.flow.as_str(), r.session_id))
        .collect();
    assert!(ids.contains(&("lookup", 100)) && ids.contains(&("page", 101)));
}

#[tokio::test]
async fn one_sided_instances_order_their_own_sides() {
    // A client simulator and a server simulator each run one side of every
    // flow, and each orders its own sides by after.
    let scenario = compile(WEB_BROWSE);
    let net = MemNet::default();
    let mut env = Env { net: net.clone() };
    let mut servers = ScenarioInstance::new(&scenario, config(Sides::Server));
    let mut client = ScenarioInstance::new(&scenario, config(Sides::Client));
    let mut env2 = Env { net };
    let (a, b) = tokio::join!(
        servers.run(&mut env, filler(), std::future::pending()),
        client.run(&mut env2, filler(), std::future::pending()),
    );
    a.unwrap();
    b.unwrap();
    assert!(client.records().iter().all(|r| r.side == Side::Client));
    assert!(servers.records().iter().all(|r| r.side == Side::Server));
    let order: Vec<&str> = client.records().iter().map(|r| r.flow.as_str()).collect();
    assert_eq!(order, ["lookup", "page"]);
}

/// A control channel that announces a passive data port, spawns the data
/// flow on both sides, and waits for the transfer to finish. `{size}` is the
/// size that the server sends on the data flow.
fn ftp(size: usize) -> String {
    format!(
        r#"
scenario: ftp_retr
vars:
  ctl_port: 21
  want: 4000
flows:
  - id: ctl
    module: ftp
    client:
      - connect: {{ port: "{{{{ vars.ctl_port }}}}" }}
      - send: "PASV\r\n"
      - expect: {{ pattern: "\r\n", timeout_ms: 2000 }}
      - set: {{ data_port: "{{{{ ftp.pasv_port(last) }}}}" }}
      - spawn: {{ flow: data }}
      - expect: {{ pattern: "226\r\n", timeout_ms: 2000 }}
      - close: fin
    server:
      - accept: {{ port: "{{{{ vars.ctl_port }}}}" }}
      - expect: {{ pattern: "\r\n", timeout_ms: 2000 }}
      - set: {{ data_port: 2121 }}
      - spawn: {{ flow: data }}
      - send: "227 2121\r\n"
      - delay: {{ ms: 50 }}
      - send: "226\r\n"
      - close: fin
  - id: data
    module: ftp
    spawned: true
    client:
      - connect: {{ port: "{{{{ vars.data_port }}}}" }}
      - expect: {{ bytes: "{{{{ vars.want }}}}", timeout_ms: 2000 }}
      - close: fin
    server:
      - accept: {{ port: "{{{{ vars.data_port }}}}" }}
      - send: "{{{{ fill({size}) }}}}"
      - close: fin
  - id: quit
    module: ftp
    after: [ctl, data]
    client:
      - connect: {{ port: 2100 }}
      - close: rst
    server:
      - accept: {{ port: 2100 }}
      - close: rst
"#
    )
}

#[tokio::test]
async fn spawn_starts_the_data_flow_with_the_parent_variables() {
    let scenario = compile(&ftp(4000));
    let net = MemNet::default();
    let mut env = Env { net: net.clone() };
    let mut instance = ScenarioInstance::new(&scenario, config(Sides::Both));
    instance
        .run(&mut env, filler(), std::future::pending())
        .await
        .unwrap();
    let opens = net.opens();
    let dial = opens
        .iter()
        .find(|o| o.flow == "data" && o.kind == OpenKind::Connect)
        .unwrap();
    // The client side read the port out of the reply before the spawn.
    assert_eq!(dial.port, 2121);
    for flow in ["ctl", "data", "quit"] {
        assert_eq!(instance.state(flow), Some(FlowState::Done), "{flow}");
    }
    // quit waits for the spawned flow as well as the control flow.
    let records: Vec<&str> = instance.records().iter().map(|r| r.flow.as_str()).collect();
    let first_quit = records.iter().position(|f| *f == "quit").unwrap();
    assert_eq!(
        records[..first_quit]
            .iter()
            .filter(|f| **f == "data")
            .count(),
        2
    );
    let data_ids: Vec<u64> = instance
        .records()
        .iter()
        .filter(|r| r.flow == "data")
        .map(|r| r.session_id)
        .collect();
    assert_eq!(data_ids, [101, 101]);
}

#[tokio::test(start_paused = true)]
async fn spawned_flow_failure_fails_its_parent_and_the_instance() {
    // The data server sends less than the client expects and closes.
    let scenario = compile(&ftp(1000));
    let mut env = Env {
        net: MemNet::default(),
    };
    let mut instance = ScenarioInstance::new(&scenario, config(Sides::Both));
    let err = instance
        .run(&mut env, filler(), std::future::pending())
        .await
        .unwrap_err();
    assert_eq!(err.scenario, "ftp_retr");
    assert_eq!(err.flow, "data");
    assert_eq!(err.spawned_by, ["ctl"]);
    assert_eq!(err.failure.side, Side::Client);
    assert_eq!(err.failure.reason, Reason::PeerClosed);
    assert!(err.to_string().contains("spawned by ctl"), "{err}");
    assert_eq!(instance.state("data"), Some(FlowState::Failed));
    assert_eq!(instance.state("ctl"), Some(FlowState::Failed));
    assert_eq!(instance.state("quit"), Some(FlowState::Skipped));
    // The instance aborted the control sides that still ran.
    assert!(instance
        .later_failures()
        .iter()
        .any(|f| f.flow == "ctl" && f.reason == Reason::Aborted));
}

#[tokio::test(start_paused = true)]
async fn failed_flow_skips_its_dependents() {
    let text = r#"
scenario: t
flows:
  - id: a
    module: m
    client:
      - connect: { port: 1 }
      - expect: { bytes: 10, timeout_ms: 100 }
    server:
      - accept: { port: 1 }
      - send: "short"
      - close: fin
  - { id: b, module: m, after: a, client: [ { connect: { port: 2 } } ] }
"#;
    let scenario = compile(text);
    let mut env = Env {
        net: MemNet::default(),
    };
    let mut instance = ScenarioInstance::new(&scenario, config(Sides::Both));
    let err = instance
        .run(&mut env, filler(), std::future::pending())
        .await
        .unwrap_err();
    assert_eq!(err.flow, "a");
    assert!(err.spawned_by.is_empty());
    assert_eq!(instance.state("a"), Some(FlowState::Failed));
    assert_eq!(instance.state("b"), Some(FlowState::Skipped));
}

#[tokio::test(start_paused = true)]
async fn stop_aborts_the_running_flows() {
    let text = r#"
scenario: t
flows:
  - id: a
    module: m
    client:
      - connect: { port: 1 }
      - expect: { bytes: 10, timeout_ms: 60000 }
"#;
    let scenario = compile(text);
    let mut env = Env {
        net: MemNet::default(),
    };
    let mut instance = ScenarioInstance::new(&scenario, config(Sides::Client));
    let err = instance
        .run(
            &mut env,
            filler(),
            tokio::time::sleep(Duration::from_millis(10)),
        )
        .await
        .unwrap_err();
    assert_eq!(err.failure.reason, Reason::Aborted);
    assert_eq!(instance.state("a"), Some(FlowState::Failed));
}

/// Hands out one end of a duplex.
struct One(Option<MemoryIo>);

impl Opener for One {
    type Io = MemoryIo;

    fn open(&mut self, _request: OpenRequest<'_>) -> impl Future<Output = io::Result<MemoryIo>> {
        let io = self.0.take();
        async move { io.ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "used")) }
    }
}

#[tokio::test]
async fn a_single_session_rejects_spawn() {
    let scenario = compile(&ftp(10));
    let flow = &scenario.flows[0];
    let (io, mut peer) = duplex(1024);
    peer.write_all(b"PASV\r\n").await.unwrap();
    let mut functions = NoFunctions;
    let mut session = Session::new(SessionCtx {
        vars: &scenario.vars,
        filler: filler(),
        flow: &flow.id,
        side: Side::Server,
        session_id: 1,
        rng: session_rng(1, 1, Side::Server),
        functions: &mut functions,
        limits: Limits::default(),
    });
    let server = flow.server.as_ref().unwrap();
    let failure = session
        .run_program(&mut One(Some(io)), server, std::future::pending())
        .await
        .unwrap_err();
    assert_eq!(failure.action_type, "spawn");
    assert_eq!(failure.action_index, 3);
    assert!(matches!(*failure.kind, FailureKind::Invalid(_)));
}
