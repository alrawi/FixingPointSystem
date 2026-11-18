// SPDX-License-Identifier: GPL-3.0-only
//! Operating system profiles of the own stack (TRN-28).
//!
//! A profile names the SYN of one operating system: the TTL, the MSS, the
//! window field and the window scale of the SYN, which options the host
//! offers, and in which order it writes them. The kernel backend applies the
//! same table through socket options and sysctls
//! (`engine/fps-backend-kernel/profiles/` and the Transport page of the
//! wiki), and it cannot set the option order of Windows 11 and macOS 14, because the
//! Linux kernel writes a fixed order. The own stack writes its own frames, so
//! it reproduces every row of the table.
//!
//! A host with a profile ([`crate::OwnConfig::client_os_profile`] and
//! [`crate::OwnConfig::server_os_profile`]) differs from a host without one
//! in five ways, all of which show in the SYN and the SYN-ACK:
//!
//! * every IP packet carries the TTL of the profile;
//! * the MSS option is the profile's MSS, lowered to the link MTU less 40
//!   bytes (the kernel's `TCP_MAXSEG` also only lowers);
//! * the window field is the profile's `syn_window`, lowered to the receive
//!   buffer, and the window scale option is the profile's `window_scale`;
//! * the host offers window scaling, timestamps, and SACK as the profile says,
//!   and the profile wins over the `window_scale`, `timestamps`, and `sack`
//!   fields of the configuration;
//! * the options are written in the order of the profile, and a SYN-ACK
//!   answers only the options that the peer offered, in the same order.

use crate::wire::SynOption::{EndOfList, Mss, Nop, SackPermitted, Timestamp, WindowScale};
use crate::wire::{Style, SynOption, MAX_WSCALE, OPTIONS_MAX};

/// The names of the built in profiles, in alphabetical order.
pub const BUILTIN_PROFILES: [&str; 3] = ["linux6", "macos14", "windows11"];

/// Errors of choosing or checking a profile.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    /// No built in profile has the name.
    #[error("unknown OS profile {name}, the built in profiles are linux6, macos14, and windows11")]
    Unknown {
        /// The requested name.
        name: String,
    },
    /// A field holds a value that the stack cannot write.
    #[error("OS profile {name}: {field} {problem}")]
    Invalid {
        /// The name of the profile.
        name: &'static str,
        /// The field, for example `ttl`.
        field: &'static str,
        /// What is wrong with the value.
        problem: &'static str,
    },
}

/// The SYN of one operating system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OsProfile {
    /// The name that test configurations use in `os_profile`.
    pub name: &'static str,
    /// The IP TTL of every packet.
    pub ttl: u8,
    /// The MSS option of the SYN and the SYN-ACK.
    pub mss: u16,
    /// The window field of the SYN and the SYN-ACK, which is never scaled.
    pub syn_window: u16,
    /// The shift count of the window scale option, from 0 to 14.
    pub window_scale: u8,
    /// Whether the host offers window scaling. When it is off, the option is
    /// left out and the window is at most 65,535 bytes.
    pub window_scaling: bool,
    /// Whether the host offers SACK.
    pub sack: bool,
    /// Whether the host offers timestamps.
    pub timestamps: bool,
    /// The options of a SYN in wire order. The list is also the order of a
    /// SYN-ACK, with the options that the peer did not offer left out.
    pub options: &'static [SynOption],
}

/// The option order of the Linux kernel in a SYN with every option.
const KERNEL_ORDER: &[SynOption] = &[Mss, SackPermitted, Timestamp, Nop, WindowScale];

/// Windows 11: TTL 128, the window scale 8, no timestamps.
pub const WINDOWS11: OsProfile = OsProfile {
    name: "windows11",
    ttl: 128,
    mss: 1460,
    syn_window: 64240,
    window_scale: 8,
    window_scaling: true,
    sack: true,
    timestamps: false,
    options: &[Mss, Nop, WindowScale, Nop, Nop, SackPermitted],
};

/// Linux 6: the defaults of the kernel.
pub const LINUX6: OsProfile = OsProfile {
    name: "linux6",
    ttl: 64,
    mss: 1460,
    syn_window: 64240,
    window_scale: 7,
    window_scaling: true,
    sack: true,
    timestamps: true,
    options: KERNEL_ORDER,
};

