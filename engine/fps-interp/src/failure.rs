// SPDX-License-Identifier: GPL-3.0-only
//! Session failures and their reasons.
//!
//! Every failure carries a [`Reason`], whose label is one value of the
//! `reason` label of `contracts/metrics/catalog.yaml`, so the worker records
//! `sessions.failed` without a translation table. The [`FailureKind`] keeps
//! the detail and the cause for logs.

use std::fmt;
use std::io;

use fps_flow::TemplateError;

use crate::rng::Side;

/// The failure reasons of the metric catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reason {
    /// `connect` or `accept` did not complete within the open timeout.
    ConnectTimeout,
    /// The peer refused the connection.
    ConnectRefused,
    /// An `expect`, or the drain of `close: fin`, did not finish in time.
    ExpectTimeout,
    /// Received data cannot match the stop rule.
    PatternMismatch,
    /// The peer reset the session while the program did not expect a close.
    UnexpectedReset,
    /// The peer closed the session before the program finished.
    PeerClosed,
    /// The TLS layer of the transport failed.
    TlsFailure,
    /// The engine stopped the session, for example because the run stopped.
    Aborted,
    /// Any other failure, such as a render error or an invalid parameter.
    Internal,
}

impl Reason {
    /// Every reason in catalog order.
    pub const ALL: [Reason; 9] = [
        Reason::ConnectTimeout,
        Reason::ConnectRefused,
        Reason::ExpectTimeout,
        Reason::PatternMismatch,
        Reason::UnexpectedReset,
        Reason::PeerClosed,
        Reason::TlsFailure,
        Reason::Aborted,
        Reason::Internal,
    ];

    /// Returns the catalog value of the `reason` label.
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::ConnectTimeout => "connect_timeout",
            Reason::ConnectRefused => "connect_refused",
            Reason::ExpectTimeout => "expect_timeout",
            Reason::PatternMismatch => "pattern_mismatch",
            Reason::UnexpectedReset => "unexpected_reset",
            Reason::PeerClosed => "peer_closed",
            Reason::TlsFailure => "tls_failure",
            Reason::Aborted => "aborted",
            Reason::Internal => "internal",
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What went wrong in one action.
#[derive(Debug, thiserror::Error)]
pub enum FailureKind {
    /// `connect` or `accept` did not complete in time.
    #[error("open timed out after {timeout_ms} ms")]
    OpenTimeout {
        /// The open timeout.
        timeout_ms: u64,
    },
    /// The peer refused the connection.
    #[error("connection refused")]
    Refused {
        /// The transport error.
        #[source]
        source: io::Error,
    },
    /// An `expect` did not finish within its timeout.
    #[error("expect timed out after {timeout_ms} ms with {received} bytes received")]
    ExpectTimeout {
        /// The timeout of the action.
        timeout_ms: u64,
        /// Bytes that the action read before the timeout.
        received: usize,
    },
    /// The peer did not close within the drain timeout of `close: fin`.
    #[error("peer did not close within {timeout_ms} ms")]
    DrainTimeout {
        /// The drain timeout.
        timeout_ms: u64,
    },
    /// `expect pattern` buffered more unmatched bytes than the limit.
    #[error("{buffered} bytes buffered without a match, limit {limit}")]
    PatternOverflow {
        /// Bytes buffered when the limit was exceeded.
        buffered: usize,
        /// The limit.
        limit: usize,
    },
    /// The peer reset the session.
    #[error("peer reset the session")]
    Reset {
        /// The transport error.
        #[source]
        source: io::Error,
    },
    /// The peer ended its write side before an `expect` finished.
    #[error("peer closed after {received} bytes")]
    PeerClosed {
        /// Bytes that the action read before end of stream.
        received: usize,
    },
    /// A write failed because the peer closed the session.
    #[error("peer closed the session during a write")]
    BrokenPipe {
        /// The transport error.
        #[source]
        source: io::Error,
    },
    /// The TLS layer of the transport failed. Transports report TLS errors
    /// as [`io::ErrorKind::InvalidData`], as `tokio-rustls` does.
    #[error("tls failure")]
    Tls {
        /// The transport error.
        #[source]
        source: io::Error,
    },
    /// The engine stopped the session.
    #[error("session aborted by the engine")]
    Aborted,
    /// A template failed to render.
    #[error("render failed")]
    Render {
        /// The template error.
        #[source]
        source: TemplateError,
    },
    /// A rendered parameter is out of range.
    #[error("{0}")]
    Invalid(String),
    /// Any other transport error.
    #[error("io error")]
    Io {
        /// The transport error.
        #[source]
        source: io::Error,
    },
}

impl FailureKind {
    /// Returns the catalog reason of the failure.
    pub fn reason(&self) -> Reason {
        match self {
            FailureKind::OpenTimeout { .. } => Reason::ConnectTimeout,
            FailureKind::Refused { .. } => Reason::ConnectRefused,
            FailureKind::ExpectTimeout { .. } | FailureKind::DrainTimeout { .. } => {
                Reason::ExpectTimeout
            }
            FailureKind::PatternOverflow { .. } => Reason::PatternMismatch,
            FailureKind::Reset { .. } => Reason::UnexpectedReset,
            FailureKind::PeerClosed { .. } | FailureKind::BrokenPipe { .. } => Reason::PeerClosed,
            FailureKind::Tls { .. } => Reason::TlsFailure,
            FailureKind::Aborted => Reason::Aborted,
            FailureKind::Render { .. } | FailureKind::Invalid(_) | FailureKind::Io { .. } => {
                Reason::Internal
            }
        }
    }

    /// Classifies a transport error of an established session.
    pub(crate) fn from_io(source: io::Error) -> Self {
        match source.kind() {
            io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted => {
                FailureKind::Reset { source }
            }
            io::ErrorKind::BrokenPipe => FailureKind::BrokenPipe { source },
            io::ErrorKind::UnexpectedEof => FailureKind::PeerClosed { received: 0 },
            io::ErrorKind::InvalidData => FailureKind::Tls { source },
            _ => FailureKind::Io { source },
        }
    }

    /// Classifies a transport error of `connect` or `accept`.
    /// A transport that times out on its own reports the interpreter's
    /// open timeout, `timeout_ms`.
    pub(crate) fn from_open_io(source: io::Error, timeout_ms: u64) -> Self {
        match source.kind() {
            io::ErrorKind::ConnectionRefused => FailureKind::Refused { source },
            io::ErrorKind::TimedOut => FailureKind::OpenTimeout { timeout_ms },
            _ => FailureKind::from_io(source),
        }
    }
}

/// A failed session side, with the location of the failing action.
#[derive(Debug, thiserror::Error)]
#[error("flow {flow} {side} action {action_index} ({action_type}): {reason}")]
pub struct Failure {
    /// Flow id.
    pub flow: String,
    /// The side that failed.
    pub side: Side,
    /// Zero based index of the failing action in document order, with the
    /// opening `connect` or `accept` at index 0, as the `action_index` label
    /// of the metric catalog counts it.
    pub action_index: usize,
    /// The `action_type` label of the failing action.
    pub action_type: &'static str,
    /// The catalog reason.
    pub reason: Reason,
    /// The detail and its cause.
    #[source]
    pub kind: FailureKind,
}

impl Failure {
    /// Returns the failure label `extension_<kind>` when a module function
    /// failed, as `contracts/extension/README.md` names it.
    pub fn extension_label(&self) -> Option<String> {
        match &self.kind {
            FailureKind::Render {
                source: TemplateError::Function { kind, .. },
            } => Some(format!("extension_{kind}")),
            _ => None,
        }
    }
}
