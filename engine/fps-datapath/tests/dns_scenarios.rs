// SPDX-License-Identifier: GPL-3.0-only
//! The dns module over the user space datapath (TRN-29).
//!
//! Both sides of every flow of every dns scenario run through the datapath:
//! the client side connects from a client host of the pools and the server
//! side accepts at a server host. UDP flows go through the user space UDP
//! layer (`fps-udp-own`, behind `DatagramIo`) and the TCP flow of
//! `tcp_fallback` through the own TCP stack, both over the same lane.
//!
//! * `dns_scenarios_complete_over_the_datapath_in_memory` needs no root: the
//!   two arms of the lane are the ends of an in memory frame pair.
//! * `dns_scenarios_complete_over_af_xdp_on_a_veth_pair` makes a network
//!   namespace with a veth pair, `dnsa` and `dnsb`, and runs this test binary
//!   again inside it (the hidden test `helper_dns`). The helper opens an
//!   AF_XDP queue on each end of the pair and gives them to the datapath as
//!   the client arm and the server arm of one lane, so every frame of every
//!   scenario leaves one AF_XDP socket and arrives at the other. It follows
//!   the pattern of `datapath_veth` of `fps-backend-afxdp`.
//! * `dns_client_over_af_xdp_completes_against_a_kernel_server` gives the
//!   other end of the pair to the kernel: the client side of every flow runs
//!   over the AF_XDP datapath, and the server side of every flow runs over
//!   kernel sockets (`fps-backend-kernel`) bound to a server host address of
//!   the pools. The kernel verifies the checksums and the framing that the
//!   user space layers send, resolves the client hosts with ARP (which the
//!   splitter answers), and its own frames have the transmit checksum offload
//!   turned off, because the veth hands partial checksums to an AF_XDP end
//!   (TRN-26).
//!
//! Every test exists for IPv4 and for IPv6 (TRN-30): the IPv6 runs use pools
//! of IPv6 prefixes (`fd00:1::/64` and `fd00:2::/64`), so the flows, the
//! checksums, and the neighbor entries are IPv6, and the splitter answers
//! neighbor discovery for the kernel server. The same file runs the loss
//! workload of `fps-tcp` (TRN-22) over the datapath: `DatapathStack` makes the
//! datapath a `TcpStack`, and a lossy queue around each arm drops frames at
//! random (a veth end of an AF_XDP socket bypasses `netem`). The loss tests run
//! in memory and on the veth pair, over both families.
//!
//! AF_XDP needs root. As a normal user the veth test runs itself again
//! through `sudo -n` and skips with a message when neither works. `CI=true` or
//! `FPS_REQUIRE_AFXDP=1` turns every skip into a failure. `FPS_DNS_REPS` sets
//! how often each scenario runs (default 5), and `FPS_DNS_RESULTS` names a
//! directory that receives `dns-veth.txt`.

use std::collections::HashMap;
use std::future::Future;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use fps_backend_afxdp::memory::frame_pair;
use fps_backend_afxdp::{
    open_port, BindMode, BoundMode, FrameIo, OpenedPort, PortConfig, QueueConfig, QueueStats,
};
use fps_backend_kernel::{KernelDatagramListener, KernelListener, ListenConfig, SocketOptions};
use fps_datapath::{
    split_pools, Connector, Datapath, DatapathConfig, DatapathSession, Lane, LaneStats, UdpListener,
};
use fps_flow::program::L4;
use fps_flow::template::Filler;
use fps_flow::{Manifest, ModuleSet, Scenario, Value, Vars};
use fps_interp::{
    session_rng, DatagramOpener, Limits, OpenRequest, Opener, Session, SessionCtx, Side,
};
use fps_io::{DatagramIo, SessionIo};
use fps_module_dns::{Dns, Message, Rcode, RecordType};
use fps_tcp::workload::{loss_transfer, LossConfig, LossRun};
use fps_tcp::{StackStats, TcpStack};
use fps_tcp_own::Pools;

const MARK: &str = "fps-dns ";
const SEED: u64 = 42;

// ---------------------------------------------------------------------------
// The scenarios and what each flow must show
// ---------------------------------------------------------------------------

/// What one flow of a scenario must show (the table of `modules/dns/tests`).
struct Want {
    scenario: &'static str,
    flow: &'static str,
    l4: L4,
    qname: &'static str,
    qtype: RecordType,
    rcode: Rcode,
    truncated: bool,
    answers: &'static [RecordType],
    additional: usize,
    authority: usize,
    server_ip: Option<&'static str>,
}

const fn udp(
    scenario: &'static str,
    qname: &'static str,
    qtype: RecordType,
    rcode: Rcode,
    answers: &'static [RecordType],
) -> Want {
    Want {
        scenario,
        flow: "query",
        l4: L4::Udp,
        qname,
        qtype,
        rcode,
        truncated: false,
        answers,
        additional: 0,
        authority: 0,
        server_ip: None,
    }
}

fn wants() -> Vec<Want> {
    use RecordType::{Aaaa, Cname, Mx, Txt, A};
    vec![
        Want {
            server_ip: Some("192.0.2.1"),
            ..udp("lookup_a", "www.example.test", A, Rcode::NoError, &[A])
        },
        Want {
            server_ip: Some("2001:db8::1"),
            ..udp(
                "lookup_aaaa",
                "www.example.test",
                Aaaa,
                Rcode::NoError,
                &[Aaaa],
            )
        },
        Want {
            server_ip: Some("192.0.2.80"),
            ..udp(
                "lookup_cname",
                "static.example.test",
                A,
                Rcode::NoError,
                &[Cname, Cname, A],
            )
        },
        Want {
            additional: 3,
            ..udp("lookup_mx", "example.test", Mx, Rcode::NoError, &[Mx, Mx])
        },
        udp("lookup_txt", "example.test", Txt, Rcode::NoError, &[Txt]),
        Want {
            authority: 1,
            ..udp("nxdomain", "missing.example.test", A, Rcode::NxDomain, &[])
        },
        Want {
            flow: "udp",
            truncated: true,
            ..udp("tcp_fallback", "big.example.test", Txt, Rcode::NoError, &[])
        },
        Want {
            flow: "tcp",
            l4: L4::Tcp,
            ..udp(
                "tcp_fallback",
                "big.example.test",
                Txt,
                Rcode::NoError,
                &[Txt],
            )
        },
    ]
}

