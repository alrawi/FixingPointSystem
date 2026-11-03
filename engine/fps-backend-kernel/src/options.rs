// SPDX-License-Identifier: GPL-3.0-only
//! Configuration of client and server sockets.

use std::io;
use std::time::Duration;

use socket2::{Domain, SockRef};

use crate::{sys, BackendError};

/// An inclusive range of local ports for client sessions.
///
/// The kernel picks each source port from this range in place of the network
/// namespace range in `net.ipv4.ip_local_port_range`. With
/// `IP_BIND_ADDRESS_NO_PORT` a port is shared by sessions that differ in
/// source address or destination, so the range limits the concurrent
/// sessions per source address and destination, not per source address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortRange {
    first: u16,
    last: u16,
}

impl PortRange {
    /// Returns the range from `first` to `last`, both included.
    ///
    /// The range must not be empty and must not contain port 0.
    pub fn new(first: u16, last: u16) -> Result<Self, BackendError> {
        if first == 0 || first > last {
            return Err(BackendError::InvalidPortRange { first, last });
        }
        Ok(Self { first, last })
    }

    /// Returns the first port of the range.
    pub fn first(&self) -> u16 {
        self.first
    }

    /// Returns the last port of the range.
    pub fn last(&self) -> u16 {
        self.last
    }

    /// Returns the number of ports in the range.
    pub fn len(&self) -> u32 {
        u32::from(self.last) - u32::from(self.first) + 1
    }

    /// Returns false, because a valid range holds at least one port.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Returns true when `port` lies inside the range.
    pub fn contains(&self, port: u16) -> bool {
        (self.first..=self.last).contains(&port)
    }

    /// Returns the value of `IP_LOCAL_PORT_RANGE`, with the last port in the
    /// upper 16 bits and the first port in the lower 16 bits.
    pub(crate) fn encode(self) -> u32 {
        (u32::from(self.last) << 16) | u32::from(self.first)
    }
}

/// The `SO_LINGER` behavior of a socket on close.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Linger {
    /// Keeps the kernel default, so close sends FIN and returns at once while
    /// the kernel delivers unsent data in the background.
    #[default]
    Default,
    /// Sets `SO_LINGER` to zero, so every close discards unsent data and sends
    /// a reset, as [`fps_io::SessionIo::abort`] does.
    Abort,
    /// Sets `SO_LINGER` to the duration, rounded down to whole seconds.
    ///
    /// Close then blocks the calling thread until the kernel delivered unsent
    /// data or the duration passed, also on a nonblocking socket. A worker
    /// thread that drops such a session stalls, so this setting suits only
    /// tests and low rates.
    Timeout(Duration),
}

/// Options that apply to every socket of a client or a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketOptions {
    /// `SO_SNDBUF` in bytes, or `None` for the kernel default. Linux doubles
    /// the value and caps it at `net.core.wmem_max`.
    pub send_buffer: Option<usize>,
    /// `SO_RCVBUF` in bytes, or `None` for the kernel default. Linux doubles
    /// the value and caps it at `net.core.rmem_max`. The backend sets it
    /// before the handshake, so the window scale reflects it.
    pub recv_buffer: Option<usize>,
    /// `TCP_NODELAY`. True turns Nagle's algorithm off, so every write leaves
    /// at once and the flow program controls segmentation.
    pub nodelay: bool,
    /// The `SO_LINGER` behavior on close.
    pub linger: Linger,
    /// `IP_TTL` on IPv4 sockets and `IPV6_UNICAST_HOPS` on IPv6 sockets, or
    /// `None` for `net.ipv4.ip_default_ttl`. An [`crate::OsProfile`] sets it.
    pub ttl: Option<u8>,
    /// `TCP_MAXSEG`, the MSS that the SYN or SYN-ACK advertises, or `None`
    /// for the MSS of the route. The kernel uses the smaller of the two.
    pub mss: Option<u16>,
    /// `TCP_WINDOW_CLAMP` in bytes, which bounds the receive window and the
    /// window scale of the handshake, or `None` for no clamp. Linux raises
    /// values below half of `SOCK_MIN_RCVBUF` to that half.
    pub window_clamp: Option<u32>,
}

impl Default for SocketOptions {
    fn default() -> Self {
        Self {
            send_buffer: None,
            recv_buffer: None,
            nodelay: true,
            linger: Linger::Default,
            ttl: None,
            mss: None,
            window_clamp: None,
        }
    }
}

