// SPDX-License-Identifier: GPL-3.0-only
//! Multi flow scenario instances on a worker (ENG-23).
//!
//! A scenario with more than one flow runs as one [`fps_interp::ScenarioInstance`]
//! per session start. The worker drives it with a [`FlowEnv`] over the kernel
//! backend, which hands each flow side its opener and its module functions and
//! keeps what the worker needs to count the side: its session id, its start,
//! its end, and its extension instances. A scenario with one flow never comes
//! here, it keeps the single session path of the worker.
//!
//! # Session ids
//!
//! The instance gives the n-th flow side that it starts the id `first + n`,
//! because a worker runs one side of each flow and so no flow session has two
//! sides in the instance. The environment counts its own starts to learn the
//! id that the instance gives, which the capture tap and the extensions need
//! when they open, and [`Run::finish`] checks the counts against the records
//! of the instance. The first id of an instance is the worker id and the
//! instance number shifted by [`ID_BLOCK_SHIFT`], so an instance owns a block
//! of 65 536 ids and its generators never meet those of another instance.
//!
//! # Openers
//!
//! The entry flow of a client opens as the single session path does,
//! including the port override of the worker. The entry flow of a server takes
//! the connection that the listener of the worker accepted. Every other flow
//! opens as the peer of its data port needs it:
//!
//! - A client flow dials the server of its session, or the host that the
//!   `connect` renders, on the rendered port. It retries a refused
//!   connection for [`OPEN_GRACE`], because the server side of a spawned flow
//!   may not have bound the port yet.
//! - A server flow binds the rendered port on the address that accepted the
//!   entry connection, accepts one connection, and releases the port. It
//!   retries a bind that finds the port in use for [`OPEN_GRACE`], because
//!   another instance may still hold the port.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use fps_backend_kernel::{KernelDatagram, KernelListener, KernelStream, ListenConfig};
use fps_capture::CaptureCell;
use fps_flow::program::{Flow, L4};
use fps_flow::{FnCall, FnError, Scenario, TemplateFn};
use fps_interp::{
    DatagramOpener, FlowEnv, InstanceConfig, OpenKind, OpenRequest, Opener, Reason,
    ScenarioInstance, Side, Sides, TlsUpgrader,
};
use rand::RngCore;
use tokio::time::{sleep, Instant};

use crate::extension::close_reason_of;
use crate::opener::{destination, global_session_id, invalid, source_address, TapOpener};
use crate::tls::SessionTls;
use crate::worker::{ClientCtx, Worker};
use crate::{SessionExtensions, SessionKey, Transport};

/// The number of low bits of a local session number that an instance keeps
/// for the ids of its own flow sessions.
pub(crate) const ID_BLOCK_SHIFT: u32 = 16;

/// How long an open of a non entry flow retries a connect that the peer
/// refuses, or a bind that finds the port in use. It stays below the default
/// open timeout of the interpreter, so a peer that never comes still fails
/// with the reason of the interpreter.
pub(crate) const OPEN_GRACE: Duration = Duration::from_secs(2);

const RETRY_FIRST: Duration = Duration::from_millis(2);
const RETRY_MAX: Duration = Duration::from_millis(50);

/// The end that a worker task owns of the connection of an instance.
pub(crate) enum Peer {
    /// A client session start, which dials.
    Client(Rc<ClientCtx>),
    /// A server session, which the listener of the worker accepted.
    Server(KernelStream),
}

/// What the environment keeps of one flow side that the instance started.
struct SideLog {
    id: u64,
    flow: usize,
    side: Side,
    started: Instant,
    /// Set when the opener of the side drops, which is when the side ends.
    ended: Rc<Cell<Option<Instant>>>,
    /// The extension instances of the side, when the flow has extensions.
    extensions: Option<Rc<RefCell<Option<SessionExtensions>>>>,
}