fn module_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/dns")
}

/// Loads every scenario of the manifest, checked against the module catalog.
fn scenarios() -> Vec<(String, Scenario)> {
    let manifest = Manifest::load(&module_dir().join("manifest.yaml")).unwrap();
    let mut set = ModuleSet::default();
    set.insert(manifest.clone());
    let ctx = set.context("dns").unwrap();
    manifest
        .scenarios
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

fn text(vars: &Vars, name: &str) -> Option<String> {
    vars.get(name).map(|v| match v {
        Value::Str(t) => t.clone(),
        other => format!("{other:?}"),
    })
}

// ---------------------------------------------------------------------------
// The openers: connect and accept over the datapath
// ---------------------------------------------------------------------------

type Sent = Arc<Mutex<Vec<Vec<u8>>>>;

/// A UDP session that records every datagram that it sends.
struct Tap {
    inner: Box<dyn DatagramIo>,
    sent: Sent,
}

impl DatagramIo for Tap {
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

struct UdpClient<'a> {
    connector: &'a Connector,
    source: IpAddr,
    server: IpAddr,
    sent: Sent,
}

impl DatagramOpener for UdpClient<'_> {
    type Io = Tap;

    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<Tap>> {
        let opened = self
            .connector
            .connect_udp(self.source, SocketAddr::new(self.server, request.port))
            .map(|inner| Tap {
                inner: Box::new(inner),
                sent: Arc::clone(&self.sent),
            })
            .map_err(io::Error::other);
        async move { opened }
    }
}

/// Where the server side of a UDP flow accepts: the datapath or the kernel.
enum UdpAcceptor<'a> {
    Datapath(&'a mut UdpListener),
    Kernel(&'a mut KernelDatagramListener),
}

struct UdpServer<'a> {
    acceptor: UdpAcceptor<'a>,
    sent: Sent,
}

impl DatagramOpener for UdpServer<'_> {
    type Io = Tap;

    fn open(&mut self, _request: OpenRequest<'_>) -> impl Future<Output = io::Result<Tap>> {
        let sent = Arc::clone(&self.sent);
        async move {
            // The session keeps the first datagram queued for its first read.
            let inner: Box<dyn DatagramIo> = match &mut self.acceptor {
                UdpAcceptor::Datapath(l) => Box::new(l.accept().await?.0),
                UdpAcceptor::Kernel(l) => Box::new(l.accept().await.map_err(io::Error::other)?.0),
            };
            Ok(Tap { inner, sent })
        }
    }
}

struct TcpClient<'a> {
    connector: &'a Connector,
    source: IpAddr,
    server: IpAddr,
}

impl Opener for TcpClient<'_> {
    type Io = DatapathSession;

    fn open(
        &mut self,
        request: OpenRequest<'_>,
    ) -> impl Future<Output = io::Result<DatapathSession>> {
        let (connector, source) = (self.connector, self.source);
        let destination = SocketAddr::new(self.server, request.port);
        async move {
            connector
                .connect(source, destination)
                .await
                .map_err(io::Error::other)
        }
    }
}

enum TcpAcceptor<'a> {
    Datapath(&'a mut fps_datapath::Listener),
    Kernel(&'a KernelListener),
}

struct TcpServer<'a> {
    acceptor: TcpAcceptor<'a>,
}

impl Opener for TcpServer<'_> {
    type Io = Box<dyn SessionIo>;

    async fn open(&mut self, _request: OpenRequest<'_>) -> io::Result<Self::Io> {
        let io: Box<dyn SessionIo> = match &mut self.acceptor {
            TcpAcceptor::Datapath(l) => Box::new(l.accept().await.map_err(io::Error::other)?),
            TcpAcceptor::Kernel(l) => Box::new(l.accept().await.map_err(io::Error::other)?.0),
        };
        Ok(io)
    }
}

