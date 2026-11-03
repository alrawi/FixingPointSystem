// SPDX-License-Identifier: GPL-3.0-only
//! Operating system profiles of the kernel backend.
//!
//! A profile names the TCP and IP behavior of one operating system. The
//! kernel backend applies the part that Linux exposes per socket, which is the
//! TTL or hop limit, the MSS, and the window clamp. The [`SystemSettings`]
//! hold the part that Linux offers only as sysctls of a network namespace,
//! and the [`Reference`] records what the real system sends, so the realism
//! check can compare a capture with it. The profiles `windows11`, `linux6`,
//! and `macos14` ship as YAML files in the `profiles/` directory of the crate.

use serde::Deserialize;

use crate::SocketOptions;

/// The names of the built in profiles, in alphabetical order.
pub const BUILTIN_PROFILES: [&str; 3] = ["linux6", "macos14", "windows11"];

const LINUX6: &str = include_str!("../profiles/linux6.yaml");
const MACOS14: &str = include_str!("../profiles/macos14.yaml");
const WINDOWS11: &str = include_str!("../profiles/windows11.yaml");

/// The smallest MSS that Linux accepts in `TCP_MAXSEG`.
const MIN_MSS: u16 = 88;
/// The largest MSS that Linux accepts in `TCP_MAXSEG`.
const MAX_MSS: u16 = 65495;

/// Errors of loading an operating system profile.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    /// No built in profile has the name.
    #[error("unknown OS profile {name}, the built in profiles are linux6, macos14, and windows11")]
    Unknown {
        /// The requested name.
        name: String,
    },
    /// The profile file is not valid YAML or does not match the profile schema.
    #[error("OS profile does not parse")]
    Parse(#[from] serde_norway::Error),
    /// A field holds a value outside its range.
    #[error("OS profile {name}: {field} {problem}")]
    Invalid {
        /// The name of the profile.
        name: String,
        /// The field, for example `socket.mss`.
        field: &'static str,
        /// What is wrong with the value.
        problem: &'static str,
    },
}

/// The behavior of one operating system that the kernel backend imitates.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OsProfile {
    /// The name that test configurations use in `os_profile`, for example
    /// `windows11`.
    pub name: String,
    /// One sentence that describes the system.
    pub description: String,
    /// The settings that the backend applies to every socket.
    pub socket: ProfileSocket,
    /// The settings that Linux offers only per network namespace.
    pub system: SystemSettings,
    /// What the real system sends in its SYN.
    pub reference: Reference,
}

/// The per socket part of a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSocket {
    /// The initial TTL of IPv4 packets and the hop limit of IPv6 packets.
    pub ttl: u8,
    /// The MSS that the SYN and the SYN-ACK advertise, through `TCP_MAXSEG`.
    pub mss: u16,
    /// `TCP_WINDOW_CLAMP` in bytes, which bounds the receive window and with
    /// it the window scale of the handshake, or `None` for no clamp.
    pub window_clamp: Option<u32>,
}

/// The part of a profile that Linux sets per network namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemSettings {
    /// `net.ipv4.tcp_sack`, which offers selective acknowledgments.
    pub sack: bool,
    /// `net.ipv4.tcp_timestamps`, which sends the timestamp option.
    pub timestamps: bool,
    /// `net.ipv4.tcp_window_scaling`, which sends the window scale option.
    pub window_scaling: bool,
    /// The maximum of `net.ipv4.tcp_rmem` in bytes. Together with the window
    /// clamp it decides the window scale of the handshake.
    pub tcp_rmem_max: u32,
}

/// The SYN of the real system, which the realism check compares with.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    /// The window field of the SYN.
    pub syn_window: u16,
    /// The shift count of the window scale option.
    pub window_scale: u8,
    /// The TCP option kinds of the SYN in wire order, with the names `mss`,
    /// `nop`, `ws`, `sok` for SACK permitted, `ts`, and `eol`.
    pub options: Vec<String>,
    /// Whether the SYN sets the IPv4 do not fragment flag.
    pub dont_fragment: bool,
}

impl OsProfile {
    /// Returns the built in profile with the name.
    pub fn builtin(name: &str) -> Result<Self, ProfileError> {
        let text = match name {
            "linux6" => LINUX6,
            "macos14" => MACOS14,
            "windows11" => WINDOWS11,
            _ => {
                return Err(ProfileError::Unknown {
                    name: name.to_owned(),
                })
            }
        };
        Self::from_yaml(text)
    }

