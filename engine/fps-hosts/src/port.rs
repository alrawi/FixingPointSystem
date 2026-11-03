// SPDX-License-Identifier: GPL-3.0-only
//! Setup and teardown of a kernel test port.
//!
//! A kernel test port is one test interface that the kernel backend uses for
//! the hosts of one pool slice. [`setup`] moves the interface into a network
//! namespace of its own and configures it in eight steps.
//!
//! 1. Refuse the interface when it looks like a management interface, that is
//!    when it is the loopback, a named management interface, enslaved to a
//!    master, the device of a default route, or the owner of a global address.
//! 2. Create the namespace `fps-port-<name>` and bring its loopback up.
//! 3. Apply the [`SysctlProfile`] inside the namespace and read every value back.
//! 4. Move the interface into the namespace, set its MTU, and bring it up.
//! 5. Create the optional 802.1Q subinterface `vlan<id>` on top of it.
//! 6. Assign the optional interface address, for a device under test that
//!    routes the pool prefix to the port.
//! 7. Add `local` routes for the addresses of the slice, so sockets bind any
//!    of them without one interface address per host (AnyIP). The routes
//!    leave out the gateway when it lies inside the slice.
//! 8. Add the default route via the gateway with the `onlink` flag.
//!
//! Every step after the namespace exists runs inside it. The only commands
//! that change the origin namespace move the test interface out and back and
//! restore its MTU and state, so setup never changes another interface. [`KernelPort::teardown`] moves the
//! interface back, restores its MTU and its state, and deletes the namespace,
//! which removes every route and subinterface that setup created. A failed
//! setup runs the same teardown before it returns the error.

use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

use serde_json::Value;

use crate::ipcmd::{CommandError, IpRunner};
use crate::pool::{HostPool, Ipv4Prefix, Ipv6Prefix, PoolError};
use crate::slice::HostRange;
use crate::sysctl::{same_value, Need, ProfileError, SysctlProfile};

/// The prefix of every namespace that port setup creates.
pub const NAMESPACE_PREFIX: &str = "fps-port-";

/// Errors that port setup and teardown report.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortError {
    /// A field of the port specification is invalid.
    #[error("port {port}: invalid specification: {reason}")]
    InvalidSpec {
        /// The port name.
        port: String,
        /// What is wrong.
        reason: String,
    },
    /// The sysctl profile is invalid.
    #[error("port {port}: {source}")]
    Profile {
        /// The port name.
        port: String,
        /// The profile error.
        #[source]
        source: ProfileError,
    },
    /// The host pool cannot map the slice of the port.
    #[error("port {port}: {source}")]
    Pool {
        /// The port name.
        port: String,
        /// The pool error.
        #[source]
        source: PoolError,
    },
    /// The interface looks like a management interface, so setup refuses it.
    #[error("port {port}: refusing interface {interface}: {reason}")]
    Management {
        /// The port name.
        port: String,
        /// The interface name.
        interface: String,
        /// Why the interface looks like a management interface.
        reason: String,
    },
    /// The interface does not exist in its origin namespace.
    #[error("port {port}: interface {interface} not found")]
    InterfaceNotFound {
        /// The port name.
        port: String,
        /// The interface name.
        interface: String,
        /// The failed lookup.
        #[source]
        source: CommandError,
    },
    /// The namespace of the port exists already, for example after a crash.
    /// [`cleanup`] removes it.
    #[error("port {port}: namespace {namespace} exists, run cleanup first")]
    NamespaceExists {
        /// The port name.
        port: String,
        /// The namespace name.
        namespace: String,
    },
    /// An `ip` command failed.
    #[error("port {port}: {step} failed")]
    Step {
        /// The port name.
        port: String,
        /// The setup or teardown step.
        step: &'static str,
        /// The failed command.
        #[source]
        source: CommandError,
    },
    /// An `ip` command printed output that setup cannot read.
    #[error("port {port}: {step}: unexpected output: {reason}")]
    Output {
        /// The port name.
        port: String,
        /// The setup step.
        step: &'static str,
        /// What is wrong with the output.
        reason: String,
    },
    /// A required sysctl key does not exist in the namespace.
    #[error("port {port}: sysctl {key} does not exist in the port namespace")]
    SysctlMissing {
        /// The port name.
        port: String,
        /// The key.
        key: &'static str,
    },
    /// A sysctl key did not take the value of the profile.
    #[error("port {port}: sysctl {key} reads '{got}' after writing '{want}'")]
    SysctlMismatch {
        /// The port name.
        port: String,
        /// The key.
        key: &'static str,
        /// The written value.
        want: String,
        /// The value read back.
        got: String,
    },
    /// Teardown could not remove everything.
    #[error("port {port}: teardown incomplete: {}", failures.join("; "))]
    Teardown {
        /// The port name.
        port: String,
        /// Each failed step with its cause.
        failures: Vec<String>,
    },
}

