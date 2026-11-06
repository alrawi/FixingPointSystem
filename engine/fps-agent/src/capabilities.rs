// SPDX-License-Identifier: GPL-3.0-only
//! Host capability detection.
//!
//! [`detect`] reads `/proc` and `/sys` under a [`SystemRoot`] and returns
//! [`HostCapabilities`], which holds every fact that detection found.
//! [`HostCapabilities::to_capabilities`] then fills the proto mirror
//! [`Capabilities`]. Some facts, namely the NUMA nodes, the kernel version,
//! and the AF_XDP evidence, have no field in the `Capabilities` message of
//! contract version 0.2.0, so they stay on [`HostCapabilities`] until the
//! contract adds them. AF_XDP support reaches the message as
//! [`Backend::AfXdp`] in `backends`.
//!
//! Detection reads these files, each relative to the root.
//!
//! | Fact | Source | Fallback |
//! |---|---|---|
//! | Online CPUs | `sys/devices/system/cpu/online` | `processor` lines of `proc/cpuinfo` |
//! | NUMA nodes | `sys/devices/system/node/node<N>/cpulist` and `meminfo` | no nodes when the directory is absent |
//! | Memory | `MemTotal` and `MemAvailable` of `proc/meminfo` | `MemFree` when `MemAvailable` is absent |
//! | Kernel version | `proc/sys/kernel/osrelease` | none |
//! | AF_XDP | kernel 4.18 or newer, then `CONFIG_XDP_SOCKETS` in `boot/config-<release>`, then `xsk_create` in `proc/kallsyms` | unknown |
//! | Ports | `sys/class/net/<name>/{type,address,mtu,speed}` | speed 0 when unreadable |
//! | Port driver | `DRIVER=` in `sys/class/net/<name>/device/uevent` | the `device/driver` link, then empty |
//! | Port NUMA node | `sys/class/net/<name>/device/numa_node` | none |

use crate::error::{DetectError, KernelError};
use crate::proto::{Backend, Capabilities, PortInfo};
use crate::sysroot::SystemRoot;

/// The contract version of `agent.proto` that this crate implements.
pub const ENGINE_API: &str = "0.2.0";

/// The agent build version.
pub const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The largest CPU list that detection accepts, which bounds allocation when
/// a file holds a nonsensical range.
const MAX_CPUS: usize = 1 << 16;

/// The `type` value of a loopback interface, `ARPHRD_LOOPBACK`.
const ARPHRD_LOOPBACK: &str = "772";

/// Everything that detection found on one host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCapabilities {
    /// The online CPU ids in ascending order.
    pub online_cpus: Vec<u32>,
    /// The NUMA nodes in ascending id order, empty when the kernel exposes none.
    pub numa_nodes: Vec<NumaNode>,
    /// `MemTotal` in bytes.
    pub memory_total_bytes: u64,
    /// `MemAvailable` in bytes, or `MemFree` on kernels without `MemAvailable`.
    pub memory_available_bytes: u64,
    /// The running kernel.
    pub kernel: KernelVersion,
    /// Whether the kernel offers AF_XDP sockets, and the evidence.
    pub af_xdp: AfXdpSupport,
    /// Every interface except loopback, in name order.
    pub ports: Vec<DetectedPort>,
}

/// One NUMA node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumaNode {
    /// The node id from the directory name `node<id>`.
    pub id: u32,
    /// The CPU ids of the node in ascending order.
    pub cpus: Vec<u32>,
    /// `MemTotal` of the node in bytes, or 0 when the node reports none.
    pub memory_bytes: u64,
}

/// The running kernel version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelVersion {
    /// The full release string, for example `6.8.0-45-generic`.
    pub release: String,
    /// The major version.
    pub major: u32,
    /// The minor version.
    pub minor: u32,
    /// The patch level, or 0 when the release names none.
    pub patch: u32,
}

impl KernelVersion {
    /// Parses a release string such as `6.8.0-45-generic`.
    pub fn parse(release: &str) -> Option<Self> {
        let release = release.trim();
        let mut parts = release.splitn(3, '.');
        let major = parts.next()?.parse().ok()?;
        let minor = leading_number(parts.next()?)?;
        let patch = parts.next().and_then(leading_number).unwrap_or(0);
        Some(Self {
            release: release.to_owned(),
            major,
            minor,
            patch,
        })
    }

    /// Reports whether this kernel is at least `major.minor`.
    pub fn at_least(&self, major: u32, minor: u32) -> bool {
        (self.major, self.minor) >= (major, minor)
    }
}

