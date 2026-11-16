// SPDX-License-Identifier: GPL-3.0-only
//! TCP stack bench harness of the engine.
//!
//! The harness drives a user space TCP stack behind [`fps_io::SessionIo`], the
//! seam that the interpreter uses, and measures three things for the decision
//! record of TRN-10.
//!
//! 1. Sessions per second, with a small request and response per session.
//! 2. Throughput of bulk transfers over concurrent sessions.
//! 3. Scaling of the stack toward 1,000,000 sockets, with the memory per
//!    socket, the cost of one poll with idle sockets, and the session rate
//!    that remains while the sockets are held.
//!
//! A stack plugs in through [`TcpStack`], so the own stack of TRN-18 runs the
//! same workloads. [`smol::SmolStack`] is the first stack. The binary
//! `examples/tcp_bench.rs` runs the workloads with plain command line
//! arguments and writes an `fps-bench/v1` document.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod cli;
pub mod clock;
pub mod link;
pub mod report;
pub mod smol;
pub mod stack;
pub mod sysinfo;
pub mod workload;

pub use clock::{Clock, ManualClock, MonotonicClock};
pub use link::LinkConfig;
pub use report::BenchReport;
pub use smol::{SmolConfig, SmolSession, SmolStack, TimeWait};
pub use stack::{StackStats, TcpStack};
pub use workload::{
    session_rate, socket_scaling, throughput, BenchError, Direction, RateConfig, RateReport,
    ScaleConfig, ScaleReport, ThroughputConfig, ThroughputReport,
};