/// An address with a prefix length, such as `10.201.0.2/24`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InterfaceAddr {
    /// The address.
    pub addr: IpAddr,
    /// The prefix length of the connected subnet.
    pub prefix_len: u8,
}

impl FromStr for InterfaceAddr {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (addr, len) = s
            .split_once('/')
            .ok_or_else(|| format!("address '{s}' has no prefix length"))?;
        let addr: IpAddr = addr
            .parse()
            .map_err(|_| format!("address '{s}' is not an IP address"))?;
        let prefix_len: u8 = len
            .parse()
            .map_err(|_| format!("address '{s}' has an invalid prefix length"))?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        if prefix_len > max {
            return Err(format!("address '{s}' has a prefix length above {max}"));
        }
        Ok(Self { addr, prefix_len })
    }
}

impl fmt::Display for InterfaceAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.prefix_len)
    }
}

/// The configuration of one kernel test port.
///
/// [`PortSpec::new`] fills the local prefixes and the gateway from a host pool
/// and the slice of the port. The other fields are public, so a caller sets
/// the VLAN, MTU, address, profile, and management names before setup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortSpec {
    /// The port name, `[a-z0-9]{1,12}`, which names the namespace.
    pub name: String,
    /// The test interface, such as `ens1f0`.
    pub interface: String,
    /// The named namespace that holds the interface before setup, or `None`
    /// for the namespace of this process.
    pub origin_netns: Option<String>,
    /// The 802.1Q id of the data subinterface, or `None` for untagged frames.
    pub vlan: Option<u16>,
    /// The MTU of the interface, or `None` to keep it.
    pub mtu: Option<u32>,
    /// An address on the data device, or `None`. The device under test may
    /// route the pool prefix to it instead of resolving every host address.
    pub address: Option<InterfaceAddr>,
    /// The IPv4 prefixes that the `local` routes cover.
    pub local_v4: Vec<Ipv4Prefix>,
    /// The IPv6 prefixes that the `local` routes cover.
    pub local_v6: Vec<Ipv6Prefix>,
    /// The next hop of the default route, or `None` for no default route.
    pub gateway: Option<IpAddr>,
    /// The sysctl profile of the namespace.
    pub sysctl: SysctlProfile,
    /// Interface names that setup always refuses, in addition to the checks
    /// of step 1.
    pub management: Vec<String>,
}

impl PortSpec {
    /// A port for the hosts `slice` of `pool` on `interface`, with the
    /// gateway of the pool and the default sysctl profile.
    pub fn new(
        name: &str,
        interface: &str,
        pool: &HostPool,
        slice: HostRange,
    ) -> Result<Self, PortError> {
        let port = name.to_owned();
        let pool_err = |source| PortError::Pool {
            port: port.clone(),
            source,
        };
        if slice.is_empty() {
            return Err(PortError::InvalidSpec {
                port,
                reason: "the host slice is empty".into(),
            });
        }
        let last_index = u32::try_from(slice.end() - 1).unwrap_or(u32::MAX);
        let gateway = pool.gateway();
        let mut local_v4 = Vec::new();
        let first = pool.ipv4_addr(slice.offset).map_err(pool_err)?;
        let last = pool.ipv4_addr(last_index).map_err(pool_err)?;
        if let (Some(first), Some(last)) = (first, last) {
            let skip = match gateway {
                Some(IpAddr::V4(g)) => Some(u128::from(u32::from(g))),
                _ => None,
            };
            let span = (u128::from(u32::from(first)), u128::from(u32::from(last)));
            for (net, len) in cover_without(span, skip, 32) {
                let net = u32::try_from(net).unwrap_or(0);
                local_v4.push(prefix_v4(&port, net, len)?);
            }
        }
        let mut local_v6 = Vec::new();
        let first = pool.ipv6_addr(slice.offset).map_err(pool_err)?;
        let last = pool.ipv6_addr(last_index).map_err(pool_err)?;
        if let (Some(first), Some(last)) = (first, last) {
            let skip = match gateway {
                Some(IpAddr::V6(g)) => Some(u128::from(g)),
                _ => None,
            };
            for (net, len) in cover_without((u128::from(first), u128::from(last)), skip, 128) {
                let p = Ipv6Prefix::new(net.into(), len).map_err(|e| PortError::InvalidSpec {
                    port: port.clone(),
                    reason: format!("local prefix: {e}"),
                })?;
                local_v6.push(p);
            }
        }
        Ok(Self {
            name: port,
            interface: interface.to_owned(),
            origin_netns: None,
            vlan: None,
            mtu: None,
            address: None,
            local_v4,
            local_v6,
            gateway,
            sysctl: SysctlProfile::default(),
            management: Vec::new(),
        })
    }

