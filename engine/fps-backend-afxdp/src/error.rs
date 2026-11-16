// SPDX-License-Identifier: GPL-3.0-only
//! Errors of port and queue setup.

use std::io;

use crate::config::ConfigError;

/// A failure while opening a port or one of its queues.
#[derive(Debug, thiserror::Error)]
pub enum AfXdpError {
    /// A setting was refused before any system call.
    #[error("invalid port settings")]
    Config(#[from] ConfigError),
    /// A system call failed. The message names the step, the interface, and
    /// the queue, and the operating system error stays behind `source`.
    #[error("{step} failed on {interface}{}", queue_suffix(.queue))]
    Syscall {
        /// The step that failed, for example `bind` or `XDP_UMEM_REG`.
        step: &'static str,
        /// The interface of the port.
        interface: String,
        /// The queue, or `None` for a step that belongs to the whole port.
        queue: Option<u32>,
        /// The operating system error.
        #[source]
        source: io::Error,
    },
    /// The kernel refused the XDP program, and the verifier log says why.
    #[error("load the XDP program on {interface}: {detail}")]
    Program {
        /// The interface of the port.
        interface: String,
        /// The error of the `bpf` call and the start of the verifier log.
        detail: String,
    },
}

fn queue_suffix(queue: &Option<u32>) -> String {
    queue.map_or_else(String::new, |q| format!(" queue {q}"))
}

impl AfXdpError {
    /// The kind of the operating system error behind the failure, if any.
    #[must_use]
    pub fn io_kind(&self) -> Option<io::ErrorKind> {
        match self {
            Self::Syscall { source, .. } => Some(source.kind()),
            _ => None,
        }
    }

    /// The raw `errno` behind the failure, if any.
    #[must_use]
    pub fn raw_os_error(&self) -> Option<i32> {
        match self {
            Self::Syscall { source, .. } => source.raw_os_error(),
            _ => None,
        }
    }

    /// Whether the step failed because the driver offers no zero copy mode,
    /// which a request for [`crate::BindMode::ZeroCopy`] meets on a device
    /// such as a veth pair.
    #[must_use]
    pub fn is_zero_copy_unsupported(&self) -> bool {
        matches!(
            self,
            Self::Syscall { step: "bind", source, .. }
                if matches!(
                    source.raw_os_error(),
                    Some(libc::EOPNOTSUPP | libc::ENODEV | libc::EINVAL)
                )
        )
    }
}
