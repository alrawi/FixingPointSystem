// SPDX-License-Identifier: GPL-3.0-only
//! The FixingPointSystem agent.
//!
//! This crate holds the agent side of the control protocol in
//! `contracts/proto/fps/agent/v1/agent.proto`. Today it detects host
//! capabilities. [`capabilities::detect`] reads `/proc` and `/sys` under a
//! [`sysroot::SystemRoot`], and [`capabilities::HostCapabilities::to_capabilities`]
//! fills [`proto::Capabilities`], the Rust mirror of the proto `Capabilities`
//! message.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod capabilities;
pub mod error;
pub mod proto;
pub mod sysroot;

pub use capabilities::{detect, HostCapabilities};
pub use error::DetectError;
pub use sysroot::SystemRoot;
