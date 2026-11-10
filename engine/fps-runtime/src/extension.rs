// SPDX-License-Identifier: GPL-3.0-only
//! WebAssembly module extensions in the worker runtime.
//!
//! A module that ships `extension.wasm` is compiled once, when the owner of
//! the runtime builds an [`ExtensionSet`]. The set is immutable and sits behind
//! an [`Arc`] in [`crate::FlowRef::extensions`], so every worker thread shares
//! the compiled component. Each session that calls a function of such a module
//! gets its own instance through `fps-wasm`, so a trap, a fuel limit, or a
//! memory limit ends that session and no other.
//!
//! The worker wraps the native module functions of a session in a
//! [`SessionExtensions`]. A call to a module of the set goes to the extension,
//! and every other call goes to the native functions unchanged. A flow without
//! an extension set takes no wrapper, so the path costs one `Option` check per
//! session.
//!
//! The instance of a module opens on the first call of the session that names
//! the module, so a session that never calls the module pays nothing for it.
//! The interpreter has no seam yet for `send`, timers, or `on-data`, so the
//! engine calls of an extension session refuse `send` and `set-timer` with
//! `not-allowed`, and the runtime calls the `render` and `on-close` hooks only.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fps_flow::manifest::{Catalog, Manifest};
use fps_flow::template::{Value, Vars};
use fps_flow::{ArgValue, FnCall, FnError, FnErrorKind, Scenario, TemplateFn};
use fps_interp::{session_rng, Reason, Side};
use fps_wasm::{
    CloseReason, EngineError, FunctionDesc, Host, Limits as WasmLimits, LogLevel, SessionInfo,
    TimerId, Transport, WasmEngine, WasmExtension, WasmFailure, WasmSession,
};
use rand::RngCore;

/// The file of a module directory that holds the extension component.
pub const EXTENSION_FILE: &str = "extension.wasm";

/// Why an extension could not be loaded into an [`ExtensionSet`].
#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    /// The WebAssembly engine did not start, or the component is invalid.
    #[error("module {module}: load extension")]
    Load {
        /// The module name.
        module: String,
        /// The host failure.
        #[source]
        source: WasmFailure,
    },
    /// The extension file exists but cannot be read.
    #[error("module {module}: read {}", path.display())]
    Read {
        /// The module name.
        module: String,
        /// The file.
        path: PathBuf,
        /// The operating system error.
        #[source]
        source: io::Error,
    },
    /// The extension catalog differs from the catalog of the manifest.
    #[error("module {module}: extension catalog differs from the manifest: {detail}")]
    Catalog {
        /// The module name.
        module: String,
        /// The first difference.
        detail: String,
    },
    /// Two extensions claim one module name.
    #[error("module {module}: extension loaded twice")]
    Duplicate {
        /// The module name.
        module: String,
    },
}

/// The compiled extensions of a run, by module name.
///
/// Build it before the runtime starts, with [`ExtensionSet::load`] or
/// [`ExtensionSet::load_module_dir`], wrap it in an [`Arc`], and hand it to
/// every [`crate::FlowRef`] whose templates call these modules. Compilation
/// happens here, once per module, and never on a worker thread.
pub struct ExtensionSet {
    engine: WasmEngine,
    modules: HashMap<String, Arc<WasmExtension>>,
}

impl fmt::Debug for ExtensionSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut names: Vec<&String> = self.modules.keys().collect();
        names.sort();
        f.debug_struct("ExtensionSet")
            .field("modules", &names)
            .finish_non_exhaustive()
    }
}

impl ExtensionSet {
    /// Creates an empty set with its own WebAssembly engine.
    pub fn new() -> Result<Self, ExtensionError> {
        let engine = WasmEngine::new().map_err(|source| ExtensionError::Load {
            module: String::new(),
            source,
        })?;
        Ok(Self {
            engine,
            modules: HashMap::new(),
        })
    }

