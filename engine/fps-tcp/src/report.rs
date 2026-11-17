// SPDX-License-Identifier: GPL-3.0-only
//! Reports of a harness run, as text for people and as `fps-bench/v1` JSON.
//!
//! The JSON follows the format of `bench/README.md`, so `bench/compare.py`
//! can compare two runs of the suite `tcp-stack` once a baseline exists. Unknown
//! top level keys are ignored there, and the report uses that room for the
//! configuration and the host.

use std::fmt::Write as _;

use crate::sysinfo::HostInfo;
use crate::workload::{RateReport, ScaleReport, ThroughputReport};

/// The direction in which a metric improves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Better {
    /// A larger value is better.
    Higher,
    /// A smaller value is better.
    Lower,
}

impl Better {
    fn as_str(self) -> &'static str {
        match self {
            Better::Higher => "higher",
            Better::Lower => "lower",
        }
    }
}

/// One named value of a report.
#[derive(Clone, Debug, PartialEq)]
pub struct Metric {
    /// The dotted name, which starts with the stack key.
    pub name: String,
    /// The value.
    pub value: f64,
    /// The direction in which the value improves.
    pub better: Better,
    /// The unit, for people.
    pub unit: &'static str,
}

/// Everything that one run of the harness measured.
#[derive(Clone, Debug)]
pub struct BenchReport {
    /// The short name of the stack, used in metric names, such as `kernel`.
    pub stack_key: String,
    /// The long name of the stack, with its version.
    pub stack_name: String,
    /// The facts about the stack and its link, from [`crate::TcpStack::notes`].
    pub stack_notes: Vec<String>,
    /// The label of the host or profile that the run belongs to.
    pub label: String,
    /// The facts about the host.
    pub host: HostInfo,
    /// The settings of the run, as name and value pairs.
    pub config: Vec<(String, String)>,
    /// The session rate result, when the workload ran.
    pub rate: Option<RateReport>,
    /// The throughput result, when the workload ran.
    pub throughput: Option<ThroughputReport>,
    /// The scaling result, when the workload ran.
    pub scale: Option<ScaleReport>,
    /// When the run finished, in seconds since the Unix epoch.
    pub finished_unix_secs: u64,
}

