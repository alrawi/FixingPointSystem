// SPDX-License-Identifier: GPL-3.0-only
//! Failures of a WebAssembly extension and their failure labels.

use fps_flow::{FnError, FnErrorKind};
use fps_stats::FailureReason;

/// Why a hook call, or the load of an extension, failed.
///
/// A failure ends the session that made the call and no other session,
/// because every session owns its own instance. Every case maps to the
/// catalog failure reason `extension_error`, see [`WasmFailure::reason`].
#[derive(Debug, thiserror::Error)]
pub enum WasmFailure {
    /// The extension returned a hook error.
    #[error("{0}")]
    Hook(FnError),
    /// The call used up its fuel limit, for example in an endless loop.
    #[error("extension ran out of fuel")]
    OutOfFuel,
    /// The instance tried to grow beyond its memory limit.
    #[error("extension exceeded its memory limit")]
    OutOfMemory,
    /// The extension trapped for another reason, such as an unreachable
    /// instruction, an out of bounds access, or a stack overflow.
    #[error("extension trapped: {0}")]
    Trap(String),
    /// An earlier call of the session failed, so the instance is unusable.
    #[error("extension session already failed")]
    SessionFailed,
    /// The component does not follow the extension world, or it breaks a rule
    /// of the host, such as a duplicate function name.
    #[error("invalid extension: {0}")]
    Invalid(String),
}

impl WasmFailure {
    /// The `reason` label of `sessions.failed`, which is `extension_error`
    /// for every extension failure.
    pub fn reason(&self) -> FailureReason {
        FailureReason::ExtensionError
    }

    /// The failure label `extension_<kind>` of `contracts/extension/README.md`.
    ///
    /// A hook error uses its own kind. Traps, limits, and invalid components
    /// count as `internal`.
    pub fn label(&self) -> String {
        match self {
            WasmFailure::Hook(e) => format!("extension_{}", e.kind),
            _ => "extension_internal".to_string(),
        }
    }

    /// Converts the failure for a caller of [`fps_flow::TemplateFn`].
    ///
    /// Everything except a hook error becomes [`FnErrorKind::Internal`].
    pub fn into_fn_error(self) -> FnError {
        match self {
            WasmFailure::Hook(e) => e,
            other => FnError::new(FnErrorKind::Internal, other.to_string()),
        }
    }
}
