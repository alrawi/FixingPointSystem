// SPDX-License-Identifier: GPL-3.0-only
//! Typed series sets with the `side` and `reason` labels of contract v0.2.
//!
//! Catalog 0.2.0 adds the `side` label, with the values `client` and `server`,
//! to every scenario level and every transport metric. It also adds the
//! failure reasons `datagram_rejected`, `template_error`, and
//! `extension_error`. [`Side`] and [`FailureReason`] name those label values
//! as types, and [`SessionCounters`] and [`TransportCounters`] register the
//! counters that a worker records for one side, so a caller cannot spell a
//! label wrongly or forget the side. The contract test `tests/series.rs`
//! compares both enums with the catalog file.

use crate::catalog::metric;
use crate::error::StatsError;
use crate::shard::{CounterId, Shard};

/// The side of a session, which is a value of the `side` label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    /// The side that opens the session.
    Client,
    /// The side that accepts the session.
    Server,
}

impl Side {
    /// Both sides in catalog order.
    pub const ALL: [Side; 2] = [Side::Client, Side::Server];

    /// Returns the catalog value of the `side` label.
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Client => "client",
            Side::Server => "server",
        }
    }
}

/// The failure reason of a session, which is a value of the `reason` label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FailureReason {
    /// `connect_timeout`.
    ConnectTimeout,
    /// `connect_refused`.
    ConnectRefused,
    /// `expect_timeout`.
    ExpectTimeout,
    /// `pattern_mismatch`.
    PatternMismatch,
    /// `unexpected_reset`.
    UnexpectedReset,
    /// `peer_closed`.
    PeerClosed,
    /// `tls_failure`.
    TlsFailure,
    /// `aborted`.
    Aborted,
    /// `internal`.
    Internal,
    /// `datagram_rejected`, a received UDP datagram broke the size limit or
    /// the stop rule of its `expect`.
    DatagramRejected,
    /// `template_error`, a template failed to render.
    TemplateError,
    /// `extension_error`, a module function failed.
    ExtensionError,
}

impl FailureReason {
    /// Every reason in catalog order.
    pub const ALL: [FailureReason; 12] = [
        FailureReason::ConnectTimeout,
        FailureReason::ConnectRefused,
        FailureReason::ExpectTimeout,
        FailureReason::PatternMismatch,
        FailureReason::UnexpectedReset,
        FailureReason::PeerClosed,
        FailureReason::TlsFailure,
        FailureReason::Aborted,
        FailureReason::Internal,
        FailureReason::DatagramRejected,
        FailureReason::TemplateError,
        FailureReason::ExtensionError,
    ];

    /// Returns the catalog value of the `reason` label.
    pub fn as_str(self) -> &'static str {
        match self {
            FailureReason::ConnectTimeout => "connect_timeout",
            FailureReason::ConnectRefused => "connect_refused",
            FailureReason::ExpectTimeout => "expect_timeout",
            FailureReason::PatternMismatch => "pattern_mismatch",
            FailureReason::UnexpectedReset => "unexpected_reset",
            FailureReason::PeerClosed => "peer_closed",
            FailureReason::TlsFailure => "tls_failure",
            FailureReason::Aborted => "aborted",
            FailureReason::Internal => "internal",
            FailureReason::DatagramRejected => "datagram_rejected",
            FailureReason::TemplateError => "template_error",
            FailureReason::ExtensionError => "extension_error",
        }
    }

    /// Returns the position of the reason in [`FailureReason::ALL`].
    fn index(self) -> usize {
        Self::ALL.iter().position(|r| *r == self).unwrap_or(0)
    }
}

/// The session counters of one scenario and one side within one shard.
///
/// [`SessionCounters::register`] registers `sessions.attempted`,
/// `sessions.established`, `sessions.succeeded`, `bytes.tx`, `bytes.rx`, and
/// one `sessions.failed` counter per [`FailureReason`], all labelled with the
/// side. Recording afterwards indexes vectors and allocates nothing.
#[derive(Debug, Clone)]
pub struct SessionCounters {
    side: Side,
    attempted: CounterId,
    established: CounterId,
    succeeded: CounterId,
    /// One counter per reason, in the order of [`FailureReason::ALL`].
    failed: Vec<CounterId>,
    bytes_tx: CounterId,
    bytes_rx: CounterId,
}

impl SessionCounters {
    /// Registers the counters for `application`, `scenario`, and `side`.
    pub fn register(
        shard: &mut Shard,
        application: &str,
        scenario: &str,
        side: Side,
    ) -> Result<Self, StatsError> {
        let labels = [
            ("application", application),
            ("scenario", scenario),
            ("side", side.as_str()),
        ];
        let mut failed = Vec::with_capacity(FailureReason::ALL.len());
        for reason in FailureReason::ALL {
            let l = [
                ("application", application),
                ("scenario", scenario),
                ("side", side.as_str()),
                ("reason", reason.as_str()),
            ];
            failed.push(shard.counter(metric::SESSIONS_FAILED, &l)?);
        }
        Ok(Self {
            side,
            attempted: shard.counter(metric::SESSIONS_ATTEMPTED, &labels)?,
            established: shard.counter(metric::SESSIONS_ESTABLISHED, &labels)?,
            succeeded: shard.counter(metric::SESSIONS_SUCCEEDED, &labels)?,
            failed,
            bytes_tx: shard.counter(metric::BYTES_TX, &labels)?,
            bytes_rx: shard.counter(metric::BYTES_RX, &labels)?,
        })
    }