/// How the environment opens the flows of its side.
enum Net {
    Client {
        ctx: Rc<ClientCtx>,
        source: Result<IpAddr, String>,
        session: u64,
    },
    Server {
        /// The accepted connection, until the entry flow takes it.
        accepted: Option<KernelStream>,
        /// The address that data ports bind on.
        bind_ip: IpAddr,
        config: ListenConfig,
    },
}

/// The [`FlowEnv`] of one instance on a worker.
pub(crate) struct InstanceEnv {
    worker: Rc<Worker>,
    scenario: Arc<Scenario>,
    entry: usize,
    net: Net,
    first_id: u64,
    log: Vec<SideLog>,
    /// The upgrader that every TCP flow side of the instance shares, which
    /// collects the handshakes of the instance for the metrics.
    tls: Option<SessionTls>,
}

/// Opens one flow side.
pub(crate) struct FlowOpener {
    kind: FlowNet,
    ended: Rc<Cell<Option<Instant>>>,
}

enum FlowNet {
    /// Dials the server. A flow other than the entry flow retries a refused
    /// connection.
    Connect {
        ctx: Rc<ClientCtx>,
        source: Result<IpAddr, String>,
        session: u64,
        entry: bool,
    },
    /// Hands out the connection that the worker accepted.
    Accepted(Option<KernelStream>),
    /// Binds the rendered port and accepts one connection.
    Bind { ip: IpAddr, config: ListenConfig },
}

impl Drop for FlowOpener {
    fn drop(&mut self) {
        if self.ended.get().is_none() {
            self.ended.set(Some(Instant::now()));
        }
    }
}

/// What one `open` does, decided before the future starts.
enum Plan {
    Connect {
        ctx: Rc<ClientCtx>,
        source: IpAddr,
        to: SocketAddr,
        retry: bool,
    },
    Accepted(Option<KernelStream>),
    Bind(SocketAddr, ListenConfig),
    Fail(io::Error),
}

impl Opener for FlowOpener {
    type Io = KernelStream;

    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<KernelStream>> {
        let plan = match (&mut self.kind, request.kind) {
            (
                FlowNet::Connect {
                    ctx,
                    source,
                    session,
                    entry,
                },
                OpenKind::Connect,
            ) => {
                // The override replaces the port of the entry connect only,
                // because a data connection goes to the port that its
                // flow renders.
                let port = if *entry { ctx.config.port } else { None };
                match (
                    source.clone().map_err(invalid),
                    destination(&ctx.config.servers, *session, port, &request),
                ) {
                    (Ok(source), Ok(to)) => Plan::Connect {
                        ctx: Rc::clone(ctx),
                        source,
                        to,
                        retry: !*entry,
                    },
                    (Err(e), _) | (_, Err(e)) => Plan::Fail(e),
                }
            }
            (FlowNet::Accepted(io), OpenKind::Accept) => Plan::Accepted(io.take()),
            (FlowNet::Bind { ip, config }, OpenKind::Accept) => {
                Plan::Bind(SocketAddr::new(*ip, request.port), config.clone())
            }
            (_, OpenKind::Connect) => Plan::Fail(invalid("a server opener cannot connect".into())),
            (_, OpenKind::Accept) => Plan::Fail(invalid("a client opener cannot accept".into())),
        };
        async move {
            match plan {
                Plan::Fail(e) => Err(e),
                Plan::Accepted(io) => {
                    io.ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "session used"))
                }
                Plan::Connect {
                    ctx,
                    source,
                    to,
                    retry,
                } => {
                    let deadline = Instant::now() + OPEN_GRACE;
                    let mut wait = RETRY_FIRST;
                    loop {
                        match ctx.connector.connect(source, to).await {
                            Ok(s) => return Ok(s),
                            Err(e)
                                if retry
                                    && e.io_kind() == Some(io::ErrorKind::ConnectionRefused)
                                    && Instant::now() + wait < deadline =>
                            {
                                sleep(wait).await;
                                wait = (wait * 2).min(RETRY_MAX);
                            }
                            Err(e) => {
                                return Err(io::Error::new(
                                    e.io_kind().unwrap_or(io::ErrorKind::Other),
                                    e,
                                ))
                            }
                        }
                    }
                }
                Plan::Bind(addr, config) => {
                    let deadline = Instant::now() + OPEN_GRACE;
                    let mut wait = RETRY_FIRST;
                    let listener = loop {
                        match KernelListener::bind(addr, &config) {
                            Ok(l) => break l,
                            Err(e)
                                if e.io_kind() == Some(io::ErrorKind::AddrInUse)
                                    && Instant::now() + wait < deadline =>
                            {
                                sleep(wait).await;
                                wait = (wait * 2).min(RETRY_MAX);
                            }
                            Err(e) => {
                                return Err(io::Error::new(
                                    e.io_kind().unwrap_or(io::ErrorKind::Other),
                                    e,
                                ))
                            }
                        }
                    };
                    // The listener drops with this future, which releases the
                    // port once the connection is accepted.
                    let (stream, _peer) = listener.accept().await.map_err(|e| {
                        io::Error::new(e.io_kind().unwrap_or(io::ErrorKind::Other), e)
                    })?;
                    Ok(stream)
                }
            }
        }
    }
}

