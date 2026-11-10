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
//!   entry connection, accepts one connection, and releases the port. The
//!   listener of a port is shared by every instance of the worker that waits
//!   on it and hands each connection to the instance that expects the address
//!   it came from, see [`crate::data_ports`]. A bind that finds the port in
//!   use by another process is retried for [`OPEN_GRACE`].
//!
//! A UDP flow opens as a datagram session of the same three kinds, see
//! [`crate::datagram`]. The entry flow of a server is the first session of a
//! UDP port that the worker listens on, and a media flow binds the rendered
//! port, which its sessions share with the other instances of the worker.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use fps_backend_kernel::KernelStream;
use fps_capture::CaptureCell;
use fps_flow::program::{Flow, L4};
use fps_flow::{FnCall, FnError, Scenario, TemplateFn};
use fps_interp::{
    FlowEnv, InstanceConfig, OpenKind, OpenRequest, Opener, Reason, ScenarioInstance, Side, Sides,
    TlsUpgrader,
};
use rand::RngCore;
use tokio::time::{sleep, Instant};

use crate::data_ports::DataPorts;
use crate::datagram::{
    DatagramFlowOpener, DatagramNet, ServerDatagram, TapDatagramOpener,
};
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

/// The first wait between two tries of a bind or a connect, which doubles up
/// to [`RETRY_MAX`].
pub(crate) const RETRY_FIRST: Duration = Duration::from_millis(2);
/// The longest wait between two tries.
pub(crate) const RETRY_MAX: Duration = Duration::from_millis(50);

/// The end that a worker task owns of the connection of an instance.
pub(crate) enum Peer {
    /// A client session start, which dials.
    Client(Rc<ClientCtx>),
    /// A server session over TCP, which the listener of the worker accepted.
    Server(KernelStream),
    /// A server session over UDP, which the first datagram of a peer opened on
    /// a port that the worker listens on.
    ServerDatagram(ServerDatagram),
}

/// The session that a server worker accepted for the entry flow.
enum Entry {
    Stream(KernelStream),
    Datagram(ServerDatagram),
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
        /// The accepted session, until the entry flow takes it.
        accepted: Option<Entry>,
        /// The address that data ports bind on.
        bind_ip: IpAddr,
        /// The host of the peer of the entry session. A data session of this
        /// instance comes from the same host.
        expect: Option<IpAddr>,
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
    /// Waits for the connection of the expected host on the rendered port.
    Bind {
        ip: IpAddr,
        ports: DataPorts,
        expect: Option<IpAddr>,
    },
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
    Bind(SocketAddr, DataPorts, Option<IpAddr>),
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
            (FlowNet::Bind { ip, ports, expect }, OpenKind::Accept) => {
                Plan::Bind(SocketAddr::new(*ip, request.port), ports.clone(), *expect)
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
                Plan::Bind(addr, ports, expect) => ports.accept(addr, expect).await,
            }
        }
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

/// Takes the accepted session of the entry flow when it has the transport of
/// the flow that asks, and leaves it for the flow that wants it otherwise.
fn take_entry<T>(
    accepted: &mut Option<Entry>,
    entry: bool,
    pick: impl FnOnce(Entry) -> Result<T, Entry>,
) -> Option<T> {
    if !entry {
        return None;
    }
    match pick(accepted.take()?) {
        Ok(io) => Some(io),
        Err(other) => {
            *accepted = Some(other);
            None
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
    type Datagram = TapDatagramOpener<DatagramFlowOpener>;

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
                expect,
            } => match take_entry(accepted, entry, |e| match e {
                Entry::Stream(io) => Ok(io),
                other => Err(other),
            }) {
                Some(io) => FlowNet::Accepted(Some(io)),
                None => FlowNet::Bind {
                    ip: *bind_ip,
                    ports: self.worker.data_ports.clone(),
                    expect: *expect,
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

    fn datagram(&mut self, flow: &Flow, side: Side) -> Self::Datagram {
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
            } => DatagramNet::Connect {
                ctx: Rc::clone(ctx),
                source: source.clone(),
                session: *session,
                entry,
            },
            Net::Server {
                accepted,
                bind_ip,
                expect,
            } => match take_entry(accepted, entry, |e| match e {
                Entry::Datagram(io) => Ok(io),
                other => Err(other),
            }) {
                Some(io) => DatagramNet::Accepted(Some(io)),
                None => DatagramNet::Bind {
                    ip: *bind_ip,
                    ports: self.worker.datagram_ports.clone(),
                    expect: *expect,
                },
            },
        };
        TapDatagramOpener::new(
            DatagramFlowOpener { kind, ended },
            self.worker.capture.clone(),
            id,
            side,
        )
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
                        transport: if flow.l4 == L4::Udp {
                            Transport::Udp
                        } else {
                            Transport::Tcp
                        },
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
            // Data ports bind on the address that the client dialed, and the
            // data sessions of the instance come from the host of the client.
            let bind_ip = io
                .local_addr()
                .map_or(IpAddr::from([0, 0, 0, 0]), |a| a.ip());
            let expect = io.peer_addr().ok().map(|a| a.ip());
            (
                Sides::Server,
                Net::Server {
                    accepted: Some(Entry::Stream(io)),
                    bind_ip,
                    expect,
                },
            )
        }
        Peer::ServerDatagram(session) => (
            Sides::Server,
            Net::Server {
                bind_ip: session.local_addr().ip(),
                expect: Some(session.peer_addr().ip()),
                accepted: Some(Entry::Datagram(session)),
            },
        ),
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

/// Returns true when the entry flow of a scenario runs over UDP, so a server
/// worker listens on a datagram port and not on a TCP listener.
pub(crate) fn entry_is_udp(scenario: &Scenario, entry: usize) -> bool {
    scenario.flows.get(entry).is_some_and(|f| f.l4 == L4::Udp)
}

/// Returns the reason that a scenario cannot run as an instance on a worker,
/// or `None` when it can. A flow may be TCP or UDP, and the entry flow of a
/// server worker decides which kind of port the worker listens on, see
/// [`entry_is_udp`].
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
    None
}

#[cfg(test)]
mod tests {
    use super::*;

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