impl BenchReport {
    /// Lists every metric of the report in a stable order.
    pub fn metrics(&self) -> Vec<Metric> {
        let key = &self.stack_key;
        let mut out = Vec::new();
        let mut push = |name: String, value: f64, better: Better, unit: &'static str| {
            out.push(Metric {
                name,
                value,
                better,
                unit,
            });
        };
        if let Some(rate) = &self.rate {
            push(
                format!("{key}.rate.sessions_per_s"),
                rate.sessions_per_s,
                Better::Higher,
                "sessions/s",
            );
            push(
                format!("{key}.rate.latency_p50_us"),
                rate.latency.p50_us as f64,
                Better::Lower,
                "us",
            );
            push(
                format!("{key}.rate.latency_p99_us"),
                rate.latency.p99_us as f64,
                Better::Lower,
                "us",
            );
            push(
                format!("{key}.rate.sessions_failed"),
                rate.errors as f64,
                Better::Lower,
                "sessions",
            );
        }
        if let Some(throughput) = &self.throughput {
            push(
                format!("{key}.throughput.gbit_per_s"),
                throughput.gbit_per_s,
                Better::Higher,
                "Gbit/s",
            );
        }
        if let Some(scale) = &self.scale {
            push(
                format!("{key}.scale.max_sockets"),
                scale.max_sockets() as f64,
                Better::Higher,
                "sockets",
            );
            for step in scale.steps.iter().filter(|s| s.reached) {
                let at = step.target_sockets;
                push(
                    format!("{key}.scale.{at}.setup_sockets_per_s"),
                    step.setup_sockets_per_s,
                    Better::Higher,
                    "sockets/s",
                );
                if let Some(bytes) = step.bytes_per_socket {
                    push(
                        format!("{key}.scale.{at}.bytes_per_socket"),
                        bytes,
                        Better::Lower,
                        "bytes",
                    );
                }
                push(
                    format!("{key}.scale.{at}.idle_poll_us"),
                    step.idle_poll_us,
                    Better::Lower,
                    "us",
                );
                push(
                    format!("{key}.scale.{at}.probe_sessions_per_s"),
                    step.probe_sessions_per_s,
                    Better::Higher,
                    "sessions/s",
                );
                push(
                    format!("{key}.scale.{at}.probe_latency_p99_us"),
                    step.probe_latency.p99_us as f64,
                    Better::Lower,
                    "us",
                );
            }
        }
        out
    }

    /// Renders the report as an `fps-bench/v1` document.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        out.push_str("{\n  \"format\": \"fps-bench/v1\",\n  \"suite\": \"tcp-stack\",\n");
        let _ = writeln!(
            out,
            "  \"recorded\": {},",
            json_string(&utc_timestamp(self.finished_unix_secs))
        );
        let _ = writeln!(
            out,
            "  \"source\": {},",
            json_string(&format!("{} on {}", self.stack_name, self.label))
        );
        let mem = self
            .host
            .mem_total_bytes
            .map_or_else(|| "null".to_string(), |m| m.to_string());
        let _ = writeln!(
            out,
            "  \"host\": {{\"cpu\": {}, \"vcpus\": {}, \"kernel\": {}, \"mem_total_bytes\": {mem}}},",
            json_string(&self.host.cpu),
            self.host.vcpus,
            json_string(&self.host.kernel),
        );
        out.push_str("  \"config\": {");
        for (index, (name, value)) in self.config.iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            let _ = write!(out, "{}: {}", json_string(name), json_string(value));
        }
        out.push_str("},\n  \"notes\": [");
        for (index, note) in self.notes().iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            out.push_str(&json_string(note));
        }
        out.push_str("],\n  \"metrics\": {\n");
        let metrics = self.metrics();
        for (index, metric) in metrics.iter().enumerate() {
            let comma = if index + 1 < metrics.len() { "," } else { "" };
            let _ = writeln!(
                out,
                "    {}: {{\"value\": {}, \"better\": \"{}\", \"unit\": \"{}\"}}{comma}",
                json_string(&metric.name),
                json_number(metric.value),
                metric.better.as_str(),
                metric.unit,
            );
        }
        out.push_str("  }\n}\n");
        out
    }

    /// Lists the facts that a reader needs to interpret the numbers.
    fn notes(&self) -> Vec<String> {
        let mut notes = self.stack_notes.clone();
        notes.push(
            "sockets counts both hosts of a pair and includes the listening sockets".to_string(),
        );
        if let Some(scale) = &self.scale {
            if let Some(reason) = &scale.stopped {
                notes.push(reason.clone());
            }
        }
        notes
    }

    /// Renders the report for people.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{} on {}", self.stack_name, self.label);
        let _ = writeln!(
            out,
            "host: {}, {} vcpus, kernel {}",
            self.host.cpu, self.host.vcpus, self.host.kernel
        );
        for (name, value) in &self.config {
            let _ = writeln!(out, "  {name} = {value}");
        }
        if let Some(rate) = &self.rate {
            let _ = writeln!(
                out,
                "sessions per second: {:.0} ({} sessions in {:.2} s, {} failed, p50 {} us, p99 {} us)",
                rate.sessions_per_s,
                rate.sessions,
                rate.elapsed_secs,
                rate.errors,
                rate.latency.p50_us,
                rate.latency.p99_us
            );
        }
        if let Some(t) = &self.throughput {
            let _ = writeln!(
                out,
                "throughput: {:.3} Gbit/s ({} bytes in {:.2} s)",
                t.gbit_per_s, t.bytes, t.elapsed_secs
            );
        }
        if let Some(scale) = &self.scale {
            let _ = writeln!(out, "socket scaling over {} shards:", scale.shards);
            let _ = writeln!(
                out,
                "  {:>10} {:>10} {:>9} {:>12} {:>9} {:>12} {:>12} {:>9}",
                "target",
                "sockets",
                "setup s",
                "sockets/s",
                "B/socket",
                "idle poll us",
                "probe /s",
                "probe p99"
            );
            for s in &scale.steps {
                let bytes = s
                    .bytes_per_socket
                    .map_or_else(|| "n/a".to_string(), |b| format!("{b:.0}"));
                let _ = writeln!(
                    out,
                    "  {:>10} {:>10} {:>9.2} {:>12.0} {:>9} {:>12.0} {:>12.0} {:>9}{}",
                    s.target_sockets,
                    s.sockets,
                    s.setup_secs,
                    s.setup_sockets_per_s,
                    bytes,
                    s.idle_poll_us,
                    s.probe_sessions_per_s,
                    s.probe_latency.p99_us,
                    if s.reached { "" } else { "  (not reached)" }
                );
            }
            if let Some(reason) = &scale.stopped {
                let _ = writeln!(out, "  stopped: {reason}");
            }
        }
        out
    }
}