/// The first Linux release with the `IP_LOCAL_PORT_RANGE` socket option, which
/// the agent needs to give each worker its own range of source ports.
pub const MIN_KERNEL: (u32, u32) = (6, 3);

impl KernelVersion {
    /// Checks that this kernel offers `IP_LOCAL_PORT_RANGE`.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::TooOld`] for a release before [`MIN_KERNEL`].
    pub fn require_ip_local_port_range(&self) -> Result<(), KernelError> {
        if self.at_least(MIN_KERNEL.0, MIN_KERNEL.1) {
            Ok(())
        } else {
            Err(KernelError::TooOld {
                release: self.release.clone(),
                required: MIN_KERNEL,
            })
        }
    }
}

/// Reads the kernel release under `root` and checks that it offers
/// `IP_LOCAL_PORT_RANGE`. The agent calls it at startup, before it does any
/// other work.
///
/// # Errors
///
/// Returns [`KernelError::TooOld`] for a kernel before Linux 6.3, and
/// [`KernelError::Detect`] when `proc/sys/kernel/osrelease` is missing or
/// malformed.
pub fn require_kernel(root: &SystemRoot) -> Result<KernelVersion, KernelError> {
    let kernel = detect_kernel(root)?;
    kernel.require_ip_local_port_range()?;
    Ok(kernel)
}

/// Whether the kernel offers AF_XDP sockets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfXdpSupport {
    /// The kernel offers AF_XDP sockets.
    Supported(XdpEvidence),
    /// The kernel lacks AF_XDP sockets.
    Unsupported(XdpEvidence),
    /// No source settled the question. The agent then does not offer the
    /// AF_XDP backend, and the backend that binds a socket finds out for sure.
    Unknown,
}

impl AfXdpSupport {
    /// Reports whether the kernel offers AF_XDP sockets.
    pub fn is_supported(self) -> bool {
        matches!(self, Self::Supported(_))
    }
}

/// The source that decided [`AfXdpSupport`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XdpEvidence {
    /// The kernel is older than 4.18, which introduced AF_XDP.
    KernelVersion,
    /// The line `CONFIG_XDP_SOCKETS` in `boot/config-<release>`.
    KernelConfig,
    /// The presence or absence of `xsk_create` in `proc/kallsyms`.
    KernelSymbols,
}

/// One interface with the facts that do not fit [`PortInfo`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedPort {
    /// The facts that the proto `PortInfo` message carries.
    pub info: PortInfo,
    /// The NUMA node of the device, or `None` when the kernel reports none.
    pub numa_node: Option<u32>,
    /// What the driver offers for XDP on this kernel.
    pub xdp: DriverXdp,
}

/// What a driver offers for XDP, from a table of known drivers.
///
/// The table is a hint for backend selection. Generic XDP works on every
/// interface, and only an AF_XDP bind with `XDP_ZEROCOPY` proves zero copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DriverXdp {
    /// The driver runs XDP programs natively.
    pub native: bool,
    /// The driver supports AF_XDP zero copy on this kernel.
    pub zero_copy: bool,
}

/// Drivers with native XDP, from the kernel tree.
const NATIVE_XDP_DRIVERS: &[&str] = &[
    "bnxt_en",
    "ena",
    "hv_netvsc",
    "i40e",
    "ice",
    "igb",
    "igc",
    "ixgbe",
    "ixgbevf",
    "mlx4_core",
    "mlx5_core",
    "nfp",
    "qede",
    "tun",
    "veth",
    "virtio_net",
];

/// Drivers with AF_XDP zero copy and the first kernel that has it.
const ZERO_COPY_DRIVERS: &[(&str, u32, u32)] = &[
    ("i40e", 4, 18),
    ("ixgbe", 4, 20),
    ("mlx5_core", 5, 3),
    ("ice", 5, 5),
    ("igc", 5, 14),
    ("virtio_net", 6, 11),
];

/// Returns what `driver` offers for XDP on `kernel`.
pub fn driver_xdp(driver: &str, kernel: &KernelVersion) -> DriverXdp {
    let native = NATIVE_XDP_DRIVERS.contains(&driver);
    let zero_copy = ZERO_COPY_DRIVERS
        .iter()
        .any(|&(d, major, minor)| d == driver && kernel.at_least(major, minor));
    DriverXdp { native, zero_copy }
}

