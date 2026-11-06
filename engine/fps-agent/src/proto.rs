// SPDX-License-Identifier: GPL-3.0-only
//! Rust mirrors of the proto messages that capability detection fills.
//!
//! Each type mirrors one message or enum of
//! `contracts/proto/fps/agent/v1/agent.proto` field for field, with the proto
//! field name as the Rust field name. The table below gives the mapping.
//! Capability detection fills these hand written mirrors, and
//! [`crate::convert::capabilities_to_pb`] copies them into the generated
//! types of [`crate::pb`] for `Register`. The contract test in
//! `tests/contract.rs` reads the proto file and fails when the two drift.
//!
//! | Proto | Rust |
//! |---|---|
//! | `string` | `String` |
//! | `uint32` | `u32` |
//! | `uint64` | `u64` |
//! | `bytes` | `Vec<u8>` |
//! | `repeated T` | `Vec<T>` |
//! | `map<string, string>` | `BTreeMap<String, String>`, so output order is stable |
//! | `enum Backend` | [`Backend`], with the proto numbers as discriminants |

use std::collections::BTreeMap;

/// Mirrors `fps.agent.v1.Backend`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(i32)]
pub enum Backend {
    /// `BACKEND_UNSPECIFIED`, which is never sent.
    #[default]
    Unspecified = 0,
    /// `BACKEND_KERNEL`, kernel sockets.
    Kernel = 1,
    /// `BACKEND_AF_XDP`, AF_XDP sockets.
    AfXdp = 2,
    /// `BACKEND_DPDK`, DPDK poll mode drivers.
    Dpdk = 3,
}

impl Backend {
    /// Returns the proto enum value name, for example `BACKEND_AF_XDP`.
    pub fn proto_name(self) -> &'static str {
        match self {
            Self::Unspecified => "BACKEND_UNSPECIFIED",
            Self::Kernel => "BACKEND_KERNEL",
            Self::AfXdp => "BACKEND_AF_XDP",
            Self::Dpdk => "BACKEND_DPDK",
        }
    }

    /// Returns the proto enum number.
    pub fn number(self) -> i32 {
        self as i32
    }
}

/// Mirrors `fps.agent.v1.Capabilities`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Capabilities {
    /// Field 1, the agent build version.
    pub agent_version: String,
    /// Field 2, the contract version that the agent implements.
    pub engine_api: String,
    /// Field 3, the transport backends that the agent supports.
    pub backends: Vec<Backend>,
    /// Field 4, the number of workers that the agent can run.
    pub cores: u32,
    /// Field 5, the memory available for sessions in bytes.
    pub memory_bytes: u64,
    /// Field 6, the traffic interfaces.
    pub ports: Vec<PortInfo>,
    /// Field 7, the installed modules with their digests.
    pub modules: Vec<ModuleRef>,
    /// Field 8, operator assigned key and value pairs.
    pub labels: BTreeMap<String, String>,
}

/// Mirrors `fps.agent.v1.PortInfo`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PortInfo {
    /// Field 1, the interface name, such as `eth1`.
    pub name: String,
    /// Field 2, the kernel or poll mode driver name.
    pub driver: String,
    /// Field 3, the link speed, or 0 when the kernel does not report one.
    pub speed_mbps: u32,
    /// Field 4, the hardware address in lowercase colon notation.
    pub mac: String,
    /// Field 5, the configured MTU.
    pub mtu: u32,
}

/// Mirrors `fps.agent.v1.ModuleRef`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModuleRef {
    /// Field 1, the module name from its manifest.
    pub name: String,
    /// Field 2, the module version from its manifest.
    pub version: String,
    /// Field 3, `sha256:` followed by 64 lowercase hex digits.
    pub digest: String,
    /// Field 4, the Ed25519 signature over the digest, or empty.
    pub signature: Vec<u8>,
    /// Field 5, the registry key name of the signature, or empty.
    pub signature_key_id: String,
}