/// The listeners of the kernel server of the second test.
struct KernelServer {
    udp: KernelDatagramListener,
    tcp: KernelListener,
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Outcome {
    udp_flows: u64,
    tcp_flows: u64,
    failures: Vec<String>,
}

fn check_udp(tag: &str, queries: &[Vec<u8>], responses: &[Vec<u8>], want: &Want) -> Vec<String> {
    let mut bad = Vec::new();
    if (queries.len(), responses.len()) != (1, 1) {
        return vec![format!(
            "{tag}: {} queries and {} responses",
            queries.len(),
            responses.len()
        )];
    }
    let (Ok(q), Ok(a)) = (Message::parse(&queries[0]), Message::parse(&responses[0])) else {
        return vec![format!("{tag}: a message does not parse")];
    };
    let mut expect = |ok: bool, what: &str| {
        if !ok {
            bad.push(format!("{tag}: {what}"));
        }
    };
    expect(
        !q.is_response() && a.is_response(),
        "query and response bits",
    );
    expect(q.id == a.id, "transaction id");
    expect(q.questions == a.questions, "question echoed");
    expect(
        q.questions[0].name.to_string() == want.qname && q.questions[0].qtype == want.qtype.code(),
        "question name and type",
    );
    expect(a.rcode() == Some(want.rcode), "rcode");
    expect(a.is_truncated() == want.truncated, "truncation bit");
    let types: Vec<u16> = a.answers.iter().map(|x| x.data.type_code()).collect();
    let want_types: Vec<u16> = want.answers.iter().map(|t| t.code()).collect();
    expect(types == want_types, "answer types");
    expect(a.additional.len() == want.additional, "additional records");
    expect(a.authority.len() == want.authority, "authority records");
    expect(responses[0].len() <= 512, "response fits the UDP limit");
    bad
}

/// Runs every flow of every dns scenario `reps` times over `dp`, and checks
/// what each flow shows. The client side always runs through the datapath. The
/// server side does too, unless `kernel` gives the listeners of a kernel
/// server, which then answers at the first server host of the pools.
async fn run_dns(
    dp: &Datapath,
    pools: &Pools,
    reps: u64,
    mut kernel: Option<KernelServer>,
) -> Outcome {
    let wants = wants();
    let loaded = scenarios();
    let (mut udp_listener, mut tcp_listener) = if kernel.is_some() {
        (None, None)
    } else {
        (
            Some(dp.listen_udp(53).unwrap()),
            Some(dp.listen(53).unwrap()),
        )
    };
    let connector = dp.connector();
    let addresses = |slice: &fps_tcp_own::HostSlice| -> Vec<IpAddr> {
        (0..slice.hosts()).filter_map(|i| slice.ip(i)).collect()
    };
    let (clients, mut servers) = (addresses(&pools.client), addresses(&pools.server));
    if kernel.is_some() {
        servers.truncate(1);
    }
    let filler = Filler::new(SEED);
    let dns = Dns::bundled().unwrap();
    let mut out = Outcome::default();
    let mut id = 0u64;
    for _ in 0..reps {
        for (name, s) in &loaded {
            for flow in &s.flows {
                id += 1;
                let tag = format!("{name}.{} (session {id})", flow.id);
                let want = wants
                    .iter()
                    .find(|w| w.scenario == name && w.flow == flow.id)
                    .unwrap_or_else(|| panic!("no expectation for {name}.{}", flow.id));
                // Client hosts take turns and the server host changes after a
                // round, as in the veth test of the datapath.
                let n = usize::try_from(id).unwrap_or(0);
                let source = clients[n % clients.len()];
                let server = servers[(n / clients.len()) % servers.len()];
                let (mut client_fns, mut server_fns) = (dns.clone(), dns.clone());
                let make = |side, functions| SessionCtx {
                    vars: &s.vars,
                    filler: &filler,
                    flow: &flow.id,
                    side,
                    session_id: id,
                    rng: session_rng(SEED, id, side),
                    functions,
                    limits: Limits::default(),
                };
                let mut client = Session::new(make(Side::Client, &mut client_fns));
                let mut server_session = Session::new(make(Side::Server, &mut server_fns));
                let server_program = flow.server.as_ref().unwrap();
                let (c2s, s2c): (Sent, Sent) = (Arc::default(), Arc::default());
                let both = async {
                    match flow.l4 {
                        L4::Udp => {
                            let mut co = UdpClient {
                                connector: &connector,
                                source,
                                server,
                                sent: Arc::clone(&c2s),
                            };
                            let mut so = UdpServer {
                                acceptor: match (&mut udp_listener, &mut kernel) {
                                    (Some(l), _) => UdpAcceptor::Datapath(l),
                                    (None, Some(k)) => UdpAcceptor::Kernel(&mut k.udp),
                                    (None, None) => unreachable!("a server side exists"),
                                },
                                sent: Arc::clone(&s2c),
                            };
                            tokio::join!(
                                client.run_datagram_program(
                                    &mut co,
                                    &flow.client,
                                    std::future::pending()
                                ),
                                server_session.run_datagram_program(
                                    &mut so,
                                    server_program,
                                    std::future::pending()
                                ),
                            )
                        }
                        _ => {
                            let mut co = TcpClient {
                                connector: &connector,
                                source,
                                server,
                            };
                            let mut so = TcpServer {
                                acceptor: match (&mut tcp_listener, &kernel) {
                                    (Some(l), _) => TcpAcceptor::Datapath(l),
                                    (None, Some(k)) => TcpAcceptor::Kernel(&k.tcp),
                                    (None, None) => unreachable!("a server side exists"),
                                },
                            };
                            tokio::join!(
                                client.run_program(&mut co, &flow.client, std::future::pending()),
                                server_session.run_program(
                                    &mut so,
                                    server_program,
                                    std::future::pending()
                                ),
                            )
                        }
                    }
                };
                let Ok((c, sv)) = tokio::time::timeout(Duration::from_secs(20), both).await else {
                    out.failures.push(format!("{tag}: timed out"));
                    // A flow that hangs leaves a session behind, so stop.
                    return out;
                };
                if let Err(f) = &c {
                    out.failures.push(format!("{tag}: client {f:?}"));
                }
                if let Err(f) = &sv {
                    out.failures.push(format!("{tag}: server {f:?}"));
                }
                if c.is_err() || sv.is_err() {
                    continue;
                }
                let (cs, ss) = (client.stats(), server_session.stats());
                let mut bad = Vec::new();
                if cs.bytes_tx != ss.bytes_rx || cs.bytes_rx != ss.bytes_tx {
                    bad.push(format!("{tag}: the sides disagree on the bytes moved"));
                }
                if text(client.overlay(), "server_ip").as_deref() != want.server_ip {
                    bad.push(format!("{tag}: server_ip variable"));
                }
                match want.l4 {
                    L4::Udp => {
                        out.udp_flows += 1;
                        let (q, a) = (c2s.lock().unwrap(), s2c.lock().unwrap());
                        bad.extend(check_udp(&tag, &q, &a, want));
                    }
                    _ => {
                        out.tcp_flows += 1;
                        // A length prefix and a response that needs TCP.
                        if cs.bytes_rx <= 514 {
                            bad.push(format!("{tag}: the TCP response is too short"));
                        }
                    }
                }
                out.failures.extend(bad);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The in memory test
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn dns_scenarios_complete_over_the_datapath_in_memory() {
    in_memory(Pools::counts(4, 4, None).unwrap()).await;
}

/// The same scenarios with IPv6 pools: every flow runs over IPv6.
#[tokio::test(flavor = "current_thread")]
async fn dns_scenarios_complete_over_ipv6_in_memory() {
    in_memory(Pools::sized_v6("fd00:1::/64", 4, "fd00:2::/64", 4, None).unwrap()).await;
}

async fn in_memory(pools: Pools) {
    let (client, server) = frame_pair(512, 1600);
    let lane = Lane {
        client: Box::new(client),
        server: Box::new(server),
        pools: split_pools(&pools, 1).unwrap().remove(0),
    };
    let dp = Datapath::start(vec![lane], DatapathConfig::default()).unwrap();
    let reps = 3;
    let out = run_dns(&dp, &pools, reps, None).await;
    assert!(out.failures.is_empty(), "{:#?}", out.failures);
    assert_eq!((out.udp_flows, out.tcp_flows), (7 * reps, reps));
    let lane = dp.stats().remove(0);
    let (c, s) = (&lane.udp_client, &lane.udp_server);
    // One query and one response per UDP flow, and nothing lost or refused.
    assert_eq!((c.tx_datagrams, s.tx_datagrams), (7 * reps, 7 * reps));
    assert_eq!((c.rx_delivered, s.rx_delivered), (7 * reps, 7 * reps));
    assert_eq!(s.rx_accepted, 7 * reps);
    for u in [c, s] {
        assert_eq!(
            u.rx_bad_ip_checksum
                + u.rx_bad_udp_checksum
                + u.rx_malformed
                + u.rx_fragments
                + u.rx_no_listener
                + u.rx_queue_full
                + u.rx_not_ours,
            0,
            "{u:?}"
        );
        assert_eq!(u.sessions_open, 0, "every session closed: {u:?}");
    }
    assert!(lane.client.tcp > 0 && lane.server.tcp > 0, "the TCP flow");
    assert_eq!(lane.client.udp_unsupported + lane.server.udp_unsupported, 0);
    assert_eq!(lane.client.udp_handled, 7 * reps);
    assert_eq!(lane.server.udp_handled, 7 * reps);
    assert!(dp.stop().iter().all(|r| !r.panicked));
}

// ---------------------------------------------------------------------------
// The veth tests, which re-run this binary in a namespace
// ---------------------------------------------------------------------------

fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("{cmd}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "{cmd} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

fn required() -> bool {
    std::env::var("CI").is_ok_and(|v| v == "true")
        || std::env::var("FPS_REQUIRE_AFXDP").is_ok_and(|v| v == "1")
}

fn skip(why: &str) {
    assert!(
        !required(),
        "the dns AF_XDP test cannot run here and CI requires it: {why}"
    );
    eprintln!("skipping the dns AF_XDP test: {why}");
}

/// Whether the kernel can create an AF_XDP socket at all.
fn kernel_has_af_xdp() -> bool {
    const AF_XDP: libc::c_int = 44;
    // SAFETY: `socket` takes no pointers, and the descriptor that it returns
    // is closed at once.
    let fd = unsafe { libc::socket(AF_XDP, libc::SOCK_RAW, 0) };
    if fd >= 0 {
        // SAFETY: `fd` is the descriptor from above, which nothing else owns.
        unsafe { libc::close(fd) };
    }
    fd >= 0 || io::Error::last_os_error().raw_os_error() != Some(libc::EAFNOSUPPORT)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// The roles of the helper: `run` and `kernel` over IPv4, `run6` and `kernel6`
/// over IPv6. The `kernel` roles give `dnsb` to the kernel.
fn is_kernel(role: &str) -> bool {
    role.starts_with("kernel")
}

fn is_v6(role: &str) -> bool {
    role.ends_with('6')
}

/// The pools of a role: the kernel server roles have two client hosts and one
/// server host, the others four and four.
fn pools_for(role: &str) -> Pools {
    match role {
        "kernel" => Pools::counts(2, 1, None),
        "kernel6" => Pools::sized_v6("fd00:1::/64", 2, "fd00:2::/64", 1, None),
        "run6" => Pools::sized_v6("fd00:1::/64", 4, "fd00:2::/64", 4, None),
        "loss6" => Pools::sized_v6("fd00:1::/64", 2, "fd00:2::/64", 2, None),
        "loss" => Pools::counts(2, 2, None),
        _ => Pools::counts(4, 4, None),
    }
    .unwrap()
}

/// A namespace with a veth pair, `dnsa` and `dnsb`, which `Drop` removes
/// again. With a `kernel` role the interface `dnsb` is set up as the kernel
/// host of the first server address: the MAC address of that host, its
/// address, and a route to the client prefix. The IPv6 address has no
/// duplicate address detection (`nodad`), so a socket can bind it at once,
/// and the kernel resolves a client host with a neighbor solicitation that the
/// splitter answers.
struct Namespace(String);

impl Namespace {
    fn up(name: &str, role: &str) -> Result<Self, String> {
        run("ip", &["netns", "add", name])?;
        let ns = Self(name.to_owned());
        run(
            "ip",
            &[
                "-n", name, "link", "add", "dnsa", "type", "veth", "peer", "name", "dnsb",
            ],
        )?;
        if is_kernel(role) {
            let pools = pools_for(role);
            let server = pools.server.ip(0).unwrap();
            let station = pools.stations().unwrap().server;
            let mac = station.local_mac(server).unwrap().0;
            let mac = mac.map(|b| format!("{b:02x}")).join(":");
            let (prefix, route, nodad) = match (server, pools.client.ip(0).unwrap()) {
                (IpAddr::V4(server), IpAddr::V4(client)) => (
                    format!("{server}/30"),
                    format!("{}/30", Ipv4Addr::from(u32::from(client) & !3)),
                    None,
                ),
                _ => (
                    format!("{server}/64"),
                    "fd00:1::/64".to_owned(),
                    Some("nodad"),
                ),
            };
            run("ip", &["-n", name, "link", "set", "dnsb", "address", &mac])?;
            let mut add = vec!["-n", name, "addr", "add", &prefix, "dev", "dnsb"];
            add.extend(nodad);
            run("ip", &add)?;
            run("ip", &["-n", name, "link", "set", "dnsb", "up"])?;
            run("ip", &["-n", name, "link", "set", "dnsa", "up"])?;
            run("ip", &["-n", name, "route", "add", &route, "dev", "dnsb"])?;
        } else {
            for dev in ["dnsa", "dnsb"] {
                run("ip", &["-n", name, "link", "set", dev, "up"])?;
            }
        }
        Ok(ns)
    }
}

impl Drop for Namespace {
    fn drop(&mut self) {
        let _ = run("ip", &["netns", "del", &self.0]);
    }
}

fn field<'a>(line: &'a str, key: &str) -> &'a str {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
        .unwrap_or_else(|| panic!("no field {key} in {line:?}"))
}

fn number(line: &str, key: &str) -> u64 {
    field(line, key)
        .parse()
        .unwrap_or_else(|_| panic!("field {key} of {line:?} is not a number"))
}

/// Runs the helper `role` of this binary in a fresh namespace and returns the
/// lines that it reported. Returns `None` when the test skipped, or when it
/// ran again under `sudo` (that run did the asserting).
fn in_namespace(test: &str, role: &str) -> Option<Vec<String>> {
    let root = run("id", &["-u"]).is_ok_and(|s| s.trim() == "0");
    if !root {
        if run("sudo", &["-n", "true"]).is_err() {
            skip("not root and no passwordless sudo");
            return None;
        }
        let exe = std::env::current_exe().unwrap();
        let mut cmd = Command::new("sudo");
        cmd.args(["-n", "env"]);
        for key in [
            "CI",
            "FPS_REQUIRE_AFXDP",
            "FPS_DNS_REPS",
            "FPS_DNS_RESULTS",
            "FPS_LOSS_PPM",
            "FPS_LOSS_KIB",
        ] {
            if let Ok(v) = std::env::var(key) {
                cmd.arg(format!("{key}={v}"));
            }
        }
        let status = cmd
            .arg(exe)
            .args(["--exact", test, "--nocapture"])
            .status()
            .unwrap();
        assert!(status.success(), "the test failed under sudo");
        return None;
    }
    if let Err(e) = run("ip", &["-V"]) {
        skip(&e);
        return None;
    }
    if !kernel_has_af_xdp() {
        skip("the kernel has no AF_XDP sockets");
        return None;
    }
    let name = format!("dns{role}{}", std::process::id() % 100_000);
    let ns = match Namespace::up(&name, role) {
        Ok(ns) => ns,
        Err(e) => {
            skip(&format!("the namespace did not come up: {e}"));
            return None;
        }
    };
    let exe = std::env::current_exe().unwrap();
    let mut child = Command::new("ip")
        .args(["netns", "exec", &ns.0])
        .arg(exe)
        .args(["--ignored", "--exact", "helper_dns", "--nocapture"])
        .env("FPS_DNS_ROLE", role)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let lines: Vec<String> = BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| l.strip_prefix(MARK).map(str::to_owned))
        .collect();
    let status = child.wait().unwrap();
    for l in &lines {
        eprintln!("{MARK}{l}");
    }
    assert!(status.success(), "the helper failed: {lines:?}");
    Some(lines)
}

fn find<'a>(lines: &'a [String], start: &str) -> &'a str {
    lines
        .iter()
        .find(|l| l.starts_with(start))
        .unwrap_or_else(|| panic!("no {start} line in {lines:?}"))
}

fn save(lines: &[String], file: &str) {
    if let Ok(dir) = std::env::var("FPS_DNS_RESULTS") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut text = lines.join("\n");
        text.push('\n');
        std::fs::write(dir.join(file), text).unwrap();
    }
}

#[test]
fn dns_scenarios_complete_over_af_xdp_on_a_veth_pair() {
    veth_run("dns_scenarios_complete_over_af_xdp_on_a_veth_pair", "run");
}

/// The same run over IPv6: the pools are IPv6 prefixes, so every flow has IPv6
/// addresses, the UDP checksum is the one of RFC 8200, and the stacks resolve
/// every neighbor from the seeded tables.
#[test]
fn dns_scenarios_complete_over_ipv6_on_a_veth_pair() {
    veth_run("dns_scenarios_complete_over_ipv6_on_a_veth_pair", "run6");
}

fn veth_run(test: &str, role: &str) {
    let Some(lines) = in_namespace(test, role) else {
        return;
    };
    let reps = env_u64("FPS_DNS_REPS", 5);
    let result = find(&lines, "result ");
    assert_eq!(number(result, "failed"), 0, "{result}");
    assert_eq!(number(result, "udp_flows"), 7 * reps, "{result}");
    assert_eq!(number(result, "tcp_flows"), reps, "{result}");
    let udp = find(&lines, "udp ");
    for key in [
        "client_tx",
        "server_tx",
        "client_rx",
        "server_rx",
        "accepted",
    ] {
        assert_eq!(number(udp, key), 7 * reps, "{key}: {udp}");
    }
    for key in [
        "bad_checksum",
        "malformed",
        "fragments",
        "no_listener",
        "queue_full",
        "not_ours",
    ] {
        assert_eq!(number(udp, key), 0, "{key}: {udp}");
    }
    let frames = find(&lines, "frames ");
    assert!(number(frames, "client_tcp") > 0 && number(frames, "server_tcp") > 0);
    assert_eq!(number(frames, "udp_unsupported"), 0, "{frames}");
    assert_eq!(number(frames, "panicked"), 0, "{frames}");
    let file = if is_v6(role) {
        "dns-veth6.txt"
    } else {
        "dns-veth.txt"
    };
    save(&lines, file);
}

#[test]
fn dns_client_over_af_xdp_completes_against_a_kernel_server() {
    kernel_run(
        "dns_client_over_af_xdp_completes_against_a_kernel_server",
        "kernel",
    );
}

/// The kernel server over IPv6: the kernel verifies the IPv6 pseudo-header
/// checksums of the user space layers and resolves the client hosts with
/// neighbor discovery, which the splitter answers.
#[test]
fn dns_client_over_af_xdp_completes_against_a_kernel_server_over_ipv6() {
    kernel_run(
        "dns_client_over_af_xdp_completes_against_a_kernel_server_over_ipv6",
        "kernel6",
    );
}

fn kernel_run(test: &str, role: &str) {
    let Some(lines) = in_namespace(test, role) else {
        return;
    };
    let reps = env_u64("FPS_DNS_REPS", 5);
    let result = find(&lines, "result ");
    assert_eq!(number(result, "failed"), 0, "{result}");
    assert_eq!(number(result, "udp_flows"), 7 * reps, "{result}");
    assert_eq!(number(result, "tcp_flows"), reps, "{result}");
    let udp = find(&lines, "udp ");
    // The client arm sent one query and received one response per flow. The
    // server arm is idle: the kernel is the server.
    assert_eq!(number(udp, "client_tx"), 7 * reps, "{udp}");
    assert_eq!(number(udp, "client_rx"), 7 * reps, "{udp}");
    assert_eq!(
        number(udp, "server_tx") + number(udp, "server_rx"),
        0,
        "{udp}"
    );
    for key in [
        "bad_checksum",
        "malformed",
        "fragments",
        "no_listener",
        "queue_full",
        "not_ours",
    ] {
        assert_eq!(number(udp, key), 0, "{key}: {udp}");
    }
    let frames = find(&lines, "frames ");
    assert!(number(frames, "client_tcp") > 0, "{frames}");
    let (answered, how) = if is_v6(role) {
        (number(frames, "client_nd_replied"), "neighbor discovery")
    } else {
        (number(frames, "client_arp_replied"), "ARP")
    };
    assert!(
        answered > 0,
        "the kernel resolved a client host with {how}: {frames}"
    );
    assert_eq!(number(frames, "panicked"), 0, "{frames}");
    let file = if is_v6(role) {
        "dns-veth-kernel6.txt"
    } else {
        "dns-veth-kernel.txt"
    };
    save(&lines, file);
}

fn say(msg: &str) {
    println!("{MARK}{msg}");
    let _ = std::io::stdout().flush();
}

fn mode_name(m: BoundMode) -> &'static str {
    match m {
        BoundMode::ZeroCopy => "zerocopy",
        BoundMode::Copy => "copy",
    }
}

