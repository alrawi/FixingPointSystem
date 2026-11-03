// SPDX-License-Identifier: GPL-3.0-only
//! Scenario model, loader, and template compiler.
//!
//! [`program`] checks a scenario file with the semantic rules of
//! `tools/contracts/validate.py` and compiles it into flat programs, one per
//! flow side, and [`template`] compiles and renders the template grammar of
//! contract v0.2. Module template functions dispatch through the
//! [`TemplateFn`] trait in [`function`], and [`manifest`] reads the function
//! catalogs that module calls are checked against. Loader errors carry a
//! typed [`Location`]. The interpreter in `fps-interp` runs the programs.
//!
//! [`Scenario::check`] accepts every scenario that the contract allows.
//! [`Scenario::load`] also rejects transport modules, spawned flows, and the
//! `spawn` and `raw` actions, which later ENG packages add.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod check;
mod error;
pub mod function;
pub mod location;
pub mod manifest;
pub mod program;
pub mod template;

pub use check::MAX_DATAGRAM;
pub use function::{ArgValue, CallArgs, FnCall, FnError, FnErrorKind, TemplateFn, ValueType};
pub use location::{Location, Side, Step};
pub use manifest::{Manifest, ManifestError, ModuleContext, ModuleSet};
pub use program::{LoadError, Scenario, Warning};
pub use template::{Template, TemplateError, Value, Vars};