    /// Compiles `wasm`, a component of the `extension` world, as the extension
    /// of `module`.
    ///
    /// With `declared` set, the catalog that the extension returns must equal
    /// the catalog of the manifest in every function name, parameter name,
    /// parameter order, type, and required flag, as `contracts/README.md`
    /// demands. The manifest keeps no default values, so those are not
    /// compared.
    pub fn load(
        &mut self,
        module: &str,
        wasm: &[u8],
        limits: WasmLimits,
        declared: Option<&Catalog>,
    ) -> Result<(), ExtensionError> {
        if self.modules.contains_key(module) {
            return Err(ExtensionError::Duplicate {
                module: module.to_owned(),
            });
        }
        let ext = WasmExtension::load(&self.engine, wasm, limits).map_err(|source| {
            ExtensionError::Load {
                module: module.to_owned(),
                source,
            }
        })?;
        if let Some(declared) = declared {
            if let Some(detail) = catalog_difference(declared, ext.functions()) {
                return Err(ExtensionError::Catalog {
                    module: module.to_owned(),
                    detail,
                });
            }
        }
        self.modules.insert(module.to_owned(), Arc::new(ext));
        Ok(())
    }

    /// Loads `extension.wasm` from the directory of the module that `manifest`
    /// describes, and checks the catalog against the manifest.
    ///
    /// Returns `Ok(false)` when the directory has no `extension.wasm`, which is
    /// the case of a module without an extension. The file is read here, after
    /// any digest the caller took, so a caller that digests the module tree
    /// reads the file itself and calls [`ExtensionSet::load_module_bytes`].
    pub fn load_module_dir(
        &mut self,
        manifest: &Manifest,
        dir: &Path,
        limits: WasmLimits,
    ) -> Result<bool, ExtensionError> {
        let path = dir.join(EXTENSION_FILE);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(source) => {
                return Err(ExtensionError::Read {
                    module: manifest.name.clone(),
                    path,
                    source,
                })
            }
        };
        self.load_module_bytes(manifest, &bytes, limits)?;
        Ok(true)
    }

    /// Compiles `bytes` as the extension of the module that `manifest`
    /// describes, and checks the catalog against the manifest.
    ///
    /// Use it when the caller has read `extension.wasm` itself, for example to
    /// digest the module tree: the component that runs is then the very bytes
    /// that were digested, and a writer cannot swap the file in between, which
    /// [`ExtensionSet::load_module_dir`] cannot promise because it reads the
    /// file again.
    pub fn load_module_bytes(
        &mut self,
        manifest: &Manifest,
        bytes: &[u8],
        limits: WasmLimits,
    ) -> Result<(), ExtensionError> {
        self.load(&manifest.name, bytes, limits, Some(&manifest.functions))
    }

    /// Returns true when the set holds an extension for `module`.
    pub fn contains(&self, module: &str) -> bool {
        self.modules.contains_key(module)
    }

    /// Returns true when the set holds no extension.
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    /// Returns the module functions of one session, with `fallback` serving
    /// every module that the set does not hold.
    pub fn session_functions(
        self: &Arc<Self>,
        key: SessionKey,
        fallback: Box<dyn TemplateFn>,
    ) -> SessionExtensions {
        SessionExtensions {
            set: Arc::clone(self),
            key,
            sessions: HashMap::new(),
            failure: None,
            fallback,
        }
    }
}

/// Describes the first difference between the manifest catalog and the
/// catalog of the extension.
fn catalog_difference(declared: &Catalog, actual: &[FunctionDesc]) -> Option<String> {
    for f in actual {
        let Some(sig) = declared.get(&f.name) else {
            return Some(format!("the extension has function '{}'", f.name));
        };
        if sig.params.len() != f.params.len() {
            return Some(format!(
                "function '{}' has {} parameters in the manifest and {} in the extension",
                f.name,
                sig.params.len(),
                f.params.len()
            ));
        }
        for (i, (m, e)) in sig.params.iter().zip(&f.params).enumerate() {
            if m.name != e.name {
                return Some(format!(
                    "function '{}' parameter {i} is '{}' in the manifest and '{}' in the extension",
                    f.name, m.name, e.name
                ));
            }
            if m.ty != e.ty {
                return Some(format!(
                    "function '{}' parameter '{}' is {} in the manifest and {} in the extension",
                    f.name, m.name, m.ty, e.ty
                ));
            }
            if m.required != e.default.is_none() {
                return Some(format!(
                    "function '{}' parameter '{}' differs in its default",
                    f.name, m.name
                ));
            }
        }
    }
    let mut missing: Vec<&String> = declared
        .keys()
        .filter(|k| !actual.iter().any(|f| &f.name == *k))
        .collect();
    missing.sort();
    missing
        .first()
        .map(|k| format!("the manifest has function '{k}'"))
}