fn open(interface: &str) -> OpenedPort {
    let config = PortConfig {
        interface: interface.to_owned(),
        queues: vec![0],
        queue: QueueConfig {
            mode: BindMode::Auto,
            ..QueueConfig::default()
        },
    };
    // The kernel releases the queue of a closed socket in a work queue, so a
    // bind right after a close can meet EBUSY for a few milliseconds.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match open_port(&config) {
            Ok(p) => return p,
            Err(e) if e.raw_os_error() == Some(16) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("opening {interface}: {e:?}"),
        }
    }
}

/// Turns the transmit checksum offload of `dev` off with the ethtool ioctl
/// (`ETHTOOL_STXCSUM`), because the veth hands a frame with a partial checksum
/// to an AF_XDP end and the hosted runner has no `ethtool` command. The
/// helper runs in the namespace of the interface.
fn tx_checksum_off(dev: &str) -> io::Result<()> {
    #[repr(C)]
    struct EthtoolValue {
        cmd: u32,
        data: u32,
    }
    const SIOCETHTOOL: libc::Ioctl = 0x8946;
    const ETHTOOL_STXCSUM: u32 = 0x17;
    // SAFETY: `socket` takes no pointers.
    let sock = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if sock < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut value = EthtoolValue {
        cmd: ETHTOOL_STXCSUM,
        data: 0,
    };
    // SAFETY: `ifreq` is plain data for which all zero bytes are valid.
    let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
    for (slot, byte) in request.ifr_name.iter_mut().zip(dev.bytes().take(15)) {
        *slot = byte as libc::c_char;
    }
    request.ifr_ifru.ifru_data = std::ptr::addr_of_mut!(value).cast();
    // SAFETY: `request` points at `value`, and both outlive the call, which
    // reads `cmd` and `data` of the structure that the request names.
    let result = unsafe { libc::ioctl(sock, SIOCETHTOOL, &mut request) };
    let error = io::Error::last_os_error();
    // SAFETY: `sock` is the descriptor from above, which nothing else owns.
    unsafe { libc::close(sock) };
    if result < 0 {
        Err(error)
    } else {
        Ok(())
    }
}

