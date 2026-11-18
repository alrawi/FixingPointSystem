// SPDX-License-Identifier: GPL-3.0-only
//! Workers over a fake transport (ENG-31).
//!
//! The fake keeps every session in memory: a `connect` finds the listener
//! that holds the destination address and hands it the other end of a duplex.
//! No socket opens, so the test shows that a worker reaches the network only
//! through [`Transport`], and it records what the workers asked the transport
//! for.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fps_backend_kernel::ConnectConfig;
use fps_flow::Scenario;
use fps_hosts::{HostPool, HostSelector, PoolSpec, Selection};
use fps_interp::Side;
use fps_io::memory::duplex;
use fps_runtime::transport::{
    BoxFuture, DatagramListener, DatagramSession, StreamListener, StreamSession, Transport,
};
use fps_runtime::{
    ClientConfig, FlowRef, Role, Runtime, RuntimeConfig, ServerConfig, SourceHosts, WorkerConfig,
};
use fps_sched::{stream_id, Arrival, LoadProfile, Objective, Phase, ScheduleConfig, Shape};
use fps_stats::metric;
use tokio::sync::mpsc;

const SEED: u64 = 20_261_118;

/// The first port that the fake hands out for a listen on port 0.
const FIRST_FAKE_PORT: u16 = 41_000;

/// A network in memory: the listeners by address, and a log of the calls.
#[derive(Debug, Default)]
struct FakeNet {
    listeners: Mutex<HashMap<SocketAddr, mpsc::UnboundedSender<StreamSession>>>,
    /// The addresses of every `listen` call, in call order.
    listens: Mutex<Vec<Vec<SocketAddr>>>,
    /// The `(source, destination)` of every `connect`.
    connects: Mutex<Vec<(IpAddr, SocketAddr)>>,
    next_port: AtomicU16,
}

impl FakeNet {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            next_port: AtomicU16::new(FIRST_FAKE_PORT),
            ..Self::default()
        })
    }
}

impl Transport for FakeNet {
    fn connect(
        &self,
        source: IpAddr,
        destination: SocketAddr,
    ) -> BoxFuture<'_, io::Result<StreamSession>> {
        Box::pin(async move {
            self.connects.lock().unwrap().push((source, destination));
            let listener = self.listeners.lock().unwrap().get(&destination).cloned();
            let listener = listener.ok_or(io::ErrorKind::ConnectionRefused)?;
            let (client, server) = duplex(64 * 1024);
            let client_addr =
                SocketAddr::new(source, self.next_port.fetch_add(1, Ordering::Relaxed));
            let accepted = StreamSession::new(server, Some(destination), Some(client_addr));
            listener
                .send(accepted)
                .map_err(|_| io::ErrorKind::ConnectionRefused)?;
            Ok(StreamSession::new(
                client,
                Some(client_addr),
                Some(destination),
            ))
        })
    }

    fn listen(&self, addresses: &[SocketAddr]) -> io::Result<Box<dyn StreamListener>> {
        self.listens.lock().unwrap().push(addresses.to_vec());
        let (tx, rx) = mpsc::unbounded_channel();
        let mut bound = Vec::new();
        let mut listeners = self.listeners.lock().unwrap();
        for a in addresses {
            let port = match a.port() {
                0 => self.next_port.fetch_add(1, Ordering::Relaxed),
                p => p,
            };
            let a = SocketAddr::new(a.ip(), port);
            if listeners.insert(a, tx.clone()).is_some() {
                return Err(io::ErrorKind::AddrInUse.into());
            }
            bound.push(a);
        }
        Ok(Box::new(FakeListener { rx, bound }))
    }

    fn connect_datagram(&self, _: IpAddr, _: SocketAddr) -> io::Result<DatagramSession> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn listen_datagram(&self, _: &[SocketAddr]) -> io::Result<Box<dyn DatagramListener>> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

struct FakeListener {
    rx: mpsc::UnboundedReceiver<StreamSession>,
    bound: Vec<SocketAddr>,
}

impl StreamListener for FakeListener {
    fn local_addrs(&self) -> Vec<SocketAddr> {
        self.bound.clone()
    }

    fn accept(&mut self) -> BoxFuture<'_, io::Result<StreamSession>> {
        Box::pin(async move {
            self.rx
                .recv()
                .await
                .ok_or_else(|| io::ErrorKind::BrokenPipe.into())
        })
    }
}

fn web_get() -> Arc<Scenario> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/examples/modules/http1/scenarios/web_get.yaml");
    let mut s = Scenario::load(&path).unwrap();
    s.fold_constants().unwrap();
    Arc::new(s)
}

fn server_worker(
    id: u32,
    scenario: &Arc<Scenario>,
    listen: Vec<SocketAddr>,
    net: &Arc<FakeNet>,
) -> WorkerConfig {
    WorkerConfig {
        id,
        core: None,
        transport: Some(Arc::clone(net) as Arc<dyn Transport>),
        role: Role::Server(ServerConfig {
            flow: FlowRef::new(Arc::clone(scenario), 0),
            listen,
            config: Default::default(),
        }),
    }
}

