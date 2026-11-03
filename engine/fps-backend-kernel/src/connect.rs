// SPDX-License-Identifier: GPL-3.0-only
//! Client sessions from host pool source addresses.

use std::net::{IpAddr, SocketAddr};

use socket2::{Domain, Protocol, SockRef, Socket, Type};
use tokio::net::TcpSocket;

use crate::{sys, BackendError, ConnectConfig, KernelStream};

/// Opens client sessions over kernel TCP sockets.
///
/// Each session binds its socket to a source address from the host pool with
/// port 0 and `IP_BIND_ADDRESS_NO_PORT`, so the kernel picks the source port at
/// connect time from the configured [`crate::PortRange`] and reuses a port for
/// different destinations. One connector serves every session of a worker.
#[derive(Debug, Clone)]
pub struct Connector {
    config: ConnectConfig,
}

impl Connector {
    /// Returns a connector with the configuration.
    pub fn new(config: ConnectConfig) -> Self {
        Self { config }
    }

    /// Returns the configuration of the connector.
    pub fn config(&self) -> &ConnectConfig {
        &self.config
    }

    /// Opens a session from `source` to `destination`.
    ///
    /// The source address must belong to the address family of the
    /// destination and must be local to the host, which for pool addresses
    /// means a local route as HST-02 sets up. An exhausted port range fails
    /// with an error whose [`BackendError::io_kind`] is
    /// [`std::io::ErrorKind::AddrNotAvailable`]. The call must run inside a
    /// Tokio runtime with I/O and time enabled.
    pub async fn connect(
        &self,
        source: IpAddr,
        destination: SocketAddr,
    ) -> Result<KernelStream, BackendError> {
        let fail = |op: &'static str| {
            move |e: std::io::Error| BackendError::Connect {
                source_addr: source,
                destination,
                op,
                source: e,
            }
        };
        if source.is_ipv4() != destination.is_ipv4() {
            return Err(BackendError::AddressFamily {
                source_addr: source,
                destination,
            });
        }
        let sock = self
            .prepare(source, destination)
            .map_err(|(op, e)| fail(op)(e))?;
        let socket = TcpSocket::from_std_stream(std::net::TcpStream::from(sock));
        let timeout = self.config.connect_timeout;
        match tokio::time::timeout(timeout, socket.connect(destination)).await {
            Ok(Ok(stream)) => Ok(KernelStream::new(stream)),
            Ok(Err(e)) => Err(fail("connect")(e)),
            Err(_) => Err(BackendError::ConnectTimeout {
                source_addr: source,
                destination,
                timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            }),
        }
    }

    /// Creates the socket, applies every option, and binds the source address.
    fn prepare(
        &self,
        source: IpAddr,
        destination: SocketAddr,
    ) -> Result<Socket, (&'static str, std::io::Error)> {
        let sock = Socket::new(
            Domain::for_address(destination),
            Type::STREAM,
            Some(Protocol::TCP),
        )
        .map_err(|e| ("socket", e))?;
        self.config.socket.apply(SockRef::from(&sock))?;
        sys::set_bind_address_no_port(&sock).map_err(|e| ("set IP_BIND_ADDRESS_NO_PORT", e))?;
        if let Some(range) = self.config.local_ports {
            sys::set_local_port_range(&sock, range).map_err(|e| ("set IP_LOCAL_PORT_RANGE", e))?;
        }
        sock.set_nonblocking(true)
            .map_err(|e| ("set O_NONBLOCK", e))?;
        sock.bind(&SocketAddr::new(source, 0).into())
            .map_err(|e| ("bind", e))?;
        Ok(sock)
    }
}