    /// Parses and validates a profile in the YAML format of the files in
    /// `profiles/`.
    pub fn from_yaml(text: &str) -> Result<Self, ProfileError> {
        let profile: Self = serde_norway::from_str(text)?;
        profile.validate()?;
        Ok(profile)
    }

    fn validate(&self) -> Result<(), ProfileError> {
        let invalid = |field, problem| ProfileError::Invalid {
            name: self.name.clone(),
            field,
            problem,
        };
        let name_ok = self
            .name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase())
            && self
                .name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !name_ok {
            return Err(invalid("name", "must match ^[a-z][a-z0-9_]*$"));
        }
        if self.socket.ttl == 0 {
            return Err(invalid("socket.ttl", "must be from 1 to 255"));
        }
        if !(MIN_MSS..=MAX_MSS).contains(&self.socket.mss) {
            return Err(invalid("socket.mss", "must be from 88 to 65495"));
        }
        if self.reference.window_scale > 14 {
            return Err(invalid("reference.window_scale", "must be from 0 to 14"));
        }
        const KINDS: [&str; 6] = ["mss", "nop", "ws", "sok", "ts", "eol"];
        if self
            .reference
            .options
            .iter()
            .any(|o| !KINDS.contains(&o.as_str()))
        {
            return Err(invalid(
                "reference.options",
                "may hold only mss, nop, ws, sok, ts, and eol",
            ));
        }
        Ok(())
    }

    /// Returns `base` with the TTL, MSS, and window clamp of the profile.
    ///
    /// The other fields of `base` stay unchanged, so a caller combines a
    /// profile with its own buffer, Nagle, and linger settings.
    pub fn socket_options(&self, base: SocketOptions) -> SocketOptions {
        SocketOptions {
            ttl: Some(self.socket.ttl),
            mss: Some(self.socket.mss),
            window_clamp: self.socket.window_clamp,
            ..base
        }
    }

    /// Returns the sysctls of the network namespace that the system settings
    /// of the profile need, as name and value pairs in the format of
    /// `sysctl -w name=value`.
    ///
    /// The minimum and default of `net.ipv4.tcp_rmem` keep the Linux defaults
    /// of 4096 and 131072 bytes.
    pub fn sysctls(&self) -> Vec<(&'static str, String)> {
        let flag = |on: bool| if on { "1" } else { "0" }.to_owned();
        vec![
            ("net.ipv4.tcp_sack", flag(self.system.sack)),
            ("net.ipv4.tcp_timestamps", flag(self.system.timestamps)),
            (
                "net.ipv4.tcp_window_scaling",
                flag(self.system.window_scaling),
            ),
            (
                "net.ipv4.tcp_rmem",
                format!("4096 131072 {}", self.system.tcp_rmem_max),
            ),
        ]
    }

    /// Compares the sysctls of the profile with the values that `read`
    /// returns and lists every sysctl that differs.
    ///
    /// `read` takes a sysctl name such as `net.ipv4.tcp_sack` and returns its
    /// value, for example from `/proc/sys/net/ipv4/tcp_sack`. Values compare
    /// after collapsing white space, and a failed read counts as a
    /// difference.
    pub fn sysctl_mismatches<F>(&self, mut read: F) -> Vec<SysctlMismatch>
    where
        F: FnMut(&str) -> std::io::Result<String>,
    {
        let normal = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        self.sysctls()
            .into_iter()
            .filter_map(|(name, want)| {
                let found = read(name).map(|v| normal(&v)).ok();
                (found.as_deref() != Some(want.as_str())).then_some(SysctlMismatch {
                    name,
                    want,
                    found,
                })
            })
            .collect()
    }
}

/// A sysctl whose value differs from what a profile needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysctlMismatch {
    /// The name of the sysctl, for example `net.ipv4.tcp_timestamps`.
    pub name: &'static str,
    /// The value that the profile needs.
    pub want: String,
    /// The value found, or `None` when the read failed.
    pub found: Option<String>,
}

