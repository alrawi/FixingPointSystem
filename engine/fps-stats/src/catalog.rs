// SPDX-License-Identifier: GPL-3.0-only
//! The metric catalog of contract v0.2 as a static table.
//!
//! The table mirrors `contracts/metrics/catalog.yaml`. The contract test
//! `tests/catalog.rs` reads the YAML file and compares every name, type, unit,
//! label list, and label value set with this file, so the two cannot drift.

/// The type of a metric, as the `type` field of the catalog names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricKind {
    /// A monotonic count, serialized as `CounterValue.count`.
    Counter,
    /// An instantaneous value, serialized as `CounterValue.gauge`.
    Gauge,
    /// A distribution, serialized as `HistogramValue.hdr_v2`.
    Histogram,
}

impl MetricKind {
    /// Returns the catalog spelling of the kind, for example `counter`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
            Self::Histogram => "histogram",
        }
    }
}

/// One metric of the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricDef {
    /// The catalog name, for example `sessions.attempted`.
    pub name: &'static str,
    /// The metric type.
    pub kind: MetricKind,
    /// The catalog unit, for example `us` or `sessions`.
    pub unit: &'static str,
    /// The labels that may qualify a value of the metric, in catalog order.
    pub labels: &'static [&'static str],
}

impl MetricDef {
    /// Reports whether shard values of this metric merge by summing.
    ///
    /// Counters and histograms always merge by summing. A gauge merges by
    /// summing only when its unit counts something that workers own
    /// separately, such as open sessions or bits per second. Ratios and
    /// percentages are derived after the merge, so a shard cannot record them.
    pub fn merges_by_sum(&self) -> bool {
        match self.kind {
            MetricKind::Counter | MetricKind::Histogram => true,
            MetricKind::Gauge => !matches!(self.unit, "ratio" | "percent" | "percentage_points"),
        }
    }
}

/// Identifies one catalog metric by its position in [`CATALOG`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MetricId(pub(crate) u16);

impl MetricId {
    /// Returns the catalog entry of the metric.
    pub fn def(self) -> &'static MetricDef {
        // Every MetricId is either a constant of `metric` or comes from
        // `MetricId::from_name`, so the index is always in range.
        CATALOG.get(usize::from(self.0)).unwrap_or(&CATALOG[0])
    }

    /// Returns the catalog name of the metric.
    pub fn name(self) -> &'static str {
        self.def().name
    }

    /// Looks a metric up by its catalog name.
    pub fn from_name(name: &str) -> Option<Self> {
        CATALOG
            .iter()
            .position(|m| m.name == name)
            .and_then(|i| u16::try_from(i).ok())
            .map(MetricId)
    }

    /// Iterates over every metric of the catalog in catalog order.
    pub fn all() -> impl Iterator<Item = MetricId> {
        (0..CATALOG.len()).filter_map(|i| u16::try_from(i).ok().map(MetricId))
    }
}

/// One constant per catalog metric, named after the metric with dots
/// replaced by underscores.
pub mod metric {
    use super::MetricId;