fn report(lane: &LaneStats) -> [String; 2] {
    let (c, s) = (&lane.udp_client, &lane.udp_server);
    let bad = |u: &fps_datapath::UdpStats| u.rx_bad_ip_checksum + u.rx_bad_udp_checksum;
    [
        format!(
            "udp client_tx={} server_tx={} client_rx={} server_rx={} accepted={} \
             bad_checksum={} malformed={} fragments={} no_listener={} queue_full={} not_ours={} \
             client_tx_blocked={}",
            c.tx_datagrams,
            s.tx_datagrams,
            c.rx_delivered,
            s.rx_delivered,
            s.rx_accepted,
            bad(c) + bad(s),
            c.rx_malformed + s.rx_malformed,
            c.rx_fragments + s.rx_fragments,
            c.rx_no_listener + s.rx_no_listener,
            c.rx_queue_full + s.rx_queue_full + c.rx_accept_full + s.rx_accept_full,
            c.rx_not_ours + s.rx_not_ours,
            c.tx_blocked,
        ),
        format!(
            "frames stack_tx={} client_tcp={} server_tcp={} client_udp={} server_udp={} \
             udp_unsupported={} client_arp_replied={} client_nd_replied={} loop_passes={}",
            lane.stack.frames,
            lane.client.tcp,
            lane.server.tcp,
            lane.client.udp_handled,
            lane.server.udp_handled,
            lane.client.udp_unsupported + lane.server.udp_unsupported,
            lane.client.arp_replied,
            lane.client.nd_replied,
            lane.loop_passes,
        ),
    ]
}

