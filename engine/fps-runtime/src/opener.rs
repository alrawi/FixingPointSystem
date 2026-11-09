// SPDX-License-Identifier: GPL-3.0-only
//! The openers that connect the interpreter to the kernel backend.

use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};

use fps_backend_kernel::{Connector, KernelStream};
use fps_capture::{CaptureCell, FlowEndpoints, Tapped, Transport};
use fps_interp::{OpenKind, OpenRequest, Opener, Side};
use fps_io::SessionIo;

use crate::config::SourceHosts;

/// The number of high bits of a session id that hold the worker id.
pub(crate) const WORKER_ID_BITS: u32 = 16;

/// Returns the run wide session id of session `local` on worker `worker`.
///
/// The worker id sits in the top 16 bits, so the sessions of different
/// workers never share an id and therefore never share a generator stream.
/// Local ids above 2^48 wrap into the low bits.
pub fn global_session_id(worker: u32, local: u64) -> u64 {
    let shift = 64 - WORKER_ID_BITS;
    (u64::from(worker) << shift) | (local & ((1u64 << shift) - 1))
}

/// Opens client sessions with a kernel connector.
///
/// The destination is the rendered `connect` host when the program sets
/// one, which must then be an IP address, and otherwise server address
/// `session mod len`. The port is the override when set and otherwise the
/// rendered port. The source address is the host that the selector picks for
/// the session.
#[derive(Debug)]
pub struct ClientOpener<'a> {
    connector: &'a Connector,
    servers: &'a [IpAddr],
    port: Option<u16>,
    source: Result<IpAddr, String>,
    session: u64,
}

impl<'a> ClientOpener<'a> {
    /// Returns the opener of local session `session` of a worker.
    pub fn new(
        connector: &'a Connector,
        sources: &SourceHosts,
        servers: &'a [IpAddr],
        port: Option<u16>,
        session: u64,
    ) -> Self {
        let index = sources.selector.select(session);
        let source = if sources.ipv6 {
            sources.pool.ipv6_addr(index).map(|a| a.map(IpAddr::V6))
        } else {
            sources.pool.ipv4_addr(index).map(|a| a.map(IpAddr::V4))
        };
        let source = match source {
            Ok(Some(a)) => Ok(a),
            Ok(None) => Err(format!(
                "pool {} has no {} prefix",
                sources.pool.name(),
                if sources.ipv6 { "IPv6" } else { "IPv4" }
            )),
            Err(e) => Err(e.to_string()),
        };
        Self {
            connector,
            servers,
            port,
            source,
            session,
        }
    }

    fn destination(&self, request: &OpenRequest<'_>) -> io::Result<SocketAddr> {
        let port = self.port.unwrap_or(request.port);
        let ip = match request.host {
            Some(host) => std::str::from_utf8(host)
                .ok()
                .and_then(|h| h.parse::<IpAddr>().ok())
                .ok_or_else(|| {
                    invalid(format!(
                        "connect host {} is not an IP address",
                        String::from_utf8_lossy(host)
                    ))
                })?,
            None => {
                let n = self.servers.len() as u64;
                if n == 0 {
                    return Err(invalid(
                        "no server address for a connect without host".into(),
                    ));
                }
                let i = usize::try_from(self.session % n).unwrap_or(0);
                *self
                    .servers
                    .get(i)
                    .ok_or_else(|| invalid("invariant: server index in range".into()))?
            }
        };
        Ok(SocketAddr::new(ip, port))
    }
}

impl Opener for ClientOpener<'_> {
    type Io = KernelStream;

    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<KernelStream>> {
        let target = match request.kind {
            OpenKind::Connect => self.destination(&request),
            OpenKind::Accept => Err(invalid("a client opener cannot accept".into())),
        };
        let source = self.source.clone().map_err(invalid);
        let connector = self.connector;
        async move {
            let (source, destination) = (source?, target?);
            connector
                .connect(source, destination)
                .await
                .map_err(|e| io::Error::new(e.io_kind().unwrap_or(io::ErrorKind::Other), e))
        }
    }
}

/// Hands the interpreter one session that a listener already accepted.
#[derive(Debug)]
pub(crate) struct Accepted<S> {
    io: Option<S>,
}

impl<S> Accepted<S> {
    pub(crate) fn new(io: S) -> Self {
        Self { io: Some(io) }
    }
}

impl<S: SessionIo> Opener for Accepted<S> {
    type Io = S;

    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<S>> {
        let io = match request.kind {
            OpenKind::Accept => self
                .io
                .take()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "session used")),
            OpenKind::Connect => Err(invalid("a server opener cannot connect".into())),
        };
        std::future::ready(io)
    }
}