    /// The namespace of the port, `fps-port-<name>`.
    pub fn namespace(&self) -> String {
        format!("{NAMESPACE_PREFIX}{}", self.name)
    }

    /// The device that carries the pool traffic, which is `vlan<id>` with a
    /// VLAN and the interface itself without one.
    pub fn data_device(&self) -> String {
        match self.vlan {
            Some(id) => format!("vlan{id}"),
            None => self.interface.clone(),
        }
    }

    fn invalid(&self, reason: impl Into<String>) -> PortError {
        PortError::InvalidSpec {
            port: self.name.clone(),
            reason: reason.into(),
        }
    }

    /// Checks every field, so that no value can reach `ip` as an option or a
    /// second argument.
    pub fn validate(&self) -> Result<(), PortError> {
        let name_ok = !self.name.is_empty()
            && self.name.len() <= 12
            && self
                .name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        if !name_ok {
            return Err(self.invalid(format!("name '{}' must match [a-z0-9]{{1,12}}", self.name)));
        }
        if !is_safe_name(&self.interface, 15) {
            return Err(self.invalid(format!(
                "interface '{}' must be 1 to 15 characters of [A-Za-z0-9_.-] and not start with '-'",
                self.interface
            )));
        }
        if let Some(ns) = &self.origin_netns {
            if !is_safe_name(ns, 64) || ns.starts_with(NAMESPACE_PREFIX) {
                return Err(self.invalid(format!("origin namespace '{ns}' is invalid")));
            }
        }
        if let Some(id) = self.vlan {
            if !(1..=4094).contains(&id) {
                return Err(self.invalid(format!("vlan {id} must be from 1 to 4094")));
            }
        }
        let has_v6 = !self.local_v6.is_empty();
        if let Some(mtu) = self.mtu {
            let min = if has_v6 { 1280 } else { 576 };
            if !(min..=65535).contains(&mtu) {
                return Err(self.invalid(format!("mtu {mtu} must be from {min} to 65535")));
            }
        }
        if self.local_v4.is_empty() && self.local_v6.is_empty() {
            return Err(self.invalid("no local prefix"));
        }
        match self.gateway {
            Some(IpAddr::V4(_)) if self.local_v4.is_empty() => {
                return Err(self.invalid("an IPv4 gateway needs an IPv4 pool"));
            }
            Some(IpAddr::V6(_)) if !has_v6 => {
                return Err(self.invalid("an IPv6 gateway needs an IPv6 pool"));
            }
            _ => {}
        }
        if let Some(a) = self.address {
            if a.addr.is_ipv6() && !has_v6 {
                return Err(self.invalid("an IPv6 address needs an IPv6 pool"));
            }
            if a.addr.is_ipv4() && self.local_v4.is_empty() {
                return Err(self.invalid("an IPv4 address needs an IPv4 pool"));
            }
        }
        self.sysctl.validate().map_err(|source| PortError::Profile {
            port: self.name.clone(),
            source,
        })
    }

    fn origin_args(&self) -> Vec<&str> {
        match &self.origin_netns {
            Some(ns) => vec!["-n", ns.as_str()],
            None => Vec::new(),
        }
    }
}

