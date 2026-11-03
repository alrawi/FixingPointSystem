// SPDX-License-Identifier: GPL-3.0-only
//! Errors of the runtime.

use std::io;

use fps_backend_kernel::BackendError;
use fps_sched::SchedError;
use fps_stats::StatsError;

/// Errors that stop a runtime from starting or a worker from running.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// The configuration is invalid.
    #[error("runtime config: {detail}")]
    Config {
        /// What is wrong.
        detail: String,
    },
    /// The schedule of a client worker is invalid.
    #[error("worker {worker}: schedule")]
    Schedule {
        /// The worker id.
        worker: u32,
        /// The scheduler error.
        #[source]
        source: SchedError,
    },
    /// A server worker could not listen.
    #[error("worker {worker}: listen")]
    Listen {
        /// The worker id.
        worker: u32,
        /// The backend error.
        #[source]
        source: BackendError,
    },
    /// A worker could not register its statistics series.
    #[error("worker {worker}: statistics")]
    Stats {
        /// The worker id.
        worker: u32,
        /// The statistics error.
        #[source]
        source: StatsError,
    },
    /// The worker thread or its Tokio runtime could not start.
    #[error("worker {worker}: start thread")]
    Thread {
        /// The worker id.
        worker: u32,
        /// The operating system error.
        #[source]
        source: io::Error,
    },
    /// A worker thread panicked or ended before it reported.
    #[error("worker {worker}: thread ended unexpectedly")]
    Lost {
        /// The worker id.
        worker: u32,
    },
}
