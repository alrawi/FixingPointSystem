// SPDX-License-Identifier: GPL-3.0-only
//! Scenario model, loader, and template compiler.
//!
//! [`program`] compiles a scenario file into flat programs, one per flow side,
//! and [`template`] compiles and renders the template grammar of contract
//! v0.1. The interpreter in `fps-interp` runs the programs.
//!
//! The crate starts from the reusable parts of spike S1 and keeps its scope.
//! It rejects transport modules, spawned flows, the `spawn` and `raw` actions,
//! and module helper functions, which later ENG packages add.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod program;
pub mod template;

pub use program::{LoadError, Scenario};
pub use template::{Template, TemplateError, Value, Vars};