    /// `mix.share_pct`, a gauge in `percent`.
    pub const MIX_SHARE_PCT: MetricId = MetricId(0);
    /// `mix.abs_error_pp`, a gauge in `percentage_points`.
    pub const MIX_ABS_ERROR_PP: MetricId = MetricId(1);
    /// `mix.max_abs_error_pp`, a gauge in `percentage_points`.
    pub const MIX_MAX_ABS_ERROR_PP: MetricId = MetricId(2);
    /// `realism.conformance_ratio`, a gauge in `ratio`.
    pub const REALISM_CONFORMANCE_RATIO: MetricId = MetricId(3);
    /// `realism.ks_statistic`, a gauge in `ratio`.
    pub const REALISM_KS_STATISTIC: MetricId = MetricId(4);
    /// `realism.ks_p_value`, a gauge in `ratio`.
    pub const REALISM_KS_P_VALUE: MetricId = MetricId(5);
    /// `sessions.attempted`, a counter in `sessions`.
    pub const SESSIONS_ATTEMPTED: MetricId = MetricId(6);
    /// `sessions.established`, a counter in `sessions`.
    pub const SESSIONS_ESTABLISHED: MetricId = MetricId(7);
    /// `sessions.succeeded`, a counter in `sessions`.
    pub const SESSIONS_SUCCEEDED: MetricId = MetricId(8);
    /// `sessions.failed`, a counter in `sessions`.
    pub const SESSIONS_FAILED: MetricId = MetricId(9);
    /// `sessions.failed_ratio`, a gauge in `ratio`.
    pub const SESSIONS_FAILED_RATIO: MetricId = MetricId(10);
    /// `sessions.concurrent`, a gauge in `sessions`.
    pub const SESSIONS_CONCURRENT: MetricId = MetricId(11);
    /// `sessions.rate`, a gauge in `sessions_per_second`.
    pub const SESSIONS_RATE: MetricId = MetricId(12);
    /// `sessions.response_time`, a histogram in `us`.
    pub const SESSIONS_RESPONSE_TIME: MetricId = MetricId(13);
    /// `sessions.duration`, a histogram in `us`.
    pub const SESSIONS_DURATION: MetricId = MetricId(14);
    /// `transactions.attempted`, a counter in `transactions`.
    pub const TRANSACTIONS_ATTEMPTED: MetricId = MetricId(15);
    /// `transactions.succeeded`, a counter in `transactions`.
    pub const TRANSACTIONS_SUCCEEDED: MetricId = MetricId(16);
    /// `bytes.tx`, a counter in `bytes`.
    pub const BYTES_TX: MetricId = MetricId(17);
    /// `bytes.rx`, a counter in `bytes`.
    pub const BYTES_RX: MetricId = MetricId(18);
    /// `throughput.bps`, a gauge in `bits_per_second`.
    pub const THROUGHPUT_BPS: MetricId = MetricId(19);
    /// `actions.expect_timeouts`, a counter in `events`.
    pub const ACTIONS_EXPECT_TIMEOUTS: MetricId = MetricId(20);
    /// `actions.pattern_mismatches`, a counter in `events`.
    pub const ACTIONS_PATTERN_MISMATCHES: MetricId = MetricId(21);
    /// `actions.unexpected_resets`, a counter in `events`.
    pub const ACTIONS_UNEXPECTED_RESETS: MetricId = MetricId(22);
    /// `actions.latency`, a histogram in `us`.
    pub const ACTIONS_LATENCY: MetricId = MetricId(23);
    /// `tcp.handshakes`, a counter in `events`.
    pub const TCP_HANDSHAKES: MetricId = MetricId(24);
    /// `tcp.handshake_failures`, a counter in `events`.
    pub const TCP_HANDSHAKE_FAILURES: MetricId = MetricId(25);
    /// `tcp.retransmissions`, a counter in `packets`.
    pub const TCP_RETRANSMISSIONS: MetricId = MetricId(26);
    /// `tcp.resets.tx`, a counter in `packets`.
    pub const TCP_RESETS_TX: MetricId = MetricId(27);
    /// `tcp.resets.rx`, a counter in `packets`.
    pub const TCP_RESETS_RX: MetricId = MetricId(28);
    /// `tcp.zero_window_events`, a counter in `events`.
    pub const TCP_ZERO_WINDOW_EVENTS: MetricId = MetricId(29);
    /// `tcp.rtt`, a histogram in `us`.
    pub const TCP_RTT: MetricId = MetricId(30);
    /// `tls.handshakes`, a counter in `events`.
    pub const TLS_HANDSHAKES: MetricId = MetricId(31);
    /// `tls.handshake_failures`, a counter in `events`.
    pub const TLS_HANDSHAKE_FAILURES: MetricId = MetricId(32);
    /// `tls.alerts.tx`, a counter in `events`.
    pub const TLS_ALERTS_TX: MetricId = MetricId(33);
    /// `tls.alerts.rx`, a counter in `events`.
    pub const TLS_ALERTS_RX: MetricId = MetricId(34);
    /// `tls.resumptions`, a counter in `events`.
    pub const TLS_RESUMPTIONS: MetricId = MetricId(35);
    /// `tls.resumption_ratio`, a gauge in `ratio`.
    pub const TLS_RESUMPTION_RATIO: MetricId = MetricId(36);
    /// `tls.handshake_time`, a histogram in `us`.
    pub const TLS_HANDSHAKE_TIME: MetricId = MetricId(37);
    /// `port.packets.tx`, a counter in `packets`.
    pub const PORT_PACKETS_TX: MetricId = MetricId(38);
    /// `port.packets.rx`, a counter in `packets`.
    pub const PORT_PACKETS_RX: MetricId = MetricId(39);
    /// `port.bytes.tx`, a counter in `bytes`.
    pub const PORT_BYTES_TX: MetricId = MetricId(40);
    /// `port.bytes.rx`, a counter in `bytes`.
    pub const PORT_BYTES_RX: MetricId = MetricId(41);
    /// `port.drops`, a counter in `packets`.
    pub const PORT_DROPS: MetricId = MetricId(42);
    /// `port.throughput.tx_bps`, a gauge in `bits_per_second`.
    pub const PORT_THROUGHPUT_TX_BPS: MetricId = MetricId(43);
    /// `port.throughput.rx_bps`, a gauge in `bits_per_second`.
    pub const PORT_THROUGHPUT_RX_BPS: MetricId = MetricId(44);
    /// `host.arp_failures`, a counter in `events`.
    pub const HOST_ARP_FAILURES: MetricId = MetricId(45);
    /// `host.nd_failures`, a counter in `events`.
    pub const HOST_ND_FAILURES: MetricId = MetricId(46);
    /// `host.active`, a gauge in `hosts`.
    pub const HOST_ACTIVE: MetricId = MetricId(47);
}