fn client_worker(
    id: u32,
    scenario: &Arc<Scenario>,
    servers: Vec<IpAddr>,
    port: u16,
    net: &Arc<FakeNet>,
) -> WorkerConfig {
    // These addresses exist in no interface of the machine, so a worker that
    // fell back to the kernel backend would fail to bind them.
    let hosts = HostPool::parse(&PoolSpec {
        name: "clients",
        ipv4: Some("198.51.100.0/24"),
        count: 4,
        ..PoolSpec::default()
    })
    .unwrap();
    let profile = LoadProfile::new(vec![Phase::new(
        "steady",
        Duration::from_secs(1),
        Shape::Hold,
        1.0,
        1.0,
    )])
    .unwrap();
    let mut schedule = ScheduleConfig::new(Objective::SessionsPerSecond(20.0), profile, SEED);
    schedule.arrival = Arrival::Constant;
    schedule.stream = stream_id(id, 0);
    WorkerConfig {
        id,
        core: None,
        transport: Some(Arc::clone(net) as Arc<dyn Transport>),
        role: Role::Client(Box::new(ClientConfig {
            flow: FlowRef::new(Arc::clone(scenario), 0),
            schedule,
            servers,
            port: Some(port),
            sources: SourceHosts {
                selector: HostSelector::new(
                    "clients",
                    hosts.range(),
                    Selection::RoundRobin,
                    SEED,
                    id,
                )
                .unwrap(),
                pool: hosts,
                ipv6: false,
            },
            connect: ConnectConfig::default(),
        })),
    }
}

#[test]
fn a_client_and_a_server_worker_run_a_scenario_over_a_fake_transport() {
    let scenario = web_get();
    let net = FakeNet::new();
    let server_a: SocketAddr = "203.0.113.1:8080".parse().unwrap();
    let server_b: SocketAddr = "203.0.113.2:8080".parse().unwrap();
    // One worker listens on two hosts of one port, and a second one on a port
    // of the transport's choice.
    let servers = Runtime::start(RuntimeConfig::new(
        SEED,
        vec![
            server_worker(10, &scenario, vec![server_a, server_b], &net),
            server_worker(11, &scenario, vec!["203.0.113.3:0".parse().unwrap()], &net),
        ],
    ))
    .unwrap();

    // The transport hears one listen for the two hosts of one port, and the
    // bound addresses come back in the order of the configuration.
    // The workers run on threads of their own, so the calls come in any order.
    let mut listens = net.listens.lock().unwrap().clone();
    listens.sort();
    assert_eq!(
        listens,
        vec![
            vec![server_a, server_b],
            vec!["203.0.113.3:0".parse().unwrap()]
        ]
    );
    let bound = servers.listen_addrs();
    assert_eq!(bound.len(), 3);
    assert_eq!(&bound[..2], &[server_a, server_b]);
    assert_eq!(bound[2].port(), FIRST_FAKE_PORT);

    let clients = Runtime::start(RuntimeConfig::new(
        SEED,
        vec![client_worker(
            0,
            &scenario,
            vec![server_a.ip(), server_b.ip()],
            8080,
            &net,
        )],
    ))
    .unwrap();
    let client = clients.join();
    servers.stop(Duration::from_secs(1));
    let server = servers.join();

    for (id, r) in client.workers.iter().chain(&server.workers) {
        assert!(r.is_ok(), "worker {id}: {r:?}");
    }
    let c = client.collector.total(Side::Client).unwrap();
    let s = server.collector.total(Side::Server).unwrap();
    // Constant arrivals at 20 per second for a second give 20 sessions.
    assert_eq!(c.counter_total(metric::SESSIONS_ATTEMPTED), 20);
    assert_eq!(c.counter_total(metric::SESSIONS_SUCCEEDED), 20);
    assert_eq!(c.counter_total(metric::SESSIONS_FAILED), 0);
    assert_eq!(s.counter_total(metric::SESSIONS_SUCCEEDED), 20);
    assert_eq!(s.counter_total(metric::SESSIONS_FAILED), 0);
    assert_eq!(
        c.counter_total(metric::BYTES_TX),
        s.counter_total(metric::BYTES_RX)
    );
    assert_eq!(
        c.counter_total(metric::BYTES_RX),
        s.counter_total(metric::BYTES_TX)
    );
    // Every session went through the fake, from a host of the client pool to a
    // server host in turn.
    let connects = net.connects.lock().unwrap();
    assert_eq!(connects.len(), 20);
    assert!(connects
        .iter()
        .all(|(src, dst)| src.to_string().starts_with("198.51.100.")
            && (*dst == server_a || *dst == server_b)));
    assert_eq!(connects.iter().filter(|(_, d)| *d == server_a).count(), 10);
}

#[test]
fn a_listen_that_the_transport_refuses_fails_the_start_with_its_error() {
    let scenario = web_get();
    let net = FakeNet::new();
    let taken: SocketAddr = "203.0.113.9:8080".parse().unwrap();
    let first = Runtime::start(RuntimeConfig::new(
        SEED,
        vec![server_worker(1, &scenario, vec![taken], &net)],
    ))
    .unwrap();
    let err = Runtime::start(RuntimeConfig::new(
        SEED,
        vec![server_worker(2, &scenario, vec![taken], &net)],
    ))
    .unwrap_err();
    let text = err.to_string();
    assert!(text.contains("worker 2: listen: address in use"), "{text}");
    first.abort();
    let _ = first.join();
}