impl HostCapabilities {
    /// Fills the proto `Capabilities` message.
    ///
    /// `modules` and `labels` stay empty, because the module store and the
    /// agent configuration own them.
    pub fn to_capabilities(&self) -> Capabilities {
        let mut backends = vec![Backend::Kernel];
        if self.af_xdp.is_supported() {
            backends.push(Backend::AfXdp);
        }
        Capabilities {
            agent_version: AGENT_VERSION.to_owned(),
            engine_api: ENGINE_API.to_owned(),
            backends,
            cores: u32::try_from(self.online_cpus.len()).unwrap_or(u32::MAX),
            memory_bytes: self.memory_available_bytes,
            ports: self.ports.iter().map(|p| p.info.clone()).collect(),
            modules: Vec::new(),
            labels: Default::default(),
        }
    }
}

/// Detects the capabilities of the host under `root`.
///
/// # Errors
///
/// Fails when `proc/meminfo` or `proc/sys/kernel/osrelease` is missing or
/// malformed, when no source lists a CPU, or when a file exists but cannot be
/// read for a reason other than absence.
pub fn detect(root: &SystemRoot) -> Result<HostCapabilities, DetectError> {
    let online_cpus = detect_cpus(root)?;
    let numa_nodes = detect_numa(root)?;
    let (memory_total_bytes, memory_available_bytes) = detect_memory(root)?;
    let kernel = detect_kernel(root)?;
    let af_xdp = detect_af_xdp(root, &kernel)?;
    let ports = detect_ports(root, &kernel)?;
    Ok(HostCapabilities {
        online_cpus,
        numa_nodes,
        memory_total_bytes,
        memory_available_bytes,
        kernel,
        af_xdp,
        ports,
    })
}

fn detect_cpus(root: &SystemRoot) -> Result<Vec<u32>, DetectError> {
    const ONLINE: &str = "sys/devices/system/cpu/online";
    const CPUINFO: &str = "proc/cpuinfo";
    if let Some(text) = root.read_optional(ONLINE)? {
        let cpus = parse_cpu_list(&text)
            .ok_or_else(|| DetectError::parse(root.path(ONLINE), "cpu list", &text))?;
        if !cpus.is_empty() {
            return Ok(cpus);
        }
    }
    let mut cpus = Vec::new();
    if let Some(text) = root.read_optional(CPUINFO)? {
        for line in text.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            if key.trim() == "processor" {
                let id = value
                    .trim()
                    .parse()
                    .map_err(|_| DetectError::parse(root.path(CPUINFO), "processor id", value))?;
                cpus.push(id);
            }
        }
    }
    if cpus.is_empty() {
        return Err(DetectError::NoCpus {
            online: root.path(ONLINE),
            cpuinfo: root.path(CPUINFO),
        });
    }
    cpus.sort_unstable();
    cpus.dedup();
    Ok(cpus)
}

fn detect_numa(root: &SystemRoot) -> Result<Vec<NumaNode>, DetectError> {
    const NODES: &str = "sys/devices/system/node";
    let mut nodes = Vec::new();
    for name in root.list_dir(NODES)? {
        let Some(id) = name.strip_prefix("node").and_then(|n| n.parse().ok()) else {
            continue;
        };
        let cpulist = format!("{NODES}/{name}/cpulist");
        let cpus = match root.read_optional(&cpulist)? {
            Some(text) => parse_cpu_list(&text)
                .ok_or_else(|| DetectError::parse(root.path(&cpulist), "cpu list", &text))?,
            None => Vec::new(),
        };
        let meminfo = format!("{NODES}/{name}/meminfo");
        let memory_bytes = root
            .read_optional(&meminfo)?
            .and_then(|text| meminfo_bytes(&text, "MemTotal"))
            .unwrap_or(0);
        nodes.push(NumaNode {
            id,
            cpus,
            memory_bytes,
        });
    }
    nodes.sort_by_key(|n| n.id);
    Ok(nodes)
}

fn detect_memory(root: &SystemRoot) -> Result<(u64, u64), DetectError> {
    const MEMINFO: &str = "proc/meminfo";
    let text = root.read(MEMINFO)?;
    let total = meminfo_bytes(&text, "MemTotal")
        .ok_or_else(|| DetectError::parse(root.path(MEMINFO), "MemTotal line", &text))?;
    let available = meminfo_bytes(&text, "MemAvailable")
        .or_else(|| meminfo_bytes(&text, "MemFree"))
        .ok_or_else(|| DetectError::parse(root.path(MEMINFO), "MemAvailable line", &text))?;
    Ok((total, available))
}