/// Every metric of the catalog, in catalog order.
pub const CATALOG: &[MetricDef] = &[
    MetricDef {
        name: "mix.share_pct",
        kind: MetricKind::Gauge,
        unit: "percent",
        labels: &["application"],
    },
    MetricDef {
        name: "mix.abs_error_pp",
        kind: MetricKind::Gauge,
        unit: "percentage_points",
        labels: &["application"],
    },
    MetricDef {
        name: "mix.max_abs_error_pp",
        kind: MetricKind::Gauge,
        unit: "percentage_points",
        labels: &[],
    },
    MetricDef {
        name: "realism.conformance_ratio",
        kind: MetricKind::Gauge,
        unit: "ratio",
        labels: &["application", "classifier"],
    },
    MetricDef {
        name: "realism.ks_statistic",
        kind: MetricKind::Gauge,
        unit: "ratio",
        labels: &["application", "property"],
    },
    MetricDef {
        name: "realism.ks_p_value",
        kind: MetricKind::Gauge,
        unit: "ratio",
        labels: &["application", "property"],
    },
    MetricDef {
        name: "sessions.attempted",
        kind: MetricKind::Counter,
        unit: "sessions",
        labels: &["application", "scenario", "side"],
    },
    MetricDef {
        name: "sessions.established",
        kind: MetricKind::Counter,
        unit: "sessions",
        labels: &["application", "scenario", "side"],
    },
    MetricDef {
        name: "sessions.succeeded",
        kind: MetricKind::Counter,
        unit: "sessions",
        labels: &["application", "scenario", "side"],
    },
    MetricDef {
        name: "sessions.failed",
        kind: MetricKind::Counter,
        unit: "sessions",
        labels: &["application", "scenario", "side", "reason"],
    },
    MetricDef {
        name: "sessions.failed_ratio",
        kind: MetricKind::Gauge,
        unit: "ratio",
        labels: &["application", "scenario", "side"],
    },
    MetricDef {
        name: "sessions.concurrent",
        kind: MetricKind::Gauge,
        unit: "sessions",
        labels: &["application", "scenario", "side"],
    },
    MetricDef {
        name: "sessions.rate",
        kind: MetricKind::Gauge,
        unit: "sessions_per_second",
        labels: &["application", "scenario", "side"],
    },
    MetricDef {
        name: "sessions.response_time",
        kind: MetricKind::Histogram,
        unit: "us",
        labels: &["application", "scenario", "side"],
    },
    MetricDef {
        name: "sessions.duration",
        kind: MetricKind::Histogram,
        unit: "us",
        labels: &["application", "scenario", "side"],
    },
    MetricDef {
        name: "transactions.attempted",
        kind: MetricKind::Counter,
        unit: "transactions",
        labels: &["application", "scenario", "side"],
    },
    MetricDef {
        name: "transactions.succeeded",
        kind: MetricKind::Counter,
        unit: "transactions",
        labels: &["application", "scenario", "side"],
    },
    MetricDef {
        name: "bytes.tx",
        kind: MetricKind::Counter,
        unit: "bytes",
        labels: &["application", "scenario", "side"],
    },
    MetricDef {
        name: "bytes.rx",
        kind: MetricKind::Counter,
        unit: "bytes",
        labels: &["application", "scenario", "side"],
    },
    MetricDef {
        name: "throughput.bps",
        kind: MetricKind::Gauge,
        unit: "bits_per_second",
        labels: &["application", "scenario", "side"],
    },
    MetricDef {
        name: "actions.expect_timeouts",
        kind: MetricKind::Counter,
        unit: "events",
        labels: &["application", "scenario", "flow", "side", "action_index"],
    },
    MetricDef {
        name: "actions.pattern_mismatches",
        kind: MetricKind::Counter,
        unit: "events",
        labels: &["application", "scenario", "flow", "side", "action_index"],
    },
    MetricDef {
        name: "actions.unexpected_resets",
        kind: MetricKind::Counter,
        unit: "events",
        labels: &["application", "scenario", "flow", "side", "action_index"],
    },
    MetricDef {
        name: "actions.latency",
        kind: MetricKind::Histogram,
        unit: "us",
        labels: &[
            "application",
            "scenario",
            "flow",
            "side",
            "action_index",
            "action_type",
        ],
    },
    MetricDef {
        name: "tcp.handshakes",
        kind: MetricKind::Counter,
        unit: "events",
        labels: &["port", "side"],
    },
    MetricDef {
        name: "tcp.handshake_failures",
        kind: MetricKind::Counter,
        unit: "events",
        labels: &["port", "side"],
    },
    MetricDef {
        name: "tcp.retransmissions",
        kind: MetricKind::Counter,
        unit: "packets",
        labels: &["port", "side"],
    },
    MetricDef {
        name: "tcp.resets.tx",
        kind: MetricKind::Counter,
        unit: "packets",
        labels: &["port", "side"],
    },
    MetricDef {
        name: "tcp.resets.rx",
        kind: MetricKind::Counter,
        unit: "packets",
        labels: &["port", "side"],
    },
    MetricDef {
        name: "tcp.zero_window_events",
        kind: MetricKind::Counter,
        unit: "events",
        labels: &["port", "side"],
    },
    MetricDef {
        name: "tcp.rtt",
        kind: MetricKind::Histogram,
        unit: "us",
        labels: &["port", "side"],
    },
    MetricDef {
        name: "tls.handshakes",
        kind: MetricKind::Counter,
        unit: "events",
        labels: &["port", "side"],
    },
    MetricDef {
        name: "tls.handshake_failures",
        kind: MetricKind::Counter,
        unit: "events",
        labels: &["port", "side"],
    },
    MetricDef {
        name: "tls.alerts.tx",
        kind: MetricKind::Counter,
        unit: "events",
        labels: &["port", "side", "alert"],
    },
    MetricDef {
        name: "tls.alerts.rx",
        kind: MetricKind::Counter,
        unit: "events",
        labels: &["port", "side", "alert"],
    },
    MetricDef {
        name: "tls.resumptions",
        kind: MetricKind::Counter,
        unit: "events",
        labels: &["port", "side"],
    },
    MetricDef {
        name: "tls.resumption_ratio",
        kind: MetricKind::Gauge,
        unit: "ratio",
        labels: &["port", "side"],
    },
    MetricDef {
        name: "tls.handshake_time",
        kind: MetricKind::Histogram,
        unit: "us",
        labels: &["port", "side"],
    },
    MetricDef {
        name: "port.packets.tx",
        kind: MetricKind::Counter,
        unit: "packets",
        labels: &["port"],
    },
    MetricDef {
        name: "port.packets.rx",
        kind: MetricKind::Counter,
        unit: "packets",
        labels: &["port"],
    },
    MetricDef {
        name: "port.bytes.tx",
        kind: MetricKind::Counter,
        unit: "bytes",
        labels: &["port"],
    },
    MetricDef {
        name: "port.bytes.rx",
        kind: MetricKind::Counter,
        unit: "bytes",
        labels: &["port"],
    },
    MetricDef {
        name: "port.drops",
        kind: MetricKind::Counter,
        unit: "packets",
        labels: &["port"],
    },
    MetricDef {
        name: "port.throughput.tx_bps",
        kind: MetricKind::Gauge,
        unit: "bits_per_second",
        labels: &["port"],
    },
    MetricDef {
        name: "port.throughput.rx_bps",
        kind: MetricKind::Gauge,
        unit: "bits_per_second",
        labels: &["port"],
    },
    MetricDef {
        name: "host.arp_failures",
        kind: MetricKind::Counter,
        unit: "events",
        labels: &["port", "host_pool"],
    },
    MetricDef {
        name: "host.nd_failures",
        kind: MetricKind::Counter,
        unit: "events",
        labels: &["port", "host_pool"],
    },
    MetricDef {
        name: "host.active",
        kind: MetricKind::Gauge,
        unit: "hosts",
        labels: &["port", "host_pool"],
    },
];

