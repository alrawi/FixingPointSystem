// SPDX-License-Identifier: GPL-3.0-only
//! Scenario model, loader, and template compiler.
//!
//! [`program`] checks a scenario file with the semantic rules of
//! `tools/contracts/validate.py` and compiles it into flat programs, one per
//! flow side, and [`template`] compiles and renders the template grammar of
//! contract v0.3. Module template functions dispatch through the
//! [`TemplateFn`] trait in [`function`], and [`manifest`] reads the function
//! catalogs that module calls are checked against. Loader errors carry a
//! typed [`Location`]. The interpreter in `fps-interp` runs the programs.
//!
//! [`Scenario::check`] accepts every scenario that the contract allows.
//! [`Scenario::load`] also rejects spawned flows and the `spawn` action, which
//! the scenario instance runner runs. The `raw` action compiles to a send of
//! its bytes without template parsing, and the contract check admits it only in
//! modules of type `strike` and `fuzzer` (`LoadError::ReservedAction`). The flow keyword
//! `transport` is sugar for a leading `tls_upgrade` on each side, and the loader desugars it.
//! [`Scenario::parse_instance`] accepts spawned flows and the `spawn` action
//! for the scenario instance runner of `fps-interp`.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod check;
pub mod clock;
mod error;
pub mod function;
pub mod location;
pub mod manifest;
pub mod program;
pub mod template;

pub use check::MAX_DATAGRAM;
pub use clock::{Clock, PinnedClock, SystemClock};
pub use function::{ArgValue, CallArgs, FnCall, FnError, FnErrorKind, TemplateFn, ValueType};
pub use location::{Location, Side, Step};
pub use manifest::{Manifest, ManifestError, ModuleContext, ModuleSet};
pub use program::{LoadError, Scenario, Warning};
pub use template::{Profiles, Template, TemplateError, Value, Vars};