fn detect_kernel(root: &SystemRoot) -> Result<KernelVersion, DetectError> {
    const OSRELEASE: &str = "proc/sys/kernel/osrelease";
    let text = root.read(OSRELEASE)?;
    KernelVersion::parse(&text)
        .ok_or_else(|| DetectError::parse(root.path(OSRELEASE), "kernel release", &text))
}

fn detect_af_xdp(root: &SystemRoot, kernel: &KernelVersion) -> Result<AfXdpSupport, DetectError> {
    if !kernel.at_least(4, 18) {
        return Ok(AfXdpSupport::Unsupported(XdpEvidence::KernelVersion));
    }
    // The release string comes from the kernel, but a path must not escape the root.
    if !kernel.release.contains('/') {
        let config = format!("boot/config-{}", kernel.release);
        if root.exists(&config) {
            let enabled = root.find_line(&config, |line| {
                (line.trim() == "CONFIG_XDP_SOCKETS=y").then_some(())
            })?;
            return Ok(match enabled {
                Some(()) => AfXdpSupport::Supported(XdpEvidence::KernelConfig),
                None => AfXdpSupport::Unsupported(XdpEvidence::KernelConfig),
            });
        }
    }
    // Symbol names stay visible to unprivileged readers, who see zero addresses.
    let mut any_symbol = false;
    let found = root.find_line("proc/kallsyms", |line| {
        let name = line.split_whitespace().nth(2)?;
        any_symbol = true;
        (name == "xsk_create").then_some(())
    })?;
    Ok(match (found, any_symbol) {
        (Some(()), _) => AfXdpSupport::Supported(XdpEvidence::KernelSymbols),
        (None, true) => AfXdpSupport::Unsupported(XdpEvidence::KernelSymbols),
        (None, false) => AfXdpSupport::Unknown,
    })
}

fn detect_ports(
    root: &SystemRoot,
    kernel: &KernelVersion,
) -> Result<Vec<DetectedPort>, DetectError> {
    const NET: &str = "sys/class/net";
    let mut ports = Vec::new();
    for name in root.list_dir(NET)? {
        let dir = format!("{NET}/{name}");
        let kind = root.read_optional(&format!("{dir}/type"))?;
        let is_loopback = match &kind {
            Some(kind) => kind.trim() == ARPHRD_LOOPBACK,
            None => name == "lo",
        };
        if is_loopback {
            continue;
        }
        let driver = port_driver(root, &dir)?;
        let mac = root
            .read_optional(&format!("{dir}/address"))?
            .map(|m| m.trim().to_ascii_lowercase())
            .unwrap_or_default();
        let mtu = read_number(root, &format!("{dir}/mtu"))?
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);
        // A link that is down or a virtual device reports -1 or nothing.
        let speed_mbps = read_number(root, &format!("{dir}/speed"))?
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);
        let numa_node = read_number(root, &format!("{dir}/device/numa_node"))?
            .and_then(|v| u32::try_from(v).ok());
        let xdp = driver_xdp(&driver, kernel);
        ports.push(DetectedPort {
            info: PortInfo {
                name,
                driver,
                speed_mbps,
                mac,
                mtu,
            },
            numa_node,
            xdp,
        });
    }
    Ok(ports)
}

/// Returns the driver of the device behind an interface, or an empty string
/// for a virtual interface without a device.
fn port_driver(root: &SystemRoot, dir: &str) -> Result<String, DetectError> {
    if let Some(uevent) = root.read_optional(&format!("{dir}/device/uevent"))? {
        if let Some(driver) = uevent
            .lines()
            .find_map(|l| l.trim().strip_prefix("DRIVER="))
        {
            return Ok(driver.to_owned());
        }
    }
    let link = root.path(&format!("{dir}/device/driver"));
    Ok(std::fs::read_link(link)
        .ok()
        .and_then(|target| target.file_name()?.to_str().map(str::to_owned))
        .unwrap_or_default())
}

/// Reads a file that holds one signed integer.
fn read_number(root: &SystemRoot, rel: &str) -> Result<Option<i64>, DetectError> {
    Ok(root
        .read_optional(rel)?
        .and_then(|text| text.trim().parse().ok()))
}