// ---------------------------------------------------------------------------
// The loss workload (TRN-22) over the datapath
// ---------------------------------------------------------------------------

/// A queue that loses frames at random: every frame that the inner queue
/// receives is dropped with the probability `loss_ppm` per million, from a
/// generator with a fixed seed. A veth end that an AF_XDP socket owns sends
/// around the queueing disciplines of the kernel, so `netem` cannot impair it
/// (TRN-26), and the loss sits in the frame queue instead, like the impaired
/// link of `fps-tcp` (TRN-22).
struct LossyQueue {
    inner: Box<dyn FrameIo>,
    loss_ppm: u32,
    state: u64,
    dropped: Arc<AtomicU64>,
}

impl LossyQueue {
    fn new(inner: Box<dyn FrameIo>, loss_ppm: u32, seed: u64, dropped: Arc<AtomicU64>) -> Self {
        Self {
            inner,
            loss_ppm,
            state: seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1,
            dropped,
        }
    }
}

/// One step of xorshift64.
fn next_random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

impl FrameIo for LossyQueue {
    fn queue_id(&self) -> u32 {
        self.inner.queue_id()
    }

    fn max_frame_len(&self) -> usize {
        self.inner.max_frame_len()
    }

    fn recv_batch(&mut self, max: usize, frame: &mut dyn FnMut(&[u8])) -> io::Result<usize> {
        let (ppm, state, dropped) = (self.loss_ppm, &mut self.state, &self.dropped);
        let mut kept = 0;
        self.inner.recv_batch(max, &mut |f| {
            if next_random(state) % 1_000_000 < u64::from(ppm) {
                dropped.fetch_add(1, Ordering::Relaxed);
            } else {
                kept += 1;
                frame(f);
            }
        })?;
        Ok(kept)
    }

