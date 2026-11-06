// SPDX-License-Identifier: GPL-3.0-only
//! WebAssembly extension host.
//!
//! Community modules ship computation that flow programs cannot express as a
//! WebAssembly component that targets the `extension` world of
//! `contracts/extension/fps-extension.wit` (decision D3). [`WasmExtension`]
//! loads such a component, and [`WasmSession`] runs the hooks of one session.
//! [`WasmFunctions`] exposes the template functions through
//! [`fps_flow::TemplateFn`], the trait that native extensions use.
//!
//! Every session owns its instance. A trap, a fuel exhaustion, or a memory
//! limit therefore ends that session only, as the contract requires. Each hook
//! call gets a fresh fuel budget from [`Limits::fuel_per_call`], and an
//! instance may hold at most [`Limits::max_memory_bytes`] of linear memory.
//! Every failure is a [`WasmFailure`] with the catalog reason
//! `extension_error`.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod extension;
mod failure;
mod functions;
mod host;

pub use extension::{
    CloseReason, DataOutcome, FunctionDesc, Limits, OwnedValue, ParamDesc, SessionInfo, Side,
    Transport, WasmEngine, WasmExtension, WasmSession,
};
pub use failure::WasmFailure;
pub use functions::WasmFunctions;
pub use host::{EngineError, Host, LogLevel, TimerId};