/// The datagram opener of an instance on this worker, which runs stream
/// flows only. the configuration check refuses a scenario with a UDP
/// flow before the worker starts, so this opener is a guard.
pub(crate) struct NoDatagrams;

impl DatagramOpener for NoDatagrams {
    type Io = KernelDatagram;

    async fn open(&mut self, _request: OpenRequest<'_>) -> io::Result<KernelDatagram> {
        Err(invalid("a worker instance runs stream flows only".into()))
    }
}

/// Lets the instance and the environment share the extension instances of
/// one flow side, so the worker can end them with the outcome of the side.
struct SharedFunctions(Rc<RefCell<Option<SessionExtensions>>>);

impl TemplateFn for SharedFunctions {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        match self.0.borrow_mut().as_mut() {
            Some(f) => f.render(call, rng, out),
            None => Err(FnError::new(
                fps_flow::FnErrorKind::Internal,
                "the extensions of this flow side already ended",
            )),
        }
    }
}

impl InstanceEnv {
    fn index_of(&self, flow: &Flow) -> usize {
        self.scenario
            .flows
            .iter()
            .position(|f| std::ptr::eq(f, flow))
            .unwrap_or(self.entry)
    }

    /// The log entry of the side that the instance is starting.
    fn current(&self) -> Option<&SideLog> {
        self.log.last()
    }
}

impl FlowEnv for InstanceEnv {
    type Stream = TapOpener<CaptureCell, FlowOpener>;
    type Datagram = NoDatagrams;

    fn stream(&mut self, flow: &Flow, side: Side) -> Self::Stream {
        let entry = self.index_of(flow) == self.entry;
        let (id, ended) = self.current().map_or_else(
            || (self.first_id, Rc::default()),
            |l| (l.id, Rc::clone(&l.ended)),
        );
        let kind = match &mut self.net {
            Net::Client {
                ctx,
                source,
                session,
            } => FlowNet::Connect {
                ctx: Rc::clone(ctx),
                source: source.clone(),
                session: *session,
                entry,
            },
            Net::Server {
                accepted,
                bind_ip,
                config,
            } => match accepted.take().filter(|_| entry) {
                Some(io) => FlowNet::Accepted(Some(io)),
                None => FlowNet::Bind {
                    ip: *bind_ip,
                    config: config.clone(),
                },
            },
        };
        TapOpener::new(
            FlowOpener { kind, ended },
            self.worker.capture.clone(),
            id,
            side,
        )
    }

    fn datagram(&mut self, _flow: &Flow, _side: Side) -> NoDatagrams {
        NoDatagrams
    }

    fn tls_upgrader(&mut self, _flow: &Flow, _side: Side) -> Option<Box<dyn TlsUpgrader>> {
        self.tls
            .clone()
            .map(|tls| Box::new(tls) as Box<dyn TlsUpgrader>)
    }

