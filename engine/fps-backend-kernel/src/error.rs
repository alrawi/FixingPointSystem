// SPDX-License-Identifier: GPL-3.0-only
//! Errors of the kernel backend.

use std::io;
use std::net::{IpAddr, SocketAddr};

/// Errors that the kernel backend reports for one socket operation.
///
/// Each variant names the addresses and the operation that failed, and the
/// variants that wrap an operating system error expose it through
/// [`std::error::Error::source`] and [`BackendError::io_kind`].
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// A local port range is empty, reversed, or starts at port 0.
    #[error("local port range {first} to {last} is invalid")]
    InvalidPortRange {
        /// The first port of the rejected range.
        first: u16,
        /// The last port of the rejected range.
        last: u16,
    },
    /// The source address and the destination belong to different address families.
    #[error("connect from {source_addr} to {destination}: address families differ")]
    AddressFamily {
        /// The source address of the session.
        source_addr: IpAddr,
        /// The destination of the session.
        destination: SocketAddr,
    },
    /// A step of opening a client session failed.
    #[error("connect from {source_addr} to {destination}: {op} failed")]
    Connect {
        /// The source address of the session.
        source_addr: IpAddr,
        /// The destination of the session.
        destination: SocketAddr,
        /// The failed step, for example `bind` or `connect`.
        op: &'static str,
        /// The operating system error.
        #[source]
        source: io::Error,
    },
    /// The handshake of a client session did not finish within the connect timeout.
    #[error("connect from {source_addr} to {destination} timed out after {timeout_ms} ms")]
    ConnectTimeout {
        /// The source address of the session.
        source_addr: IpAddr,
        /// The destination of the session.
        destination: SocketAddr,
        /// The connect timeout in milliseconds.
        timeout_ms: u64,
    },
    /// A step of opening a listener failed.
    #[error("listen on {address}: {op} failed")]
    Listen {
        /// The address that the listener binds.
        address: SocketAddr,
        /// The failed step, for example `bind` or `listen`.
        op: &'static str,
        /// The operating system error.
        #[source]
        source: io::Error,
    },
    /// Accepting a server session failed.
    #[error("accept on {address} failed")]
    Accept {
        /// The local address of the listener.
        address: SocketAddr,
        /// The operating system error.
        #[source]
        source: io::Error,
    },
}

impl BackendError {
    /// Returns the kind of the wrapped operating system error, if any.
    ///
    /// Statistics use it to count address errors, for example
    /// [`io::ErrorKind::AddrNotAvailable`] when a local port range is exhausted.
    pub fn io_kind(&self) -> Option<io::ErrorKind> {
        match self {
            Self::Connect { source, .. }
            | Self::Listen { source, .. }
            | Self::Accept { source, .. } => Some(source.kind()),
            Self::ConnectTimeout { .. } => Some(io::ErrorKind::TimedOut),
            Self::InvalidPortRange { .. } | Self::AddressFamily { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_error_names_addresses_and_step_and_keeps_its_cause() {
        let err = BackendError::Connect {
            source_addr: "127.0.0.2".parse().unwrap(),
            destination: "127.0.0.1:80".parse().unwrap(),
            op: "bind",
            source: io::Error::from(io::ErrorKind::AddrInUse),
        };
        assert_eq!(
            err.to_string(),
            "connect from 127.0.0.2 to 127.0.0.1:80: bind failed"
        );
        assert_eq!(err.io_kind(), Some(io::ErrorKind::AddrInUse));
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn timeout_reports_timed_out_kind() {
        let err = BackendError::ConnectTimeout {
            source_addr: "127.0.0.2".parse().unwrap(),
            destination: "127.0.0.1:80".parse().unwrap(),
            timeout_ms: 250,
        };
        assert_eq!(err.io_kind(), Some(io::ErrorKind::TimedOut));
        assert!(err.to_string().ends_with("timed out after 250 ms"));
    }
}