    fn send_batch_with(
        &mut self,
        count: usize,
        fill: &mut dyn FnMut(usize, &mut [u8]) -> usize,
    ) -> io::Result<usize> {
        self.inner.send_batch_with(count, fill)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }

    fn wait_readable(&mut self, timeout: Duration) -> io::Result<bool> {
        self.inner.wait_readable(timeout)
    }

    fn stats(&self) -> QueueStats {
        self.inner.stats()
    }
}

/// The datapath as a `TcpStack` of the bench harness, so that the loss
/// workload of `fps-tcp` runs on it unchanged: a client host and a server
/// host of the pools, and the lane between them.
#[derive(Clone)]
struct DatapathStack {
    datapath: Arc<Datapath>,
    connector: Connector,
    client: IpAddr,
    server: IpAddr,
    listeners: Arc<Mutex<HashMap<u16, Arc<tokio::sync::Mutex<fps_datapath::Listener>>>>>,
    dropped: Arc<AtomicU64>,
}

impl TcpStack for DatapathStack {
    type Session = DatapathSession;

    fn name(&self) -> String {
        "user space datapath (fps-datapath)".to_owned()
    }

    fn notes(&self) -> Vec<String> {
        vec![
            "the frames of both arms cross a lossy queue of the test, which drops received frames at random".to_owned(),
        ]
    }

    fn listen(&self, port: u16) -> io::Result<()> {
        let listener = self.datapath.listen(port).map_err(io::Error::other)?;
        self.listeners
            .lock()
            .map_err(|_| io::Error::other("poisoned"))?
            .insert(port, Arc::new(tokio::sync::Mutex::new(listener)));
        Ok(())
    }

    fn connect(&self, port: u16) -> impl Future<Output = io::Result<DatapathSession>> + Send {
        let connector = self.connector.clone();
        let destination = SocketAddr::new(self.server, port);
        let source = self.client;
        async move {
            connector
                .connect(source, destination)
                .await
                .map_err(io::Error::other)
        }
    }

    fn accept(&self, port: u16) -> impl Future<Output = io::Result<DatapathSession>> + Send {
        let listener = self
            .listeners
            .lock()
            .ok()
            .and_then(|m| m.get(&port).cloned());
        async move {
            let listener = listener.ok_or_else(|| io::Error::other("port not listened on"))?;
            let mut listener = listener.lock().await;
            listener.accept().await.map_err(io::Error::other)
        }
    }

    fn drive(&self) -> impl Future<Output = ()> + Send {
        // The threads of the lanes drive the stack.
        std::future::pending()
    }

    fn poll_once(&self) {}

    fn stats(&self) -> StackStats {
        StackStats {
            link_lost: self.dropped.load(Ordering::Relaxed),
            ..StackStats::default()
        }
    }
}

/// What a loss run showed.
struct LossResult {
    run: LossRun,
    dropped: u64,
    bytes: u64,
    tcp_frames: u64,
}

/// Runs the loss workload over a lane whose two arms lose `loss_ppm` frames
/// per million, between the first client host and the first server host.
async fn loss_over(
    client: Box<dyn FrameIo>,
    server: Box<dyn FrameIo>,
    pools: &Pools,
    loss_ppm: u32,
    kib: u64,
) -> LossResult {
    let dropped = Arc::new(AtomicU64::new(0));
    let arm = |q, seed| -> Box<dyn FrameIo> {
        Box::new(LossyQueue::new(q, loss_ppm, seed, Arc::clone(&dropped)))
    };
    let lane = Lane {
        client: arm(client, 1),
        server: arm(server, 2),
        pools: split_pools(pools, 1).unwrap().remove(0),
    };
    let datapath = Arc::new(Datapath::start(vec![lane], DatapathConfig::default()).unwrap());
    let stack = DatapathStack {
        connector: datapath.connector(),
        client: pools.client.ip(0).unwrap(),
        server: pools.server.ip(0).unwrap(),
        datapath: Arc::clone(&datapath),
        listeners: Arc::default(),
        dropped: Arc::clone(&dropped),
    };
    let bytes = kib * 1024;
    let config = LossConfig {
        bytes,
        timeout: Duration::from_secs(40),
        ..LossConfig::default()
    };
    let run = loss_transfer(&stack, &config).await;
    let lane = datapath.stats().remove(0);
    LossResult {
        run,
        dropped: dropped.load(Ordering::Relaxed),
        bytes,
        tcp_frames: lane.client.tcp + lane.server.tcp,
    }
}

fn assert_loss_run(r: &LossResult) {
    assert!(r.run.completed, "{:?}", r.run.failure);
    assert_eq!(r.run.confirmed_bytes, r.bytes);
    assert!(r.dropped > 0, "the lossy queues dropped nothing");
    assert_eq!(r.run.link_lost, r.dropped);
    // The bytes arrived although frames were lost, so the stack sent segments
    // again: the frames that reached the stacks are more than the data needs.
    let segments = r.bytes / 1400;
    assert!(
        r.tcp_frames > segments,
        "{} frames for {segments} segments",
        r.tcp_frames
    );
}