    fn functions(&mut self, flow: &Flow, side: Side) -> Box<dyn TemplateFn> {
        let w = Rc::clone(&self.worker);
        let index = self.index_of(flow);
        let id = self.first_id.wrapping_add(self.log.len() as u64);
        let native = (w.flow.functions)();
        let (functions, extensions) = match &w.flow.extensions {
            Some(set) => {
                let shared = Rc::new(RefCell::new(Some(set.session_functions(
                    SessionKey {
                        seed: w.seed,
                        session_id: id,
                        side,
                        scenario: Arc::clone(&self.scenario),
                        flow: index,
                        transport: Transport::Tcp,
                    },
                    native,
                ))));
                let boxed: Box<dyn TemplateFn> = Box::new(SharedFunctions(Rc::clone(&shared)));
                (boxed, Some(shared))
            }
            None => (native, None),
        };
        w.metrics_of(index).started(&mut w.shard.borrow_mut());
        self.log.push(SideLog {
            id,
            flow: index,
            side,
            started: Instant::now(),
            ended: Rc::default(),
            extensions,
        });
        functions
    }
}

/// Runs the instance of one session start on `worker` and records every flow
/// side that it started in the shard of the worker.
pub(crate) async fn run(worker: Rc<Worker>, local_id: u64, peer: Peer) {
    let scenario = Arc::clone(&worker.flow.scenario);
    let first_id = global_session_id(worker.id, local_id << ID_BLOCK_SHIFT);
    let (sides, net) = match peer {
        Peer::Client(ctx) => {
            let source = source_address(&ctx.config.sources, local_id);
            (
                Sides::Client,
                Net::Client {
                    ctx,
                    source,
                    session: local_id,
                },
            )
        }
        Peer::Server(io) => {
            // Data ports bind on the address that the client dialed.
            let bind_ip = io
                .local_addr()
                .map_or(IpAddr::from([0, 0, 0, 0]), |a| a.ip());
            (
                Sides::Server,
                Net::Server {
                    accepted: Some(io),
                    bind_ip,
                    config: worker.listen_config.clone(),
                },
            )
        }
    };
    let mut env = InstanceEnv {
        worker: Rc::clone(&worker),
        scenario: Arc::clone(&scenario),
        entry: worker.flow.flow,
        net,
        first_id,
        log: Vec::new(),
        tls: worker.session_tls(),
    };
    let mut instance = ScenarioInstance::new(
        &scenario,
        InstanceConfig {
            sides,
            run_seed: worker.seed,
            first_session_id: first_id,
            limits: worker.flow.limits,
        },
    );
    // The instance fails as a whole when one side fails. The sides carry
    // their own failure counts, so the error needs no further record.
    let _ = instance
        .run(&mut env, &worker.filler, worker.aborted())
        .await;
    finish(&worker, &env, &instance, first_id);
}

/// Records the end of every started side, and ends its extensions.
fn finish(worker: &Worker, env: &InstanceEnv, instance: &ScenarioInstance<'_>, first_id: u64) {
    if let Some(tls) = &env.tls {
        worker.record_tls(tls.take_events());
    }
    let mut recorded = vec![false; env.log.len()];
    let now = Instant::now();
    let micros = |log: &SideLog| {
        let end = log.ended.get().unwrap_or(now);
        u64::try_from(end.saturating_duration_since(log.started).as_micros()).unwrap_or(u64::MAX)
    };
    for record in instance.records() {
        let k = usize::try_from(record.session_id.wrapping_sub(first_id)).unwrap_or(usize::MAX);
        let log = env.log.get(k);
        debug_assert!(
            log.is_some_and(|l| l.id == record.session_id && l.side == record.side),
            "the instance gave side {:?} of {} the id {}, which the environment did not predict",
            record.side,
            record.flow,
            record.session_id
        );
        let Some(log) = log else { continue };
        recorded[k] = true;
        worker.metrics_of(log.flow).ended_with(
            &mut worker.shard.borrow_mut(),
            record.stats,
            record.failed,
            micros(log),
        );
        end_extensions(log, record.failed);
    }
    // A side that started and left no record, which only a dropped instance
    // does, counts as an internal failure so that its gauge returns to zero.
    for (log, done) in env.log.iter().zip(recorded) {
        if !done {
            worker.metrics_of(log.flow).ended_with(
                &mut worker.shard.borrow_mut(),
                fps_interp::SideStats::default(),
                Some(Reason::Internal),
                micros(log),
            );
            end_extensions(log, Some(Reason::Internal));
        }
    }
}

