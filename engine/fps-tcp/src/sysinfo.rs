// SPDX-License-Identifier: GPL-3.0-only
//! Facts about the host that the reports record.
//!
//! The functions read `/proc` and return `None` or a placeholder on a system
//! without it, so the harness runs everywhere and the reports say what is
//! unknown.

use std::fs;

/// Returns the resident set size of this process in bytes.
pub fn resident_bytes() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib * 1024)
}

/// The host facts that a report carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostInfo {
    /// The processor model.
    pub cpu: String,
    /// The number of logical processors that the process may use.
    pub vcpus: usize,
    /// The kernel release.
    pub kernel: String,
    /// The memory of the host in bytes, when it is known.
    pub mem_total_bytes: Option<u64>,
}

impl HostInfo {
    /// Reads the facts of the current host.
    pub fn detect() -> Self {
        let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        let cpu = parse_cpu_model(&cpuinfo).unwrap_or_else(|| "unknown".into());
        let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "unknown".into());
        let vcpus = std::thread::available_parallelism().map_or(1, usize::from);
        let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
        Self {
            cpu,
            vcpus,
            kernel,
            mem_total_bytes: parse_mem_total(&meminfo),
        }
    }
}

fn parse_cpu_model(cpuinfo: &str) -> Option<String> {
    let line = cpuinfo.lines().find(|l| l.starts_with("model name"))?;
    let (_, model) = line.split_once(':')?;
    Some(model.trim().to_string())
}

fn parse_mem_total(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_model_comes_from_the_first_model_name_line() {
        let text = "processor\t: 0\nmodel name\t: Test CPU @ 2.00GHz\nmodel name\t: other\n";
        assert_eq!(parse_cpu_model(text).as_deref(), Some("Test CPU @ 2.00GHz"));
        assert_eq!(parse_cpu_model("nothing here"), None);
    }

    #[test]
    fn memory_total_is_converted_from_kibibytes() {
        assert_eq!(
            parse_mem_total("MemTotal:       2048 kB\nMemFree: 1 kB\n"),
            Some(2048 * 1024)
        );
        assert_eq!(parse_mem_total(""), None);
    }

    #[test]
    fn detect_reports_at_least_one_cpu() {
        assert!(HostInfo::detect().vcpus >= 1);
    }
}