/// macOS 14: the window 65,535, the window scale 6, and an end of list byte.
pub const MACOS14: OsProfile = OsProfile {
    name: "macos14",
    ttl: 64,
    mss: 1460,
    syn_window: 65535,
    window_scale: 6,
    window_scaling: true,
    sack: true,
    timestamps: true,
    options: &[
        Mss,
        Nop,
        WindowScale,
        Nop,
        Nop,
        Timestamp,
        SackPermitted,
        EndOfList,
    ],
};

impl OsProfile {
    /// Returns the built in profile with the name.
    pub fn builtin(name: &str) -> Result<Self, ProfileError> {
        match name {
            "linux6" => Ok(LINUX6),
            "macos14" => Ok(MACOS14),
            "windows11" => Ok(WINDOWS11),
            _ => Err(ProfileError::Unknown {
                name: name.to_owned(),
            }),
        }
    }

    /// Checks that the stack can write the profile: a TTL above 0, a window
    /// scale up to 14, an MSS that fits a frame, and options that fit the 40
    /// bytes of a TCP header when the host offers every one of them.
    pub fn validate(&self) -> Result<(), ProfileError> {
        let invalid = |field, problem| ProfileError::Invalid {
            name: self.name,
            field,
            problem,
        };
        if self.ttl == 0 {
            return Err(invalid("ttl", "must be from 1 to 255"));
        }
        if self.mss < 64 {
            return Err(invalid("mss", "must be at least 64"));
        }
        if self.window_scale > MAX_WSCALE {
            return Err(invalid("window_scale", "must be from 0 to 14"));
        }
        if self.options.is_empty() {
            return Err(invalid("options", "must not be empty"));
        }
        let size: usize = self
            .options
            .iter()
            .map(|o| match o {
                Mss => 4,
                WindowScale => 3,
                SackPermitted => 2,
                Timestamp => 10,
                Nop | EndOfList => 1,
            })
            .sum();
        if size > OPTIONS_MAX {
            return Err(invalid("options", "take more than 40 bytes"));
        }
        Ok(())
    }

    /// The way that a host with this profile writes its frames.
    ///
    /// The order of the Linux kernel is the writer that the stack has always
    /// had, and it pads a SYN-ACK that leaves an option out as the kernel
    /// does (`nop nop sok`), so a profile with that order keeps it.
    pub fn style(&self) -> Style {
        Style {
            ttl: self.ttl,
            syn_order: if self.options == KERNEL_ORDER {
                &[]
            } else {
                self.options
            },
        }
    }

    /// The option names of the SYN in the notation of the OS profile table,
    /// for example `["mss", "sok", "ts", "nop", "ws"]`.
    pub fn option_names(&self) -> Vec<&'static str> {
        self.options.iter().map(|o| o.name()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builtin_profiles_are_valid_and_found_by_name() {
        for name in BUILTIN_PROFILES {
            let profile = OsProfile::builtin(name).expect("a built in profile");
            assert_eq!(profile.name, name);
            profile.validate().expect("valid");
        }
        assert!(matches!(
            OsProfile::builtin("beos"),
            Err(ProfileError::Unknown { .. })
        ));
    }

    #[test]
    fn validation_names_the_field_that_is_out_of_range() {
        let field = |p: OsProfile| match p.validate() {
            Err(ProfileError::Invalid { field, .. }) => field,
            other => panic!("expected an error, got {other:?}"),
        };
        assert_eq!(field(OsProfile { ttl: 0, ..LINUX6 }), "ttl");
        assert_eq!(field(OsProfile { mss: 10, ..LINUX6 }), "mss");
        assert_eq!(
            field(OsProfile {
                window_scale: 15,
                ..LINUX6
            }),
            "window_scale"
        );
        assert_eq!(
            field(OsProfile {
                options: &[],
                ..LINUX6
            }),
            "options"
        );
        assert_eq!(
            field(OsProfile {
                options: &[Mss, Timestamp, Timestamp, Timestamp, Timestamp],
                ..LINUX6
            }),
            "options"
        );
    }

    #[test]
    fn the_option_names_follow_the_table() {
        assert_eq!(LINUX6.option_names(), ["mss", "sok", "ts", "nop", "ws"]);
        assert_eq!(
            WINDOWS11.option_names(),
            ["mss", "nop", "ws", "nop", "nop", "sok"]
        );
        assert_eq!(
            MACOS14.option_names(),
            ["mss", "nop", "ws", "nop", "nop", "ts", "sok", "eol"]
        );
    }
}