/// Parses a kernel CPU list such as `0-3,8,10-11`. An empty list is valid.
/// Returns `None` for malformed text and for lists longer than [`MAX_CPUS`].
pub fn parse_cpu_list(text: &str) -> Option<Vec<u32>> {
    let text = text.trim();
    let mut cpus = Vec::new();
    if text.is_empty() {
        return Some(cpus);
    }
    for part in text.split(',') {
        let (start, end) = match part.split_once('-') {
            Some((a, b)) => (a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?),
            None => {
                let v = part.trim().parse::<u32>().ok()?;
                (v, v)
            }
        };
        if end < start {
            return None;
        }
        let count = usize::try_from(end - start).ok()?.checked_add(1)?;
        if cpus.len().checked_add(count)? > MAX_CPUS {
            return None;
        }
        cpus.extend(start..=end);
    }
    cpus.sort_unstable();
    cpus.dedup();
    Some(cpus)
}

/// Returns the value of `key` in a meminfo file in bytes. The function reads
/// both `/proc/meminfo` lines (`MemTotal: 16 kB`) and per node lines
/// (`Node 0 MemTotal: 16 kB`).
fn meminfo_bytes(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let mut tokens = line.split_whitespace();
        tokens.find(|t| t.strip_suffix(':') == Some(key))?;
        let value: u64 = tokens.next()?.parse().ok()?;
        match tokens.next() {
            Some("kB") => value.checked_mul(1024),
            None => Some(value),
            Some(_) => None,
        }
    })
}

/// Parses the decimal digits at the start of `s`.
fn leading_number(s: &str) -> Option<u32> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    s[..end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_list_parses_ranges_and_singles() {
        assert_eq!(
            parse_cpu_list("0-3,8,10-11\n"),
            Some(vec![0, 1, 2, 3, 8, 10, 11])
        );
        assert_eq!(parse_cpu_list("5"), Some(vec![5]));
        assert_eq!(parse_cpu_list("\n"), Some(vec![]));
    }

    #[test]
    fn cpu_list_rejects_malformed_and_huge_input() {
        assert_eq!(parse_cpu_list("3-1"), None);
        assert_eq!(parse_cpu_list("a-b"), None);
        assert_eq!(parse_cpu_list("0-4294967295"), None);
        assert_eq!(parse_cpu_list("1,,2"), None);
    }

    #[test]
    fn meminfo_reads_host_and_node_lines() {
        let host = "MemTotal:       16314612 kB\nMemFree:  100 kB\nMemAvailable:   8000 kB\n";
        assert_eq!(meminfo_bytes(host, "MemTotal"), Some(16_314_612 * 1024));
        assert_eq!(meminfo_bytes(host, "MemAvailable"), Some(8000 * 1024));
        let node = "Node 1 MemTotal:       8157306 kB\nNode 1 MemFree: 1 kB\n";
        assert_eq!(meminfo_bytes(node, "MemTotal"), Some(8_157_306 * 1024));
        assert_eq!(meminfo_bytes(node, "MemAvailable"), None);
        assert_eq!(
            meminfo_bytes("MemTotal: 99999999999999999999 kB", "MemTotal"),
            None
        );
    }

    #[test]
    fn kernel_release_parses_distribution_suffixes() {
        let k = KernelVersion::parse("6.8.0-45-generic\n");
        assert_eq!(
            k.map(|k| (k.major, k.minor, k.patch, k.release)),
            Some((6, 8, 0, "6.8.0-45-generic".to_owned()))
        );
        let k = KernelVersion::parse("5.15-rc1");
        assert_eq!(k.map(|k| (k.major, k.minor, k.patch)), Some((5, 15, 0)));
        assert_eq!(KernelVersion::parse("linux"), None);
        assert_eq!(KernelVersion::parse("6"), None);
    }

    fn kernel(release: &str) -> KernelVersion {
        KernelVersion::parse(release).unwrap_or(KernelVersion {
            release: String::new(),
            major: 0,
            minor: 0,
            patch: 0,
        })
    }

    #[test]
    fn virtio_net_zero_copy_starts_with_kernel_6_11() {
        let before = driver_xdp("virtio_net", &kernel("6.10.3"));
        let after = driver_xdp("virtio_net", &kernel("6.12.0"));
        assert_eq!(
            before,
            DriverXdp {
                native: true,
                zero_copy: false
            }
        );
        assert_eq!(
            after,
            DriverXdp {
                native: true,
                zero_copy: true
            }
        );
        assert_eq!(
            driver_xdp("e1000e", &kernel("6.12.0")),
            DriverXdp::default()
        );
    }
}
