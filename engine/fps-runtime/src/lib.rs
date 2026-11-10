// SPDX-License-Identifier: GPL-3.0-only
//! Worker runtime of the engine.
//!
//! The runtime runs one single threaded Tokio runtime per worker thread and
//! optionally pins each thread to one core. Workers share nothing mutable.
//! Each worker owns its sessions, its timers, its [`fps_stats::Shard`], its
//! filler, and either its schedule or its listeners, and reads only immutable
//! data such as the compiled scenario.
//!
//! A client worker drains an [`fps_sched::Schedule`] with a [`TokioClock`]
//! and runs the client side of each session start with `fps-interp` over a
//! kernel [`fps_backend_kernel::Connector`], taking source addresses from a
//! host pool through an [`fps_hosts::HostSelector`]. A server worker accepts
//! sessions on kernel listeners and runs the server side of each one. Every
//! worker sends a cumulative [`WorkerSnapshot`] once per snapshot interval,
//! one second by default, and a final one when it ends. The [`Collector`]
//! keeps the newest snapshot of every worker and merges them.
//!
//! A module that ships `extension.wasm` runs through an [`ExtensionSet`], which
//! compiles each component once, shares it across the workers, and gives every
//! session its own instance, see [`FlowRef::extensions`].
//!
//! A flow that upgrades its transport with `tls_upgrade` resolves the profile of
//! the action in [`RuntimeConfig::tls`], see [`TlsProfiles`]. The upgrade wraps the
//! session in TLS below the interpreter and above the capture tap, so a capture
//! records the encrypted bytes.
//!
//! [`RuntimeHandle::stop`] stops new sessions and lets open sessions finish
//! within the ramp down time, after which it resets the rest.
//! [`RuntimeHandle::abort`] resets every open session at once.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod clock;
mod collector;
mod config;
mod control;
mod error;
mod extension;
mod instance;
mod metrics;
mod opener;
mod runtime;
mod tls;
mod worker;

pub use clock::TokioClock;
pub use collector::{Collector, WorkerSnapshot};
pub use config::{
    ClientConfig, FlowRef, FunctionFactory, Role, RuntimeConfig, ServerConfig, SourceHosts,
    WorkerConfig,
};
pub use error::RuntimeError;
pub use extension::{
    close_reason, extension_rng, ExtensionError, ExtensionSet, SessionExtensions, SessionKey,
    EXTENSION_FILE,
};
pub use fps_wasm::Transport;
pub use opener::{global_session_id, ClientOpener};
pub use runtime::{RunReport, Runtime, RuntimeHandle, WorkerExit};
pub use tls::{ConnectorFactory, TlsProfiles, DEFAULT_SERVER_NAME};