impl SocketOptions {
    /// Applies the options to a socket and returns the name of the failed
    /// option on error.
    pub(crate) fn apply(&self, sock: SockRef<'_>) -> Result<(), (&'static str, io::Error)> {
        if let Some(size) = self.send_buffer {
            sock.set_send_buffer_size(size)
                .map_err(|e| ("set SO_SNDBUF", e))?;
        }
        if let Some(size) = self.recv_buffer {
            sock.set_recv_buffer_size(size)
                .map_err(|e| ("set SO_RCVBUF", e))?;
        }
        sock.set_tcp_nodelay(self.nodelay)
            .map_err(|e| ("set TCP_NODELAY", e))?;
        let linger = match self.linger {
            Linger::Default => None,
            Linger::Abort => Some(Some(Duration::ZERO)),
            Linger::Timeout(d) => Some(Some(d)),
        };
        if let Some(value) = linger {
            sock.set_linger(value).map_err(|e| ("set SO_LINGER", e))?;
        }
        if let Some(ttl) = self.ttl {
            if sock.domain().map_err(|e| ("get SO_DOMAIN", e))? == Domain::IPV6 {
                sock.set_unicast_hops_v6(u32::from(ttl))
                    .map_err(|e| ("set IPV6_UNICAST_HOPS", e))?;
            } else {
                sock.set_ttl_v4(u32::from(ttl))
                    .map_err(|e| ("set IP_TTL", e))?;
            }
        }
        if let Some(mss) = self.mss {
            sock.set_tcp_mss(u32::from(mss))
                .map_err(|e| ("set TCP_MAXSEG", e))?;
        }
        if let Some(clamp) = self.window_clamp {
            sys::set_window_clamp(&sock, clamp).map_err(|e| ("set TCP_WINDOW_CLAMP", e))?;
        }
        Ok(())
    }
}

/// Configuration of a [`crate::Connector`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectConfig {
    /// The local port range of client sessions, or `None` for the range of
    /// the network namespace.
    pub local_ports: Option<PortRange>,
    /// The longest time a handshake may take before the connect fails.
    pub connect_timeout: Duration,
    /// Options for every client socket.
    pub socket: SocketOptions,
}

impl Default for ConnectConfig {
    fn default() -> Self {
        Self {
            local_ports: None,
            connect_timeout: Duration::from_secs(5),
            socket: SocketOptions::default(),
        }
    }
}

/// Configuration of a [`crate::KernelListener`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenConfig {
    /// The accept queue length that `listen` requests. The kernel caps it at
    /// `net.core.somaxconn`, and values above `i32::MAX` become `i32::MAX`.
    pub backlog: u32,
    /// `SO_REUSEADDR`, which lets a listener bind while earlier sessions on
    /// its address wait in TIME_WAIT.
    pub reuse_address: bool,
    /// Options for the listening socket. Accepted sessions inherit them.
    pub socket: SocketOptions,
}

impl Default for ListenConfig {
    fn default() -> Self {
        Self {
            backlog: 4096,
            reuse_address: true,
            socket: SocketOptions::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_range_rejects_empty_reversed_and_zero_ranges() {
        assert!(PortRange::new(0, 10).is_err());
        assert!(PortRange::new(10, 9).is_err());
        let one = PortRange::new(80, 80).unwrap();
        assert_eq!(one.len(), 1);
        assert!(one.contains(80) && !one.contains(81));
        let full = PortRange::new(1, u16::MAX).unwrap();
        assert_eq!(full.len(), 65535);
    }

    #[test]
    fn port_range_encodes_last_port_high_and_first_port_low() {
        let range = PortRange::new(40000, 44999).unwrap();
        assert_eq!(range.encode(), (44999 << 16) | 40000);
    }

    #[test]
    fn defaults_turn_nagle_off_and_keep_kernel_linger() {
        let opts = SocketOptions::default();
        assert!(opts.nodelay);
        assert_eq!(opts.linger, Linger::Default);
        assert_eq!(ListenConfig::default().backlog, 4096);
        assert_eq!(ConnectConfig::default().local_ports, None);
        assert_eq!((opts.ttl, opts.mss, opts.window_clamp), (None, None, None));
    }

    #[test]
    fn ttl_mss_and_window_clamp_reach_ipv4_and_ipv6_sockets() {
        use socket2::{Protocol, Socket, Type};
        let opts = SocketOptions {
            ttl: Some(128),
            mss: Some(1400),
            window_clamp: Some(3_145_728),
            ..SocketOptions::default()
        };
        let v4 = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
        opts.apply(SockRef::from(&v4)).unwrap();
        assert_eq!(v4.ttl_v4().unwrap(), 128);
        assert_eq!(v4.tcp_mss().unwrap(), 1400);
        #[cfg(target_os = "linux")]
        assert_eq!(sys::window_clamp(&v4).unwrap(), 3_145_728);
        // Hosts without IPv6 cannot create the socket, so the check skips.
        if let Ok(v6) = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP)) {
            opts.apply(SockRef::from(&v6)).unwrap();
            assert_eq!(v6.unicast_hops_v6().unwrap(), 128);
            assert_eq!(v6.tcp_mss().unwrap(), 1400);
        }
    }
}