/// What identifies one session to its extensions.
#[derive(Clone, Debug)]
pub struct SessionKey {
    /// The run seed.
    pub seed: u64,
    /// The run wide session id, see [`crate::global_session_id`].
    pub session_id: u64,
    /// The side of the flow that the session runs.
    pub side: Side,
    /// The compiled scenario, whose variables the extension sees at open.
    pub scenario: Arc<Scenario>,
    /// The index of the session's flow in `scenario.flows`.
    pub flow: usize,
    /// The transport of the session, which the extension sees at open.
    pub transport: Transport,
}

/// The module functions of one session: extension modules through
/// `fps-wasm`, every other module through the native functions.
pub struct SessionExtensions {
    set: Arc<ExtensionSet>,
    key: SessionKey,
    sessions: HashMap<String, WasmSession>,
    failure: Option<WasmFailure>,
    fallback: Box<dyn TemplateFn>,
}

impl SessionExtensions {
    /// Takes the failure of the last failed extension call, which the
    /// `extension_error` count of the session comes from.
    pub fn take_failure(&mut self) -> Option<WasmFailure> {
        self.failure.take()
    }

    /// Ends the extension sessions of this session with the `on-close` hook
    /// and returns the first failure of a hook, or of an earlier call.
    ///
    /// The worker has recorded the outcome of the session by then, so a
    /// failure here does not change the failure counts.
    pub fn finish(mut self, reason: CloseReason) -> Option<WasmFailure> {
        let mut failure = self.failure.take();
        let mut names: Vec<String> = self.sessions.keys().cloned().collect();
        names.sort();
        for name in names {
            if let Some(mut session) = self.sessions.remove(&name) {
                if let Err(e) = session.on_close(reason) {
                    failure.get_or_insert(e);
                }
            }
        }
        failure
    }

    fn open(&self, module: &str, ext: &WasmExtension) -> Result<WasmSession, WasmFailure> {
        let flow = self.key.scenario.flows.get(self.key.flow);
        let vars = sorted_vars(&self.key.scenario.vars);
        let info = SessionInfo {
            id: self.key.session_id,
            flow: flow.map_or("", |f| f.id.as_str()),
            side: match self.key.side {
                Side::Client => fps_wasm::Side::Client,
                Side::Server => fps_wasm::Side::Server,
            },
            transport: self.key.transport,
            vars: &vars,
        };
        let host = SessionHost {
            module: module.to_owned(),
            session: self.key.session_id,
            rng: Box::new(extension_rng(
                self.key.seed,
                module,
                self.key.session_id,
                self.key.side,
            )),
        };
        ext.open(&info, Box::new(host))
    }
}

/// The variables of a scenario in name order, so that an extension sees the
/// same list in every run.
fn sorted_vars(vars: &Vars) -> Vec<(&str, ArgValue<'_>)> {
    let mut out: Vec<(&str, ArgValue<'_>)> = vars
        .iter()
        .map(|(k, v)| {
            let v = match v {
                Value::Int(i) => ArgValue::Int(*i),
                Value::Float(f) => ArgValue::Float(*f),
                Value::Bool(b) => ArgValue::Boolean(*b),
                Value::Str(s) => ArgValue::Text(s.as_str()),
            };
            (k.as_str(), v)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(b.0));
    out
}

impl TemplateFn for SessionExtensions {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        let set = Arc::clone(&self.set);
        let Some(ext) = set.modules.get(call.module) else {
            return self.fallback.render(call, rng, out);
        };
        if !ext.functions().iter().any(|f| f.name == call.function) {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!(
                    "module '{}' has no function '{}'",
                    call.module, call.function
                ),
            ));
        }
        if !self.sessions.contains_key(call.module) {
            match self.open(call.module, ext) {
                Ok(s) => {
                    self.sessions.insert(call.module.to_owned(), s);
                }
                Err(failure) => return Err(self.record(failure)),
            }
        }
        let Some(session) = self.sessions.get_mut(call.module) else {
            return Err(FnError::new(
                FnErrorKind::Internal,
                "invariant: the extension session was just opened",
            ));
        };
        match session.render(call.function, &call.args, out) {
            Ok(()) => Ok(()),
            Err(failure) => Err(self.record(failure)),
        }
    }
}