/// Ends the extension sessions of a side with the `on-close` hook. The
/// outcome is recorded by now, so a failing hook changes no count.
fn end_extensions(log: &SideLog, failed: Option<Reason>) {
    if let Some(ext) = log.extensions.as_ref().and_then(|e| e.borrow_mut().take()) {
        let _ = ext.finish(close_reason_of(failed));
    }
}

/// Returns the reason that a scenario cannot run as an instance on a worker,
/// or `None` when it can.
pub(crate) fn refusal(scenario: &Scenario, entry: usize) -> Option<String> {
    let Some(flow) = scenario.flows.get(entry) else {
        return Some(format!("scenario {} has no flow {entry}", scenario.name));
    };
    if flow.spawned {
        return Some(format!(
            "flow {} is spawned and cannot be the entry flow of scenario {}",
            flow.id, scenario.name
        ));
    }
    scenario.flows.iter().find(|f| f.l4 == L4::Udp).map(|f| {
        format!(
            "scenario {} has the UDP flow {}; a worker runs the flows of an instance over TCP",
            scenario.name, f.id
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind_opener(ip: IpAddr) -> FlowOpener {
        FlowOpener {
            kind: FlowNet::Bind {
                ip,
                config: ListenConfig::default(),
            },
            ended: Rc::default(),
        }
    }

    #[tokio::test]
    async fn a_bind_waits_for_a_port_that_another_instance_holds() {
        let ip = IpAddr::from([127, 0, 0, 1]);
        let holder = KernelListener::bind(SocketAddr::new(ip, 0), &ListenConfig::default())
            .map_err(|e| e.to_string())
            .expect("the holder binds");
        let port = holder.local_addr().port();
        let mut opener = bind_opener(ip);
        let open = opener.open(OpenRequest {
            kind: OpenKind::Accept,
            host: None,
            port,
        });
        let release = async {
            sleep(Duration::from_millis(40)).await;
            drop(holder);
            // The accept of the opener starts once its bind succeeds.
            for _ in 0..200 {
                if tokio::net::TcpStream::connect((ip, port)).await.is_ok() {
                    return true;
                }
                sleep(Duration::from_millis(5)).await;
            }
            false
        };
        let (stream, dialed) = tokio::join!(open, release);
        assert!(dialed, "the port came free");
        assert!(stream.is_ok(), "{:?}", stream.err());
    }

    #[tokio::test]
    async fn a_used_stream_is_gone_and_a_server_opener_cannot_connect() {
        let mut opener = FlowOpener {
            kind: FlowNet::Accepted(None),
            ended: Rc::default(),
        };
        let accept = OpenRequest {
            kind: OpenKind::Accept,
            host: None,
            port: 1,
        };
        let err = opener.open(accept).await.map(|_| ()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
        let connect = OpenRequest {
            kind: OpenKind::Connect,
            ..accept
        };
        let err = opener.open(connect).await.map(|_| ()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn the_end_of_a_side_is_the_drop_of_its_opener() {
        let ended = Rc::new(Cell::new(None));
        let opener = FlowOpener {
            kind: FlowNet::Accepted(None),
            ended: Rc::clone(&ended),
        };
        assert!(ended.get().is_none());
        drop(opener);
        assert!(ended.get().is_some());
    }
}