/// Reports whether `s` is a name that `ip` reads as one plain argument.
fn is_safe_name(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && !s.starts_with('-')
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

fn prefix_v4(port: &str, net: u32, len: u8) -> Result<Ipv4Prefix, PortError> {
    Ipv4Prefix::new(net.into(), len).map_err(|e| PortError::InvalidSpec {
        port: port.to_owned(),
        reason: format!("local prefix: {e}"),
    })
}

/// The smallest list of aligned prefixes that covers `first..=last` of a
/// `bits` wide address space, in ascending order.
pub fn cover_range(first: u128, last: u128, bits: u32) -> Vec<(u128, u8)> {
    let mut out = Vec::new();
    if first > last || bits == 0 || bits > 128 {
        return out;
    }
    let mut start = first;
    loop {
        // The largest block that starts at `start`, is aligned, and ends at or
        // before `last`.
        let mut log = if start == 0 {
            bits
        } else {
            start.trailing_zeros().min(bits)
        };
        while log > 0 && block_last(start, log).is_none_or(|end| end > last) {
            log -= 1;
        }
        let end = block_last(start, log).unwrap_or(start);
        // `log` never exceeds `bits`, which is at most 128.
        out.push((start, u8::try_from(bits - log).unwrap_or(0)));
        if end >= last {
            break;
        }
        start = end + 1;
    }
    out
}

/// The last address of the block of `2^log` addresses that starts at `start`.
fn block_last(start: u128, log: u32) -> Option<u128> {
    if log >= 128 {
        return (start == 0).then_some(u128::MAX);
    }
    start.checked_add((1u128 << log) - 1)
}

/// [`cover_range`] of `span`, leaving out `skip` when it lies inside.
fn cover_without(span: (u128, u128), skip: Option<u128>, bits: u32) -> Vec<(u128, u8)> {
    let (first, last) = span;
    match skip {
        Some(g) if g >= first && g <= last => {
            let mut out = Vec::new();
            if g > first {
                out.extend(cover_range(first, g - 1, bits));
            }
            if g < last {
                out.extend(cover_range(g + 1, last, bits));
            }
            out
        }
        _ => cover_range(first, last, bits),
    }
}

/// The state of the interface before setup, which teardown restores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkState {
    /// The MTU.
    pub mtu: u32,
    /// Whether the interface was administratively up.
    pub up: bool,
}

/// A configured test port. Drop does nothing, so a caller must call
/// [`KernelPort::teardown`], or [`cleanup`] after a crash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelPort {
    spec: PortSpec,
    original: LinkState,
    skipped: Vec<&'static str>,
}

impl KernelPort {
    /// The specification that setup applied.
    pub fn spec(&self) -> &PortSpec {
        &self.spec
    }

    /// The namespace of the port, where the backend opens its sockets.
    pub fn namespace(&self) -> String {
        self.spec.namespace()
    }

    /// The state of the interface before setup.
    pub fn original(&self) -> LinkState {
        self.original
    }

    /// The optional sysctl keys that the namespace lacks, which setup skipped.
    pub fn skipped_sysctls(&self) -> &[&'static str] {
        &self.skipped
    }

    /// Moves the interface back, restores its MTU and state, and deletes the
    /// namespace with every route and subinterface in it.
    pub fn teardown(self, ip: &mut dyn IpRunner) -> Result<(), PortError> {
        teardown_spec(&self.spec, Some(self.original), ip)
    }
}

/// Removes the port of `spec` after a crash or a failed teardown, without
/// restoring the MTU and state of the interface. A port that does not exist
/// is not an error.
pub fn cleanup(spec: &PortSpec, ip: &mut dyn IpRunner) -> Result<(), PortError> {
    spec.validate()?;
    if !namespace_exists(spec, ip)? {
        return Ok(());
    }
    teardown_spec(spec, None, ip)
}

/// Parses the JSON array that `ip -j` prints. iproute2 prints nothing instead
/// of `[]` for some empty lists.
fn json_list(spec: &PortSpec, step: &'static str, text: &str) -> Result<Vec<Value>, PortError> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(items)) => Ok(items),
        Ok(_) => Err(output_err(spec, step, "not a JSON array")),
        Err(e) => Err(output_err(spec, step, &format!("invalid JSON: {e}"))),
    }
}

fn output_err(spec: &PortSpec, step: &'static str, reason: &str) -> PortError {
    PortError::Output {
        port: spec.name.clone(),
        step,
        reason: reason.to_owned(),
    }
}