#[tokio::test(flavor = "current_thread")]
async fn loss_workload_completes_over_ipv6_in_memory() {
    let pools = Pools::sized_v6("fd00:1::/64", 2, "fd00:2::/64", 2, None).unwrap();
    let (client, server) = frame_pair(4096, 1600);
    let r = loss_over(Box::new(client), Box::new(server), &pools, 20_000, 192).await;
    assert_loss_run(&r);
}

#[tokio::test(flavor = "current_thread")]
async fn loss_workload_completes_over_ipv4_in_memory() {
    let pools = Pools::counts(2, 2, None).unwrap();
    let (client, server) = frame_pair(4096, 1600);
    let r = loss_over(Box::new(client), Box::new(server), &pools, 20_000, 192).await;
    assert_loss_run(&r);
}

#[test]
fn loss_workload_completes_over_ipv6_on_a_veth_pair() {
    veth_loss("loss_workload_completes_over_ipv6_on_a_veth_pair", "loss6");
}

#[test]
fn loss_workload_completes_over_ipv4_on_a_veth_pair() {
    veth_loss("loss_workload_completes_over_ipv4_on_a_veth_pair", "loss");
}

fn veth_loss(test: &str, role: &str) {
    let Some(lines) = in_namespace(test, role) else {
        return;
    };
    let result = find(&lines, "loss ");
    assert_eq!(number(result, "completed"), 1, "{result}");
    assert_eq!(
        number(result, "confirmed"),
        number(result, "bytes"),
        "{result}"
    );
    assert!(number(result, "dropped") > 0, "{result}");
    assert_eq!(
        number(result, "lost"),
        number(result, "dropped"),
        "{result}"
    );
    assert!(
        number(result, "tcp_frames") > number(result, "bytes") / 1400,
        "{result}"
    );
    let file = if is_v6(role) {
        "loss-veth6.txt"
    } else {
        "loss-veth.txt"
    };
    save(&lines, file);
}

fn helper_loss(role: &str) {
    let mut a = open("dnsa");
    let mut b = open("dnsb");
    let modes = format!(
        "client={} server={}",
        mode_name(a.info.queues[0].bound),
        mode_name(b.info.queues[0].bound)
    );
    let ppm = u32::try_from(env_u64("FPS_LOSS_PPM", 20_000)).unwrap();
    let kib = env_u64("FPS_LOSS_KIB", 512);
    let pools = pools_for(role);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let r = runtime.block_on(loss_over(
        Box::new(a.queues.remove(0)),
        Box::new(b.queues.remove(0)),
        &pools,
        ppm,
        kib,
    ));
    if let Some(f) = &r.run.failure {
        eprintln!("{MARK}failure {f}");
    }
    say(&format!(
        "loss completed={} bytes={} confirmed={} elapsed_ms={} dropped={} lost={} tcp_frames={} loss_ppm={ppm} {modes}",
        u8::from(r.run.completed),
        r.bytes,
        r.run.confirmed_bytes,
        r.run.elapsed.as_millis(),
        r.dropped,
        r.run.link_lost,
        r.tcp_frames,
    ));
}

#[test]
#[ignore = "started by the dns veth tests inside a namespace"]
fn helper_dns() {
    let Ok(role) = std::env::var("FPS_DNS_ROLE") else {
        return;
    };
    if role.starts_with("loss") {
        helper_loss(&role);
        return;
    }
    let kernel_server = is_kernel(&role);
    let reps = env_u64("FPS_DNS_REPS", 5);
    let mut a = open("dnsa");
    let (pools, lane, modes, _spare) = if kernel_server {
        // The kernel owns `dnsb`. The server arm of the lane is the end of an
        // in memory pair that nobody writes to.
        tx_checksum_off("dnsb").expect("the transmit offload of dnsb turns off");
        let (spare_arm, spare) = frame_pair(8, 1600);
        let pools = pools_for(&role);
        let modes = format!("client={} server=kernel", mode_name(a.info.queues[0].bound));
        let lane = Lane {
            client: Box::new(a.queues.remove(0)),
            server: Box::new(spare_arm),
            pools: split_pools(&pools, 1).unwrap().remove(0),
        };
        (pools, lane, modes, Some(spare))
    } else {
        let mut b = open("dnsb");
        let modes = format!(
            "client={} server={}",
            mode_name(a.info.queues[0].bound),
            mode_name(b.info.queues[0].bound)
        );
        let pools = pools_for(&role);
        let lane = Lane {
            client: Box::new(a.queues.remove(0)),
            server: Box::new(b.queues.remove(0)),
            pools: split_pools(&pools, 1).unwrap().remove(0),
        };
        (pools, lane, modes, None)
    };
    let dp = Datapath::start(vec![lane], DatapathConfig::default()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let started = Instant::now();
    let out = {
        let _guard = runtime.enter();
        let kernel = kernel_server.then(|| {
            let address = SocketAddr::new(pools.server.ip(0).unwrap(), 53);
            KernelServer {
                udp: KernelDatagramListener::bind(address, &SocketOptions::default()).unwrap(),
                tcp: KernelListener::bind(address, &ListenConfig::default()).unwrap(),
            }
        });
        runtime.block_on(run_dns(&dp, &pools, reps, kernel))
    };
    let elapsed = started.elapsed();
    for f in &out.failures {
        eprintln!("{MARK}failure {f}");
    }
    say(&format!(
        "result reps={reps} udp_flows={} tcp_flows={} failed={} elapsed_ms={} {modes}",
        out.udp_flows,
        out.tcp_flows,
        out.failures.len(),
        elapsed.as_millis()
    ));
    let lane = dp.stats().remove(0);
    let reports = dp.stop();
    let [udp, frames] = report(&lane);
    say(&udp);
    say(&format!(
        "{frames} panicked={}",
        u8::from(reports[0].panicked)
    ));
}
