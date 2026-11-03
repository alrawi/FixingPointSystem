// SPDX-License-Identifier: GPL-3.0-only
//! Server sessions on host pool addresses.

use std::net::{IpAddr, SocketAddr};

use socket2::{Domain, Protocol, SockRef, Socket, Type};
use tokio::net::TcpListener;

use crate::{BackendError, KernelStream, ListenConfig};

/// A listening kernel TCP socket on one pool address.
///
/// Accepted sessions inherit the buffer size, Nagle, and linger options of
/// the listener, so an accept costs no extra system calls.
#[derive(Debug)]
pub struct KernelListener {
    inner: TcpListener,
    address: SocketAddr,
}

impl KernelListener {
    /// Binds `address` and starts listening with the configured backlog.
    ///
    /// Port 0 lets the kernel pick a port, which [`KernelListener::local_addr`]
    /// then reports. An IPv6 listener accepts only IPv6 sessions, so IPv4 and
    /// IPv6 pool addresses can share a port. The call must run inside a Tokio
    /// runtime with I/O enabled.
    pub fn bind(address: SocketAddr, config: &ListenConfig) -> Result<Self, BackendError> {
        let fail = |op: &'static str| {
            move |e: std::io::Error| BackendError::Listen {
                address,
                op,
                source: e,
            }
        };
        let sock = Socket::new(
            Domain::for_address(address),
            Type::STREAM,
            Some(Protocol::TCP),
        )
        .map_err(fail("socket"))?;
        if address.is_ipv6() {
            sock.set_only_v6(true).map_err(fail("set IPV6_V6ONLY"))?;
        }
        sock.set_reuse_address(config.reuse_address)
            .map_err(fail("set SO_REUSEADDR"))?;
        config
            .socket
            .apply(SockRef::from(&sock))
            .map_err(|(op, e)| fail(op)(e))?;
        sock.set_nonblocking(true).map_err(fail("set O_NONBLOCK"))?;
        sock.bind(&address.into()).map_err(fail("bind"))?;
        let backlog = i32::try_from(config.backlog).unwrap_or(i32::MAX);
        sock.listen(backlog).map_err(fail("listen"))?;
        let inner =
            TcpListener::from_std(std::net::TcpListener::from(sock)).map_err(fail("register"))?;
        let address = inner.local_addr().map_err(fail("getsockname"))?;
        Ok(Self { inner, address })
    }

    /// Returns the bound address and port.
    pub fn local_addr(&self) -> SocketAddr {
        self.address
    }

    /// Waits for the next session and returns it with the peer address.
    ///
    /// An error concerns one accept, for example `EMFILE`, and the listener
    /// stays usable.
    pub async fn accept(&self) -> Result<(KernelStream, SocketAddr), BackendError> {
        match self.inner.accept().await {
            Ok((stream, peer)) => Ok((KernelStream::new(stream), peer)),
            Err(source) => Err(BackendError::Accept {
                address: self.address,
                source,
            }),
        }
    }
}

/// Binds one listener per pool address on the same port.
///
/// With port 0 each listener gets its own kernel chosen port. The call fails
/// on the first address that cannot bind and closes the listeners it opened
/// before.
pub fn bind_pool<I>(
    addresses: I,
    port: u16,
    config: &ListenConfig,
) -> Result<Vec<KernelListener>, BackendError>
where
    I: IntoIterator<Item = IpAddr>,
{
    addresses
        .into_iter()
        .map(|ip| KernelListener::bind(SocketAddr::new(ip, port), config))
        .collect()
}
