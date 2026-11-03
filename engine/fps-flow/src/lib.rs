// SPDX-License-Identifier: GPL-3.0-only
//! Scenario model, loader, and template compiler.
//!
//! [`program`] compiles a scenario file into flat programs, one per flow side,
//! and [`template`] compiles and renders the template grammar of contract
//! v0.2. Module template functions dispatch through the [`TemplateFn`] trait
//! in [`function`]. The interpreter in `fps-interp` runs the programs.
//!
//! The crate rejects transport modules, spawned flows, and the `spawn` and
//! `raw` actions, which later ENG packages add.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod function;
pub mod program;
pub mod template;

pub use function::{ArgValue, CallArgs, FnCall, FnError, FnErrorKind, TemplateFn, ValueType};
pub use program::{LoadError, Scenario};
pub use template::{Template, TemplateError, Value, Vars};