    /// Returns the side of the counters.
    pub fn side(&self) -> Side {
        self.side
    }

    /// Counts a started session.
    #[inline]
    pub fn attempted(&self, shard: &mut Shard) {
        shard.inc(self.attempted);
    }

    /// Counts a session that reached the established state.
    #[inline]
    pub fn established(&self, shard: &mut Shard) {
        shard.inc(self.established);
    }

    /// Counts a session that finished every action.
    #[inline]
    pub fn succeeded(&self, shard: &mut Shard) {
        shard.inc(self.succeeded);
    }

    /// Counts a failed session under its reason.
    #[inline]
    pub fn failed(&self, shard: &mut Shard, reason: FailureReason) {
        if let Some(id) = self.failed.get(reason.index()) {
            shard.inc(*id);
        }
    }

    /// Adds the application bytes that the side sent and received.
    #[inline]
    pub fn add_bytes(&self, shard: &mut Shard, tx: u64, rx: u64) {
        shard.add(self.bytes_tx, tx);
        shard.add(self.bytes_rx, rx);
    }
}

/// The transport counters of one port and one side within one shard.
///
/// The set covers the plain `tcp.*` and `tls.*` counters of the catalog. The
/// alert counters carry the extra `alert` label, so a caller registers them
/// with [`Shard::counter`] and the labels `port`, `side`, and `alert`.
#[derive(Debug, Clone)]
pub struct TransportCounters {
    side: Side,
    tcp_handshakes: CounterId,
    tcp_handshake_failures: CounterId,
    tcp_retransmissions: CounterId,
    tcp_resets_tx: CounterId,
    tcp_resets_rx: CounterId,
    tcp_zero_window_events: CounterId,
    tls_handshakes: CounterId,
    tls_handshake_failures: CounterId,
    tls_resumptions: CounterId,
}

impl TransportCounters {
    /// Registers the counters for `port` and `side`.
    pub fn register(shard: &mut Shard, port: &str, side: Side) -> Result<Self, StatsError> {
        let l = [("port", port), ("side", side.as_str())];
        Ok(Self {
            side,
            tcp_handshakes: shard.counter(metric::TCP_HANDSHAKES, &l)?,
            tcp_handshake_failures: shard.counter(metric::TCP_HANDSHAKE_FAILURES, &l)?,
            tcp_retransmissions: shard.counter(metric::TCP_RETRANSMISSIONS, &l)?,
            tcp_resets_tx: shard.counter(metric::TCP_RESETS_TX, &l)?,
            tcp_resets_rx: shard.counter(metric::TCP_RESETS_RX, &l)?,
            tcp_zero_window_events: shard.counter(metric::TCP_ZERO_WINDOW_EVENTS, &l)?,
            tls_handshakes: shard.counter(metric::TLS_HANDSHAKES, &l)?,
            tls_handshake_failures: shard.counter(metric::TLS_HANDSHAKE_FAILURES, &l)?,
            tls_resumptions: shard.counter(metric::TLS_RESUMPTIONS, &l)?,
        })
    }

    /// Returns the side of the counters.
    pub fn side(&self) -> Side {
        self.side
    }

    /// Counts a completed TCP handshake.
    #[inline]
    pub fn tcp_handshake(&self, shard: &mut Shard) {
        shard.inc(self.tcp_handshakes);
    }

    /// Counts a failed TCP handshake.
    #[inline]
    pub fn tcp_handshake_failure(&self, shard: &mut Shard) {
        shard.inc(self.tcp_handshake_failures);
    }

    /// Adds retransmitted TCP packets.
    #[inline]
    pub fn tcp_retransmissions(&self, shard: &mut Shard, packets: u64) {
        shard.add(self.tcp_retransmissions, packets);
    }

    /// Counts a TCP reset that the side sent.
    #[inline]
    pub fn tcp_reset_tx(&self, shard: &mut Shard) {
        shard.inc(self.tcp_resets_tx);
    }

    /// Counts a TCP reset that the side received.
    #[inline]
    pub fn tcp_reset_rx(&self, shard: &mut Shard) {
        shard.inc(self.tcp_resets_rx);
    }

    /// Counts a zero window event.
    #[inline]
    pub fn tcp_zero_window(&self, shard: &mut Shard) {
        shard.inc(self.tcp_zero_window_events);
    }

    /// Counts a completed TLS handshake.
    #[inline]
    pub fn tls_handshake(&self, shard: &mut Shard) {
        shard.inc(self.tls_handshakes);
    }

    /// Counts a failed TLS handshake.
    #[inline]
    pub fn tls_handshake_failure(&self, shard: &mut Shard) {
        shard.inc(self.tls_handshake_failures);
    }

    /// Counts a resumed TLS session.
    #[inline]
    pub fn tls_resumption(&self, shard: &mut Shard) {
        shard.inc(self.tls_resumptions);
    }
}
