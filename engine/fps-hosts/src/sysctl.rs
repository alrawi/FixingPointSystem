// SPDX-License-Identifier: GPL-3.0-only
//! The sysctl profile of a kernel test port.
//!
//! Every key of the profile is per network namespace, so the profile changes
//! only the namespace of the port and never the host. The keys
//! `net.core.rmem_max` and `net.core.wmem_max` are optional. Older kernels keep
//! them global, and newer kernels do not let a child namespace raise them above
//! the host value, so setup skips them when they are missing or do not take
//! the value. The TCP buffer limits `tcp_rmem` and `tcp_wmem` are required and
//! bound the autotuned buffers without that host limit.

/// Whether setup fails when the namespace lacks a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Setup fails when the key is missing or does not take the value.
    Required,
    /// Setup skips the key when it is missing in the namespace.
    Optional,
}

/// One sysctl key and the value that setup writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysctlEntry {
    /// The dotted key, such as `net.ipv4.tcp_tw_reuse`.
    pub key: &'static str,
    /// The value, with fields separated by one space.
    pub value: String,
    /// Whether the key must exist.
    pub need: Need,
}

/// Errors in a sysctl profile.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    /// The local port range is empty or starts below 1024.
    #[error("sysctl profile: local port range {low}-{high} must satisfy 1024 <= low < high")]
    PortRange {
        /// The first port.
        low: u16,
        /// The last port.
        high: u16,
    },
    /// A buffer triple is not ordered as minimum, default, and maximum.
    #[error("sysctl profile: {key} {min} {default} {max} must satisfy 0 < min <= default <= max")]
    Buffer {
        /// The key.
        key: &'static str,
        /// The minimum.
        min: u32,
        /// The default.
        default: u32,
        /// The maximum.
        max: u32,
    },
    /// `tcp_tw_reuse` is not 0, 1, or 2.
    #[error("sysctl profile: tcp_tw_reuse {0} must be 0, 1, or 2")]
    TwReuse(u8),
}

/// Minimum, default, and maximum of a TCP buffer in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferSizes {
    /// The minimum buffer size.
    pub min: u32,
    /// The initial buffer size.
    pub default: u32,
    /// The maximum buffer size that autotuning reaches.
    pub max: u32,
}

/// The sysctl profile of a test port.
///
/// The default suits a port that opens tens of thousands of sessions from
/// many source addresses. It widens the local port range, lets new outgoing
/// connections reuse TIME_WAIT pairs, and raises the socket buffer limits to
/// 16 MiB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysctlProfile {
    /// `net.ipv4.ip_local_port_range`, which IPv6 shares.
    pub local_port_range: (u16, u16),
    /// `net.ipv4.tcp_tw_reuse`. 1 reuses TIME_WAIT for every outgoing
    /// connection, 2 only for loopback, and 0 never.
    pub tcp_tw_reuse: u8,
    /// `net.ipv4.tcp_rmem`.
    pub tcp_rmem: BufferSizes,
    /// `net.ipv4.tcp_wmem`.
    pub tcp_wmem: BufferSizes,
    /// `net.core.rmem_max`, the largest `SO_RCVBUF` a socket may request.
    pub rmem_max: u32,
    /// `net.core.wmem_max`, the largest `SO_SNDBUF` a socket may request.
    pub wmem_max: u32,
}

const MIB16: u32 = 16 * 1024 * 1024;

impl Default for SysctlProfile {
    fn default() -> Self {
        Self {
            local_port_range: (1024, 65535),
            tcp_tw_reuse: 1,
            tcp_rmem: BufferSizes {
                min: 4096,
                default: 131_072,
                max: MIB16,
            },
            tcp_wmem: BufferSizes {
                min: 4096,
                default: 16_384,
                max: MIB16,
            },
            rmem_max: MIB16,
            wmem_max: MIB16,
        }
    }
}

impl SysctlProfile {
    /// Checks the ranges that the kernel would otherwise reject or clamp.
    pub fn validate(&self) -> Result<(), ProfileError> {
        let (low, high) = self.local_port_range;
        if low < 1024 || low >= high {
            return Err(ProfileError::PortRange { low, high });
        }
        if self.tcp_tw_reuse > 2 {
            return Err(ProfileError::TwReuse(self.tcp_tw_reuse));
        }
        for (key, b) in [
            ("net.ipv4.tcp_rmem", self.tcp_rmem),
            ("net.ipv4.tcp_wmem", self.tcp_wmem),
        ] {
            if b.min == 0 || b.min > b.default || b.default > b.max {
                return Err(ProfileError::Buffer {
                    key,
                    min: b.min,
                    default: b.default,
                    max: b.max,
                });
            }
        }
        Ok(())
    }