/// A session transport that knows its two addresses, which the tap needs to
/// synthesize frames.
pub(crate) trait Endpoints {
    /// Returns the local and the peer address, or `None` when the transport
    /// has none, for example after a reset.
    fn endpoints(&self) -> Option<(SocketAddr, SocketAddr)>;
}

impl Endpoints for KernelStream {
    fn endpoints(&self) -> Option<(SocketAddr, SocketAddr)> {
        Some((self.local_addr().ok()?, self.peer_addr().ok()?))
    }
}

/// Wraps the sessions that an opener opens in the tap of the current capture.
///
/// It reads the [`CaptureCell`] once per open, which is one atomic load while
/// no capture runs. A session that opens while a capture runs is wrapped by
/// [`fps_capture::Capture::begin`], which decides by the sampler whether the
/// session is recorded. The local address of a client session is its source
/// and that of a server session is its listener.
pub(crate) struct TapOpener<'a, O> {
    inner: O,
    cell: &'a CaptureCell,
    session_id: u64,
    side: Side,
}

impl<'a, O> TapOpener<'a, O> {
    /// Wraps `inner`, whose sessions carry the run wide id `session_id`.
    pub(crate) fn new(inner: O, cell: &'a CaptureCell, session_id: u64, side: Side) -> Self {
        Self {
            inner,
            cell,
            session_id,
            side,
        }
    }
}

impl<O: Opener> Opener for TapOpener<'_, O>
where
    O::Io: Endpoints,
{
    type Io = Tapped<O::Io>;

    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<Self::Io>> {
        let (cell, id, side) = (self.cell, self.session_id, self.side);
        let opened = self.inner.open(request);
        async move {
            let io = opened.await?;
            let Some(capture) = cell.current() else {
                return Ok(Tapped::new(io, None));
            };
            let handle = io.endpoints().and_then(|(local, peer)| {
                let (client, server, side) = match side {
                    Side::Client => (local, peer, fps_capture::Side::Client),
                    Side::Server => (peer, local, fps_capture::Side::Server),
                };
                let flow = FlowEndpoints::new(client, server).ok()?;
                capture.begin(id, flow, Transport::Tcp, side)
            });
            Ok(Tapped::new(io, handle))
        }
    }
}

fn invalid(detail: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, detail)
}

#[cfg(test)]
mod tests {
    use fps_hosts::{HostPool, HostSelector, PoolSpec, Selection};

    use super::*;

    #[test]
    fn worker_ids_sit_in_the_high_bits() {
        assert_eq!(global_session_id(0, 5), 5);
        assert_eq!(global_session_id(1, 5), (1 << 48) | 5);
        assert_ne!(global_session_id(1, 0), global_session_id(2, 0));
        assert_eq!(global_session_id(3, 1 << 48), 3 << 48);
    }

    #[test]
    fn picks_server_round_robin_and_source_from_the_pool() {
        let pool = HostPool::parse(&PoolSpec {
            name: "clients",
            ipv4: Some("127.30.0.0/24"),
            count: 4,
            ..PoolSpec::default()
        })
        .unwrap();
        let selector =
            HostSelector::new("clients", pool.range(), Selection::RoundRobin, 1, 0).unwrap();
        let sources = SourceHosts {
            pool,
            selector,
            ipv6: false,
        };
        let connector = Connector::new(Default::default());
        let servers: Vec<IpAddr> =
            vec!["127.31.0.1".parse().unwrap(), "127.31.0.2".parse().unwrap()];
        let req = OpenRequest {
            kind: OpenKind::Connect,
            host: None,
            port: 8080,
        };
        let o = ClientOpener::new(&connector, &sources, &servers, None, 3);
        assert_eq!(
            o.destination(&req).unwrap(),
            "127.31.0.2:8080".parse().unwrap()
        );
        assert_eq!(
            o.source.clone().unwrap(),
            "127.30.0.4".parse::<IpAddr>().unwrap()
        );
        let o = ClientOpener::new(&connector, &sources, &servers, Some(9), 2);
        assert_eq!(
            o.destination(&req).unwrap(),
            "127.31.0.1:9".parse().unwrap()
        );
        let host = OpenRequest {
            host: Some(b"127.32.0.9"),
            ..req
        };
        assert_eq!(
            o.destination(&host).unwrap(),
            "127.32.0.9:9".parse().unwrap()
        );
        let bad = OpenRequest {
            host: Some(b"www.example.test"),
            ..req
        };
        assert_eq!(
            o.destination(&bad).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