impl SessionExtensions {
    /// Keeps `failure` for [`SessionExtensions::take_failure`] and returns the
    /// error that the renderer sees. The renderer wraps it in a function error
    /// of the template, which [`fps_interp::FailureKind::reason`] reports as
    /// [`Reason::ExtensionError`], whatever its kind.
    fn record(&mut self, failure: WasmFailure) -> FnError {
        debug_assert_eq!(
            Reason::ExtensionError.as_str(),
            failure.reason().as_str(),
            "every extension failure has the reason extension_error"
        );
        let err = match &failure {
            WasmFailure::Hook(e) => e.clone(),
            other => FnError::new(FnErrorKind::Internal, other.to_string()),
        };
        self.failure = Some(failure);
        err
    }
}

/// Maps the outcome of a session to the close reason of its extensions.
///
/// Every runner that calls [`SessionExtensions::finish`] uses this mapping, so
/// that the extension sees one close reason for one outcome on any transport.
pub fn close_reason(outcome: &Result<(), fps_interp::Failure>) -> CloseReason {
    close_reason_of(outcome.as_ref().err().map(|f| f.reason))
}

/// Maps the failure reason of a session, or `None` for a session that
/// finished, to the close reason of its extensions. A scenario instance keeps
/// only the reason of each side, see [`fps_interp::SideRecord`].
pub(crate) fn close_reason_of(failed: Option<Reason>) -> CloseReason {
    match failed {
        None => CloseReason::Finished,
        Some(Reason::Aborted | Reason::UnexpectedReset) => CloseReason::Reset,
        Some(Reason::PeerClosed) => CloseReason::PeerClosed,
        Some(Reason::ConnectTimeout | Reason::ExpectTimeout) => CloseReason::Timeout,
        Some(_) => CloseReason::Failed,
    }
}

/// The domain constant that separates the extension generators from the
/// template generator of the same session, which [`session_rng`] derives from
/// the plain run seed.
const EXTENSION_DOMAIN: u64 = 0x6670_735f_6578_7473; // "fps_exts"

/// Returns the generator of the extension of `module` in one session.
///
/// It derives from the run seed, the module name, the session id, and the side
/// through [`session_rng`], so a replay reproduces every draw, the order in
/// which a worker runs its sessions does not change them, and neither the
/// template stream of the session nor the streams of other modules share it.
pub fn extension_rng(seed: u64, module: &str, session_id: u64, side: Side) -> impl RngCore + Send {
    session_rng(seed ^ EXTENSION_DOMAIN ^ fnv1a(module), session_id, side)
}

/// FNV-1a over the module name, which the generator mixes further.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The engine calls of one extension session.
///
/// Randomness comes from the session generator and nothing else. The
/// interpreter has no seam for transport writes or timers yet, so `send` and
/// `set_timer` refuse with [`EngineError::NotAllowed`], which a guest sees as
/// the `not-allowed` error of the contract.
struct SessionHost {
    module: String,
    session: u64,
    rng: Box<dyn RngCore + Send>,
}

impl Host for SessionHost {
    fn send(&mut self, _data: &[u8]) -> Result<(), EngineError> {
        Err(EngineError::NotAllowed)
    }

    fn set_timer(&mut self, _delay_ms: u32) -> Result<TimerId, EngineError> {
        Err(EngineError::NotAllowed)
    }

    fn cancel_timer(&mut self, _timer: TimerId) {}

    fn log(&mut self, level: LogLevel, message: &str, fields: &[(&str, &str)]) {
        let (module, session) = (self.module.as_str(), self.session);
        match level {
            LogLevel::Trace => {
                tracing::trace!(module, session, ?fields, "extension: {message}");
            }
            LogLevel::Debug => {
                tracing::debug!(module, session, ?fields, "extension: {message}");
            }
            LogLevel::Info => {
                tracing::info!(module, session, ?fields, "extension: {message}");
            }
            LogLevel::Warn => {
                tracing::warn!(module, session, ?fields, "extension: {message}");
            }
            // LogLevel is non_exhaustive, and the highest level covers the rest.
            _ => tracing::error!(module, session, ?fields, "extension: {message}"),
        }
    }

    fn random_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }

    fn random_bytes(&mut self, out: &mut [u8]) {
        self.rng.fill_bytes(out);
    }
}
