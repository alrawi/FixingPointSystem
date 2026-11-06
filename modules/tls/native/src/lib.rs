// SPDX-License-Identifier: GPL-3.0-only
//! TLS transport module of the engine.
//!
//! The crate wraps a [`fps_io::SessionIo`] session in TLS 1.2 or 1.3 with
//! rustls and the ring provider, and the wrapped session is again a
//! [`fps_io::SessionIo`], so the interpreter runs a flow program over it
//! without change. Four parts make up the module.
//!
//! * [`ClientProfile`] and [`ServerProfile`] fix what a side puts on the wire:
//!   versions, SNI, ALPN, key exchange groups, and cipher suite order.
//! * [`RunAuthority`] generates a certificate authority for one run and issues
//!   server certificates from it.
//! * [`TlsConnector`] and [`TlsAcceptor`] run the handshake over any session
//!   and return a [`TlsSession`] with a [`HandshakeReport`]. A connector keeps
//!   its resumption store, so later sessions of the same connector resume.
//! * [`NssKeyLog`] writes the secrets of every handshake in the NSS key log
//!   format, which Wireshark and tshark read to decrypt a capture.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod authority;
mod error;
mod keylog;
mod profile;
mod session;

pub use authority::{LeafKey, RunAuthority, ServerIdentity};
pub use error::{Side, TlsError};
pub use keylog::NssKeyLog;
pub use profile::{
    CipherSuite, ClientProfile, ClientResumption, Group, ServerProfile, ServerResumption,
    TlsVersion,
};
pub use session::{HandshakeReport, TlsAcceptor, TlsConnector, TlsSession};

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "tls";