fn step_err<'a>(spec: &'a PortSpec, step: &'static str) -> impl Fn(CommandError) -> PortError + 'a {
    move |source| PortError::Step {
        port: spec.name.clone(),
        step,
        source,
    }
}

fn namespace_exists(spec: &PortSpec, ip: &mut dyn IpRunner) -> Result<bool, PortError> {
    let step = "list namespaces";
    let out = ip
        .ip(&["-j", "netns", "list"])
        .map_err(step_err(spec, step))?;
    let ns = spec.namespace();
    Ok(json_list(spec, step, &out)?
        .iter()
        .any(|v| v.get("name").and_then(Value::as_str) == Some(ns.as_str())))
}

/// Runs `ip` with the origin namespace options in front of `args`.
fn in_origin(
    spec: &PortSpec,
    ip: &mut dyn IpRunner,
    args: &[&str],
) -> Result<String, CommandError> {
    let mut full = spec.origin_args();
    full.extend_from_slice(args);
    ip.ip(&full)
}

/// Runs `ip -n <port namespace>` with `args`.
fn in_port(spec: &PortSpec, ip: &mut dyn IpRunner, args: &[&str]) -> Result<String, CommandError> {
    let ns = spec.namespace();
    let mut full = vec!["-n", ns.as_str()];
    full.extend_from_slice(args);
    ip.ip(&full)
}

/// Step 1 refuses a management interface and returns the state to restore.
fn preflight(spec: &PortSpec, ip: &mut dyn IpRunner) -> Result<LinkState, PortError> {
    let dev = spec.interface.as_str();
    let refuse = |reason: String| PortError::Management {
        port: spec.name.clone(),
        interface: dev.to_owned(),
        reason,
    };
    if spec.management.iter().any(|m| m == dev) {
        return Err(refuse("it is listed as a management interface".into()));
    }
    if namespace_exists(spec, ip)? {
        return Err(PortError::NamespaceExists {
            port: spec.name.clone(),
            namespace: spec.namespace(),
        });
    }
    let out = in_origin(spec, ip, &["-j", "link", "show", "dev", dev]).map_err(|source| {
        PortError::InterfaceNotFound {
            port: spec.name.clone(),
            interface: dev.to_owned(),
            source,
        }
    })?;
    let links = json_list(spec, "read interface", &out)?;
    let link = links
        .first()
        .ok_or_else(|| output_err(spec, "read interface", "no link in the output"))?;
    let flags: Vec<&str> = link
        .get("flags")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if dev == "lo" || flags.contains(&"LOOPBACK") {
        return Err(refuse("it is a loopback interface".into()));
    }
    if let Some(master) = link.get("master").and_then(Value::as_str) {
        return Err(refuse(format!("it is enslaved to {master}")));
    }
    let mtu = link
        .get("mtu")
        .and_then(Value::as_u64)
        .and_then(|m| u32::try_from(m).ok())
        .ok_or_else(|| output_err(spec, "read interface", "no mtu"))?;
    let up = flags.contains(&"UP");
    for family in ["-4", "-6"] {
        // A kernel without IPv6 fails the -6 query, and then has no IPv6
        // default route either.
        let Ok(out) = in_origin(spec, ip, &["-j", family, "route", "show", "default"]) else {
            continue;
        };
        for route in json_list(spec, "read default routes", &out)? {
            if route.get("dev").and_then(Value::as_str) == Some(dev) {
                return Err(refuse(format!(
                    "it carries the {family} default route of its namespace"
                )));
            }
        }
    }
    let out = in_origin(spec, ip, &["-j", "addr", "show", "dev", dev])
        .map_err(step_err(spec, "read addresses"))?;
    for entry in json_list(spec, "read addresses", &out)? {
        let infos = entry
            .get("addr_info")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for info in infos {
            let scope = info.get("scope").and_then(Value::as_str).unwrap_or("");
            if scope != "link" && scope != "host" {
                let local = info.get("local").and_then(Value::as_str).unwrap_or("?");
                return Err(refuse(format!(
                    "it carries the address {local}, flush it if the interface is a test port"
                )));
            }
        }
    }
    Ok(LinkState { mtu, up })
}