/// The labels whose values the catalog enumerates, with those values.
/// Labels that the catalog does not list here take any value.
pub const LABEL_VALUES: &[(&str, &[&str])] = &[
    (
        "action_type",
        &[
            "connect", "accept", "send", "expect", "delay", "set", "loop", "branch", "spawn",
            "close", "raw",
        ],
    ),
    ("side", &["client", "server"]),
    (
        "reason",
        &[
            "connect_timeout",
            "connect_refused",
            "expect_timeout",
            "pattern_mismatch",
            "unexpected_reset",
            "peer_closed",
            "tls_failure",
            "aborted",
            "internal",
            "datagram_rejected",
            "template_error",
            "extension_error",
        ],
    ),
    ("classifier", &["ndpi", "zeek"]),
    (
        "property",
        &[
            "session_bytes",
            "request_bytes",
            "response_bytes",
            "inter_arrival",
            "session_duration",
        ],
    ),
];

/// Every label name of the catalog, in catalog order.
pub const LABEL_NAMES: &[&str] = &[
    "application",
    "scenario",
    "flow",
    "action_index",
    "action_type",
    "side",
    "reason",
    "classifier",
    "property",
    "alert",
    "port",
    "host_pool",
];

/// Returns the enumerated values of a label, or `None` when any value is allowed.
pub fn label_values(label: &str) -> Option<&'static [&'static str]> {
    LABEL_VALUES
        .iter()
        .find(|(name, _)| *name == label)
        .map(|(_, values)| *values)
}