/// Reads a sysctl of the current network namespace from `/proc/sys`.
///
/// It suits [`OsProfile::sysctl_mismatches`].
pub fn read_proc_sysctl(name: &str) -> std::io::Result<String> {
    let path = format!("/proc/sys/{}", name.replace('.', "/"));
    std::fs::read_to_string(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_profile_loads_under_its_own_name() {
        for name in BUILTIN_PROFILES {
            let profile = OsProfile::builtin(name).unwrap();
            assert_eq!(profile.name, name);
            assert_eq!(profile.socket.mss, 1460);
        }
    }

    #[test]
    fn builtin_profiles_carry_the_ttl_of_their_system() {
        let ttl = |n| OsProfile::builtin(n).unwrap().socket.ttl;
        assert_eq!(ttl("windows11"), 128);
        assert_eq!(ttl("linux6"), 64);
        assert_eq!(ttl("macos14"), 64);
    }

    #[test]
    fn window_clamp_and_rmem_give_the_reference_window_scale() {
        // Linux computes the scale as ilog2(min(max(rcvbuf space, tcp_rmem
        // max), window clamp)) - 15, clamped to 0..=14, see
        // tcp_select_initial_window. The default receive buffer space of
        // 64 KiB is below every tcp_rmem maximum of the profiles.
        for name in BUILTIN_PROFILES {
            let p = OsProfile::builtin(name).unwrap();
            let space = p
                .socket
                .window_clamp
                .map_or(p.system.tcp_rmem_max, |c| c.min(p.system.tcp_rmem_max));
            let scale = (space.ilog2().saturating_sub(15)).min(14);
            assert_eq!(scale, u32::from(p.reference.window_scale), "{name}");
        }
    }

    #[test]
    fn unknown_profile_names_the_builtin_ones() {
        let err = OsProfile::builtin("beos5").unwrap_err();
        assert!(err.to_string().contains("beos5"));
        assert!(err.to_string().contains("windows11"));
    }

    #[test]
    fn invalid_fields_are_rejected_with_their_name() {
        let good = WINDOWS11;
        let cases = [
            ("ttl: 128", "ttl: 0", "socket.ttl"),
            ("mss: 1460", "mss: 40", "socket.mss"),
            (
                "window_scale: 8",
                "window_scale: 15",
                "reference.window_scale",
            ),
            ("[mss, nop", "[mss, wscale", "reference.options"),
            ("name: windows11", "name: Windows11", "name"),
        ];
        for (from, to, field) in cases {
            let text = good.replacen(from, to, 1);
            assert_ne!(text, good, "{from} not found");
            match OsProfile::from_yaml(&text) {
                Err(ProfileError::Invalid { field: f, .. }) => assert_eq!(f, field),
                other => panic!("{field}: {other:?}"),
            }
        }
    }

    #[test]
    fn unknown_keys_and_wrong_types_fail_to_parse() {
        let extra = WINDOWS11.replacen("  ttl: 128", "  ttl: 128\n  tos: 0", 1);
        assert!(matches!(
            OsProfile::from_yaml(&extra),
            Err(ProfileError::Parse(_))
        ));
        let wide = WINDOWS11.replacen("ttl: 128", "ttl: 300", 1);
        assert!(matches!(
            OsProfile::from_yaml(&wide),
            Err(ProfileError::Parse(_))
        ));
    }

    #[test]
    fn socket_options_keep_the_base_and_add_the_profile() {
        let base = SocketOptions {
            recv_buffer: Some(1 << 16),
            nodelay: false,
            ..SocketOptions::default()
        };
        let opts = OsProfile::builtin("macos14").unwrap().socket_options(base);
        assert_eq!(opts.ttl, Some(64));
        assert_eq!(opts.mss, Some(1460));
        assert_eq!(opts.window_clamp, Some(3_145_728));
        assert_eq!(opts.recv_buffer, Some(1 << 16));
        assert!(!opts.nodelay);
    }

    #[test]
    fn sysctls_follow_the_system_settings() {
        let p = OsProfile::builtin("windows11").unwrap();
        assert_eq!(
            p.sysctls(),
            vec![
                ("net.ipv4.tcp_sack", "1".to_owned()),
                ("net.ipv4.tcp_timestamps", "0".to_owned()),
                ("net.ipv4.tcp_window_scaling", "1".to_owned()),
                ("net.ipv4.tcp_rmem", "4096 131072 16777216".to_owned()),
            ]
        );
    }

    #[test]
    fn sysctl_mismatches_list_differences_and_failed_reads() {
        let p = OsProfile::builtin("windows11").unwrap();
        let found = p.sysctl_mismatches(|name| match name {
            "net.ipv4.tcp_sack" => Ok("1\n".to_owned()),
            "net.ipv4.tcp_timestamps" => Ok("1\n".to_owned()),
            "net.ipv4.tcp_rmem" => Ok("4096\t131072\t16777216\n".to_owned()),
            _ => Err(std::io::ErrorKind::NotFound.into()),
        });
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "net.ipv4.tcp_timestamps");
        assert_eq!(found[0].found.as_deref(), Some("1"));
        assert_eq!(found[1].name, "net.ipv4.tcp_window_scaling");
        assert_eq!(found[1].found, None);
    }
}