/// Configures the port of `spec`. On failure, setup removes what it created
/// before it returns the error.
pub fn setup(spec: &PortSpec, ip: &mut dyn IpRunner) -> Result<KernelPort, PortError> {
    spec.validate()?;
    let original = preflight(spec, ip)?;
    let ns = spec.namespace();
    ip.ip(&["netns", "add", ns.as_str()])
        .map_err(step_err(spec, "create namespace"))?;
    match configure(spec, ip) {
        Ok(skipped) => Ok(KernelPort {
            spec: spec.clone(),
            original,
            skipped,
        }),
        Err(e) => {
            // The original error matters more than a rollback failure, and
            // cleanup reports any remains on the next run.
            let _ = teardown_spec(spec, Some(original), ip);
            Err(e)
        }
    }
}

/// Steps 2 to 8, which run after the namespace exists.
fn configure(spec: &PortSpec, ip: &mut dyn IpRunner) -> Result<Vec<&'static str>, PortError> {
    let ns = spec.namespace();
    let dev = spec.interface.as_str();
    in_port(spec, ip, &["link", "set", "dev", "lo", "up"])
        .map_err(step_err(spec, "bring loopback up"))?;
    let skipped = apply_sysctls(spec, ip)?;
    in_origin(spec, ip, &["link", "set", "dev", dev, "netns", ns.as_str()])
        .map_err(step_err(spec, "move interface"))?;
    let mtu = spec.mtu.map(|m| m.to_string());
    let mut args = vec!["link", "set", "dev", dev];
    if let Some(m) = &mtu {
        args.extend(["mtu", m.as_str()]);
    }
    args.push("up");
    in_port(spec, ip, &args).map_err(step_err(spec, "bring interface up"))?;
    let data = spec.data_device();
    if let Some(id) = spec.vlan {
        let id = id.to_string();
        in_port(
            spec,
            ip,
            &[
                "link", "add", "link", dev, "name", &data, "type", "vlan", "id", &id,
            ],
        )
        .map_err(step_err(spec, "create vlan"))?;
        in_port(spec, ip, &["link", "set", "dev", &data, "up"])
            .map_err(step_err(spec, "bring vlan up"))?;
    }
    if let Some(a) = spec.address {
        let a = a.to_string();
        in_port(spec, ip, &["addr", "add", &a, "dev", &data])
            .map_err(step_err(spec, "add address"))?;
    }
    let locals = spec
        .local_v4
        .iter()
        .map(ToString::to_string)
        .chain(spec.local_v6.iter().map(ToString::to_string));
    for p in locals {
        in_port(spec, ip, &["route", "add", "local", &p, "dev", "lo"])
            .map_err(step_err(spec, "add local route"))?;
    }
    if let Some(gw) = spec.gateway {
        let family = if gw.is_ipv4() { "-4" } else { "-6" };
        let gw = gw.to_string();
        in_port(
            spec,
            ip,
            &[
                family, "route", "add", "default", "via", &gw, "dev", &data, "onlink",
            ],
        )
        .map_err(step_err(spec, "add default route"))?;
    }
    Ok(skipped)
}

/// Step 3 writes every key of the profile in the port namespace and reads it
/// back.
fn apply_sysctls(spec: &PortSpec, ip: &mut dyn IpRunner) -> Result<Vec<&'static str>, PortError> {
    let ns = spec.namespace();
    let mut skipped = Vec::new();
    for entry in spec.sysctl.entries(!spec.local_v6.is_empty()) {
        let read = |ip: &mut dyn IpRunner| {
            ip.ip(&["netns", "exec", ns.as_str(), "sysctl", "-n", entry.key])
        };
        if read(ip).is_err() {
            match entry.need {
                Need::Optional => {
                    skipped.push(entry.key);
                    continue;
                }
                Need::Required => {
                    return Err(PortError::SysctlMissing {
                        port: spec.name.clone(),
                        key: entry.key,
                    })
                }
            }
        }
        let assign = format!("{}={}", entry.key, entry.value);
        ip.ip(&["netns", "exec", ns.as_str(), "sysctl", "-q", "-w", &assign])
            .map_err(step_err(spec, "write sysctl"))?;
        let got = read(ip).map_err(step_err(spec, "read sysctl"))?;
        if !same_value(&entry.value, &got) {
            return Err(PortError::SysctlMismatch {
                port: spec.name.clone(),
                key: entry.key,
                want: entry.value,
                got: got.trim().to_owned(),
            });
        }
    }
    Ok(skipped)
}

