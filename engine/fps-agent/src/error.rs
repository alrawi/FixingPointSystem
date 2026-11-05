// SPDX-License-Identifier: GPL-3.0-only
//! Errors of capability detection and of the startup kernel check.

use std::path::PathBuf;

/// Errors that capability detection reports for one file under the system root.
#[derive(Debug, thiserror::Error)]
pub enum DetectError {
    /// A file or directory that detection needs could not be read.
    #[error("read {path}: io error")]
    Read {
        /// The full path, including the system root.
        path: PathBuf,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
    /// A file held a value that detection could not parse.
    #[error("parse {path}: invalid {what} {value:?}")]
    Parse {
        /// The full path, including the system root.
        path: PathBuf,
        /// The kind of value, for example `cpu list`.
        what: &'static str,
        /// The offending text, truncated to 64 bytes.
        value: String,
    },
    /// No source gave the number of online CPUs.
    #[error("cores: neither {online} nor {cpuinfo} lists a cpu")]
    NoCpus {
        /// The path of the online CPU list.
        online: PathBuf,
        /// The path of the cpuinfo fallback.
        cpuinfo: PathBuf,
    },
}

impl DetectError {
    /// Builds a [`DetectError::Parse`] and truncates long values.
    pub(crate) fn parse(path: PathBuf, what: &'static str, value: &str) -> Self {
        let value = value.trim();
        let mut end = value.len().min(64);
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        Self::Parse {
            path,
            what,
            value: value[..end].to_owned(),
        }
    }
}

/// Errors of the startup kernel check.
#[derive(Debug, thiserror::Error)]
pub enum KernelError {
    /// The kernel is older than the first release with `IP_LOCAL_PORT_RANGE`.
    #[error(
        "kernel {release} is too old: the agent needs the IP_LOCAL_PORT_RANGE socket option, \
         which requires Linux {}.{} or newer (decision D12 item 3)",
        .required.0, .required.1
    )]
    TooOld {
        /// The release string of the running kernel, for example `5.15.0-91-generic`.
        release: String,
        /// The least major and minor version that the agent accepts.
        required: (u32, u32),
    },
    /// The kernel release could not be read.
    #[error("read the kernel release for the IP_LOCAL_PORT_RANGE check")]
    Detect(#[from] DetectError),
}