    /// The entries that setup writes, in order. `ipv6` selects whether the
    /// namespace keeps IPv6 on, without router advertisements, or turns it
    /// off so the port sends no IPv6 traffic.
    pub fn entries(&self, ipv6: bool) -> Vec<SysctlEntry> {
        let triple = |b: BufferSizes| format!("{} {} {}", b.min, b.default, b.max);
        let e = |key, value: String, need| SysctlEntry { key, value, need };
        let mut out = vec![
            e(
                "net.ipv4.ip_local_port_range",
                format!("{} {}", self.local_port_range.0, self.local_port_range.1),
                Need::Required,
            ),
            e(
                "net.ipv4.tcp_tw_reuse",
                self.tcp_tw_reuse.to_string(),
                Need::Required,
            ),
            e("net.ipv4.tcp_rmem", triple(self.tcp_rmem), Need::Required),
            e("net.ipv4.tcp_wmem", triple(self.tcp_wmem), Need::Required),
            e(
                "net.core.rmem_max",
                self.rmem_max.to_string(),
                Need::Optional,
            ),
            e(
                "net.core.wmem_max",
                self.wmem_max.to_string(),
                Need::Optional,
            ),
        ];
        if ipv6 {
            out.push(e("net.ipv6.conf.all.accept_ra", "0".into(), Need::Required));
            out.push(e(
                "net.ipv6.conf.default.accept_ra",
                "0".into(),
                Need::Required,
            ));
        } else {
            // A kernel without IPv6 has no such keys and needs no change.
            out.push(e(
                "net.ipv6.conf.all.disable_ipv6",
                "1".into(),
                Need::Optional,
            ));
            out.push(e(
                "net.ipv6.conf.default.disable_ipv6",
                "1".into(),
                Need::Optional,
            ));
        }
        out
    }
}

/// Reports whether a value read back from `/proc/sys` equals the written one.
/// The kernel separates fields with tabs, so the comparison splits on any
/// whitespace.
pub fn same_value(written: &str, read: &str) -> bool {
    written.split_whitespace().eq(read.split_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_valid_and_lists_ipv4_keys_first() {
        let p = SysctlProfile::default();
        p.validate().unwrap();
        let keys: Vec<_> = p.entries(false).iter().map(|e| e.key).collect();
        assert_eq!(
            keys,
            [
                "net.ipv4.ip_local_port_range",
                "net.ipv4.tcp_tw_reuse",
                "net.ipv4.tcp_rmem",
                "net.ipv4.tcp_wmem",
                "net.core.rmem_max",
                "net.core.wmem_max",
                "net.ipv6.conf.all.disable_ipv6",
                "net.ipv6.conf.default.disable_ipv6",
            ]
        );
        let e = p.entries(true);
        assert_eq!(e[0].value, "1024 65535");
        assert_eq!(e[2].value, "4096 131072 16777216");
        assert!(e.iter().any(|x| x.key == "net.ipv6.conf.all.accept_ra"));
        assert!(!e.iter().any(|x| x.key.ends_with("disable_ipv6")));
    }

    #[test]
    fn validate_rejects_bad_ranges() {
        let mut p = SysctlProfile {
            local_port_range: (2000, 2000),
            ..SysctlProfile::default()
        };
        assert!(matches!(p.validate(), Err(ProfileError::PortRange { .. })));
        p.local_port_range = (80, 9000);
        assert!(matches!(p.validate(), Err(ProfileError::PortRange { .. })));
        p.local_port_range = (1024, 65535);
        p.tcp_tw_reuse = 3;
        assert_eq!(p.validate(), Err(ProfileError::TwReuse(3)));
        p.tcp_tw_reuse = 1;
        p.tcp_wmem.default = p.tcp_wmem.max + 1;
        assert!(matches!(
            p.validate(),
            Err(ProfileError::Buffer {
                key: "net.ipv4.tcp_wmem",
                ..
            })
        ));
    }

    #[test]
    fn same_value_ignores_tabs_and_trailing_newline() {
        assert!(same_value("1024 65535", "1024\t65535\n"));
        assert!(!same_value("1024 65535", "32768\t60999\n"));
    }
}