fn json_number(value: f64) -> String {
    if value.is_finite() {
        format!("{value}")
    } else {
        "0".to_string()
    }
}

fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Formats seconds since the Unix epoch as an ISO 8601 UTC timestamp.
pub fn utc_timestamp(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let rest = unix_secs % 86_400;
    // Civil date from a day count, after Howard Hinnant's algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workload::{LatencySummary, ScaleStep};

    fn host() -> HostInfo {
        HostInfo {
            cpu: "Test \"CPU\"".into(),
            vcpus: 2,
            kernel: "6.1".into(),
            mem_total_bytes: Some(1 << 30),
        }
    }

    #[test]
    fn timestamps_match_known_dates() {
        assert_eq!(utc_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_timestamp(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(utc_timestamp(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(json_string("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
        assert_eq!(json_string("\u{1}"), "\"\\u0001\"");
    }

    #[test]
    fn non_finite_numbers_become_zero() {
        assert_eq!(json_number(f64::NAN), "0");
        assert_eq!(json_number(1.5), "1.5");
    }

    fn report() -> BenchReport {
        BenchReport {
            stack_key: "kernel".into(),
            stack_name: "kernel loopback TCP".into(),
            stack_notes: vec!["a note of the stack".into()],
            label: "unit".into(),
            host: host(),
            config: vec![("workload".into(), "all".into())],
            rate: Some(RateReport {
                sessions: 100,
                errors: 0,
                elapsed_secs: 1.0,
                sessions_per_s: 100.0,
                latency: LatencySummary {
                    p50_us: 10,
                    p99_us: 20,
                    max_us: 30,
                },
            }),
            throughput: Some(ThroughputReport {
                bytes: 1000,
                elapsed_secs: 1.0,
                gbit_per_s: 2.5,
            }),
            scale: Some(ScaleReport {
                shards: 1,
                steps: vec![
                    ScaleStep {
                        target_sockets: 1000,
                        sockets: 1004,
                        reached: true,
                        setup_secs: 0.5,
                        setup_sockets_per_s: 2000.0,
                        rss_bytes: Some(10),
                        bytes_per_socket: Some(900.0),
                        idle_poll_us: 50.0,
                        probe_sessions_per_s: 400.0,
                        probe_latency: LatencySummary::default(),
                    },
                    ScaleStep {
                        target_sockets: 2000,
                        sockets: 1500,
                        reached: false,
                        setup_secs: 9.0,
                        setup_sockets_per_s: 10.0,
                        rss_bytes: None,
                        bytes_per_socket: None,
                        idle_poll_us: 0.0,
                        probe_sessions_per_s: 0.0,
                        probe_latency: LatencySummary::default(),
                    },
                ],
                stopped: Some("step 2000 stopped".into()),
            }),
            finished_unix_secs: 1_700_000_000,
        }
    }

    #[test]
    fn metrics_name_every_measure_and_skip_unreached_steps() {
        let names: Vec<String> = report().metrics().into_iter().map(|m| m.name).collect();
        for expected in [
            "kernel.rate.sessions_per_s",
            "kernel.rate.latency_p99_us",
            "kernel.throughput.gbit_per_s",
            "kernel.scale.max_sockets",
            "kernel.scale.1000.bytes_per_socket",
            "kernel.scale.1000.idle_poll_us",
        ] {
            assert!(names.iter().any(|n| n == expected), "missing {expected}");
        }
        assert!(!names.iter().any(|n| n.contains(".2000.")));
    }

    #[test]
    fn json_has_the_bench_format_and_balanced_braces() {
        let json = report().to_json();
        assert!(json.contains("\"format\": \"fps-bench/v1\""));
        assert!(json.contains("\"suite\": \"tcp-stack\""));
        assert!(json.contains("\"recorded\": \"2023-11-14T22:13:20Z\""));
        assert!(json.contains("Test \\\"CPU\\\""));
        assert!(json.contains("\"better\": \"lower\""));
        let opens = json.matches('{').count();
        let closes = json.matches('}').count();
        assert_eq!(opens, closes);
    }

    #[test]
    fn text_lists_the_three_measures() {
        let text = report().to_text();
        assert!(text.contains("sessions per second: 100"));
        assert!(text.contains("throughput: 2.500 Gbit/s"));
        assert!(text.contains("(not reached)"));
    }
}
