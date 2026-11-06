// SPDX-License-Identifier: GPL-3.0-only
//! The [`TemplateFn`] adapter over WebAssembly sessions.

use std::collections::HashMap;

use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use rand::RngCore;

use crate::extension::WasmSession;
use crate::failure::WasmFailure;

/// The module table of one session, with a [`WasmSession`] for each module
/// that the flows of the session name.
///
/// A native extension plugs into the same [`TemplateFn`] seam, so the
/// renderer calls one code path for both kinds. The table checks the call
/// against the catalog of the module first, which saves a boundary crossing
/// for a call that the extension would reject, and then forwards it to the
/// `render` hook.
///
/// Randomness reaches an extension only through the `random-*` engine calls,
/// which the session [`Host`](crate::Host) serves. The `rng` argument of
/// [`TemplateFn::render`] is therefore not used here, and the host of each
/// session derives its generator from the run seed and the session id.
#[derive(Default)]
pub struct WasmFunctions {
    sessions: HashMap<String, WasmSession>,
    /// The failure that ended the last call, for the `extension_error` count.
    last_failure: Option<WasmFailure>,
}

impl WasmFunctions {
    /// Creates an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers the session of module `module`, replacing an earlier one.
    pub fn insert(&mut self, module: impl Into<String>, session: WasmSession) {
        self.sessions.insert(module.into(), session);
    }

    /// Returns the session of `module`, for the other hooks.
    pub fn session_mut(&mut self, module: &str) -> Option<&mut WasmSession> {
        self.sessions.get_mut(module)
    }

    /// Takes the failure of the last failed call. A failure counts as the
    /// reason `extension_error`, see [`WasmFailure::reason`].
    pub fn take_failure(&mut self) -> Option<WasmFailure> {
        self.last_failure.take()
    }
}

impl TemplateFn for WasmFunctions {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        _rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        let Some(session) = self.sessions.get_mut(call.module) else {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("no module provides '{}.{}'", call.module, call.function),
            ));
        };
        if !session.functions().iter().any(|f| f.name == call.function) {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!(
                    "module '{}' has no function '{}'",
                    call.module, call.function
                ),
            ));
        }
        session
            .render(call.function, &call.args, out)
            .map_err(|failure| {
                let err = match &failure {
                    WasmFailure::Hook(e) => e.clone(),
                    other => FnError::new(FnErrorKind::Internal, other.to_string()),
                };
                self.last_failure = Some(failure);
                err
            })
    }
}