/// Removes the port of `spec`. Every step runs even when an earlier one fails,
/// and the result lists the failures.
fn teardown_spec(
    spec: &PortSpec,
    original: Option<LinkState>,
    ip: &mut dyn IpRunner,
) -> Result<(), PortError> {
    let ns = spec.namespace();
    let dev = spec.interface.as_str();
    let mut failures = Vec::new();
    if spec.vlan.is_some() {
        let data = spec.data_device();
        if in_port(spec, ip, &["link", "show", "dev", &data]).is_ok() {
            if let Err(e) = in_port(spec, ip, &["link", "del", "dev", &data]) {
                failures.push(format!("delete vlan: {e}"));
            }
        }
    }
    let in_ns = in_port(spec, ip, &["link", "show", "dev", dev]).is_ok();
    let mut stranded = false;
    if in_ns {
        // The pid names the namespace of this process when the interface came
        // from there.
        let pid = std::process::id().to_string();
        let target = spec.origin_netns.as_deref().unwrap_or(pid.as_str());
        let _ = in_port(spec, ip, &["link", "set", "dev", dev, "down"]);
        if let Err(e) = in_port(spec, ip, &["link", "set", "dev", dev, "netns", target]) {
            failures.push(format!("move interface back: {e}"));
            stranded = true;
        }
    }
    if let Some(state) = original {
        if in_ns && !stranded {
            let mtu = state.mtu.to_string();
            let updown = if state.up { "up" } else { "down" };
            if let Err(e) = in_origin(spec, ip, &["link", "set", "dev", dev, "mtu", &mtu, updown]) {
                failures.push(format!("restore interface: {e}"));
            }
        }
    }
    // Deleting the namespace would destroy a virtual interface that could not
    // move back, so the namespace stays for the operator in that case.
    if !stranded {
        match namespace_exists(spec, ip) {
            Ok(true) => {
                if let Err(e) = ip.ip(&["netns", "del", ns.as_str()]) {
                    failures.push(format!("delete namespace: {e}"));
                }
            }
            Ok(false) => {}
            Err(e) => failures.push(e.to_string()),
        }
        match namespace_exists(spec, ip) {
            Ok(false) => {}
            Ok(true) => failures.push(format!("namespace {ns} still exists")),
            Err(e) => failures.push(e.to_string()),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(PortError::Teardown {
            port: spec.name.clone(),
            failures,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::PoolSpec;

    fn pool(ipv4: Option<&str>, ipv6: Option<&str>, count: u32, gw: Option<&str>) -> HostPool {
        HostPool::parse(&PoolSpec {
            name: "p",
            ipv4,
            ipv6,
            count,
            gateway: gw,
            mac_base: None,
        })
        .unwrap()
    }

    fn render(v: &[(u128, u8)]) -> Vec<String> {
        v.iter()
            .map(|(n, l)| {
                format!(
                    "{}/{l}",
                    std::net::Ipv4Addr::from(u32::try_from(*n).unwrap())
                )
            })
            .collect()
    }

    #[test]
    fn cover_range_finds_the_minimal_aligned_blocks() {
        let a = |s: &str| u128::from(u32::from(s.parse::<std::net::Ipv4Addr>().unwrap()));
        assert_eq!(
            render(&cover_range(a("10.0.0.1"), a("10.0.0.6"), 32)),
            ["10.0.0.1/32", "10.0.0.2/31", "10.0.0.4/31", "10.0.0.6/32"]
        );
        assert_eq!(
            render(&cover_range(a("10.0.0.0"), a("10.0.255.255"), 32)),
            ["10.0.0.0/16"]
        );
        assert_eq!(
            render(&cover_range(0, u128::from(u32::MAX), 32)),
            ["0.0.0.0/0"]
        );
        assert_eq!(cover_range(0, u128::MAX, 128), [(0, 0)]);
        assert_eq!(cover_range(u128::MAX, u128::MAX, 128), [(u128::MAX, 128)]);
        assert!(cover_range(5, 4, 32).is_empty());
    }

    #[test]
    fn cover_range_blocks_are_disjoint_and_exact() {
        for (first, last) in [(1u128, 20_000u128), (3, 3), (7, 1000), (65_534, 65_537)] {
            let blocks = cover_range(first, last, 32);
            let mut next = first;
            for (net, len) in &blocks {
                assert_eq!(*net, next, "blocks of {first}..={last} must be contiguous");
                let size = 1u128 << (32 - u32::from(*len));
                assert_eq!(net % size, 0, "block {net}/{len} must be aligned");
                next = net + size;
            }
            assert_eq!(next, last + 1);
            assert!(blocks.len() <= 64);
        }
    }

    #[test]
    fn spec_covers_the_slice_and_skips_the_gateway() {
        let p = pool(Some("10.0.0.0/29"), None, 5, Some("10.0.0.3"));
        let spec = PortSpec::new("a", "eth0", &p, p.range()).unwrap();
        let got: Vec<_> = spec.local_v4.iter().map(ToString::to_string).collect();
        assert_eq!(
            got,
            ["10.0.0.1/32", "10.0.0.2/32", "10.0.0.4/31", "10.0.0.6/32"]
        );
        assert_eq!(spec.gateway, Some("10.0.0.3".parse().unwrap()));

        let p = pool(Some("10.10.0.0/16"), None, 20_000, Some("10.201.0.1"));
        let spec = PortSpec::new("a", "eth0", &p, p.range()).unwrap();
        // 10.10.0.1 to 10.10.78.32 for 20,000 hosts.
        let first = spec.local_v4.first().unwrap().to_string();
        let last = spec.local_v4.last().unwrap().to_string();
        assert_eq!(first, "10.10.0.1/32");
        assert_eq!(last, "10.10.78.32/32");

        let slice = HostRange::new(10_000, 100);
        let spec = PortSpec::new("a", "eth0", &p, slice).unwrap();
        assert!(spec.local_v4.iter().all(|net| {
            (10_000..10_100).any(|i| net.contains(p.ipv4_addr(i).unwrap().unwrap()))
        }));
        assert!(!spec
            .local_v4
            .iter()
            .any(|net| net.contains(p.ipv4_addr(9_999).unwrap().unwrap())));
    }

    #[test]
    fn spec_covers_ipv6_and_rejects_empty_slices() {
        let p = pool(None, Some("2001:db8::/64"), 4, Some("2001:db8::1"));
        let spec = PortSpec::new("a", "eth0", &p, p.range()).unwrap();
        let got: Vec<_> = spec.local_v6.iter().map(ToString::to_string).collect();
        assert_eq!(got, ["2001:db8::2/127", "2001:db8::4/127"]);
        assert!(matches!(
            PortSpec::new("a", "eth0", &p, HostRange::new(0, 0)),
            Err(PortError::InvalidSpec { .. })
        ));
    }

    fn spec() -> PortSpec {
        let p = pool(Some("10.10.0.0/16"), None, 20_000, Some("10.10.0.1"));
        PortSpec::new("t1", "eth1", &p, p.range()).unwrap()
    }

    #[test]
    fn validate_rejects_names_that_ip_would_read_as_options() {
        let mut s = spec();
        s.validate().unwrap();
        for bad in ["-n", "", "a b", "abcdefghijklmnop", "a/b", ".."] {
            s.interface = bad.into();
            assert!(s.validate().is_err(), "interface '{bad}'");
        }
        s.interface = "eth1".into();
        for bad in ["T1", "a-b", "abcdefghijklm"] {
            s.name = bad.into();
            assert!(s.validate().is_err(), "name '{bad}'");
        }
        s.name = "t1".into();
        s.origin_netns = Some("-all".into());
        assert!(s.validate().is_err());
        s.origin_netns = Some("fps-port-x".into());
        assert!(s.validate().is_err());
        s.origin_netns = None;
        s.vlan = Some(4095);
        assert!(s.validate().is_err());
        s.vlan = Some(100);
        s.mtu = Some(100);
        assert!(s.validate().is_err());
        s.mtu = Some(9000);
        s.gateway = Some("2001:db8::1".parse().unwrap());
        assert!(s.validate().is_err());
        s.gateway = None;
        s.validate().unwrap();
    }

    #[test]
    fn interface_addr_parses_and_prints() {
        let a: InterfaceAddr = "10.201.0.2/24".parse().unwrap();
        assert_eq!(a.to_string(), "10.201.0.2/24");
        assert!("10.201.0.2".parse::<InterfaceAddr>().is_err());
        assert!("10.201.0.2/33".parse::<InterfaceAddr>().is_err());
        assert!("2001:db8::2/64".parse::<InterfaceAddr>().is_ok());
    }
}
