// SPDX-License-Identifier: GPL-3.0-only
//! The FixingPointSystem agent.
//!
//! This crate holds the agent side of the control protocol in
//! `contracts/proto/fps/agent/v1/agent.proto`. It has three parts.
//!
//! 1. Capability detection. [`capabilities::detect`] reads `/proc` and
//!    `/sys` under a [`sysroot::SystemRoot`], and
//!    [`capabilities::HostCapabilities::to_capabilities`] fills
//!    [`proto::Capabilities`], the Rust mirror of the proto `Capabilities`
//!    message.
//! 2. The standalone mode. [`run::run`] executes a [`plan::Plan`] in its
//!    canonical JSON mapping without a controller. [`modules`] loads the
//!    plan modules with digest checks and registers their native template
//!    functions, [`assemble`] turns the plan into engine workers, `fps-runtime`
//!    runs the TCP flows, [`udp`] runs the UDP flows over the datagram
//!    openers, and [`criteria`] evaluates the pass and fail rules.
//!    [`trust`] verifies the signature of every module digest against the
//!    trust store of the agent (AGT-04).
//! 3. The controller session. [`session::serve`] dials the controller,
//!    registers, and serves `prepare`, `start`, and `stop` on the session
//!    stream with the run path of the standalone mode. [`pb`] holds the
//!    generated messages and stubs, and [`convert`] maps them to the types
//!    of the run path.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod assemble;
pub mod capabilities;
pub mod control;
pub mod convert;
pub mod criteria;
pub mod error;
pub mod identity;
pub mod modules;
pub mod pb;
pub mod plan;
pub mod portfilter;
pub mod proto;
pub mod run;
pub mod session;
pub mod sysroot;
pub mod tls;
pub mod trust;
pub mod udp;

pub use capabilities::{detect, require_kernel, HostCapabilities};
pub use error::{DetectError, KernelError};
pub use run::{run, Outcome, RunError, RunOptions};
pub use sysroot::SystemRoot;
