// SPDX-License-Identifier: GPL-3.0-only
//! The WebAssembly engine, the loaded component, and its sessions.

use std::sync::Arc;

use fps_flow::{ArgValue, CallArgs, FnError, FnErrorKind, ValueType};
use wasmtime::component::{Component, HasSelf, Linker, ResourceAny};
use wasmtime::{Config, Engine, ResourceLimiter, Store, StoreLimits, StoreLimitsBuilder, Trap};

use crate::failure::WasmFailure;
use crate::host::{EngineError, Host, LogLevel, NoHost, Phase, TimerId};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../../contracts/extension",
        world: "extension",
        imports: { default: trappable },
    });
}

use bindings::exports::fps::extension::template;
use bindings::fps::extension::{engine as wit_engine, types as wit};

/// The limits of one extension. A session gets its own copy of the memory
/// limits, and every hook call gets a fresh fuel budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Fuel for one hook call, including `on-open`. A fuel unit is about one
    /// WebAssembly instruction. A call that uses it up traps.
    pub fuel_per_call: u64,
    /// Bytes of linear memory that one instance may hold.
    pub max_memory_bytes: usize,
    /// Table elements that one instance may hold.
    pub max_table_elements: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            fuel_per_call: 20_000_000,
            max_memory_bytes: 16 << 20,
            max_table_elements: 10_000,
        }
    }
}

/// Raised inside the guest when it exceeds its memory limit.
#[derive(Debug, thiserror::Error)]
#[error("memory limit exceeded")]
struct MemoryLimitExceeded;

/// `StoreLimits` that turns a refused memory growth into a trap. Plain
/// `StoreLimits` returns -1 from `memory.grow`, which lets a guest carry on
/// after it ran out of memory.
struct Limiter {
    inner: StoreLimits,
    denied: bool,
}

impl ResourceLimiter for Limiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let ok = self.inner.memory_growing(current, desired, maximum)?;
        if !ok {
            self.denied = true;
            return Err(wasmtime::Error::new(MemoryLimitExceeded));
        }
        Ok(true)
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let ok = self.inner.table_growing(current, desired, maximum)?;
        if !ok {
            self.denied = true;
            return Err(wasmtime::Error::new(MemoryLimitExceeded));
        }
        Ok(true)
    }

    fn instances(&self) -> usize {
        self.inner.instances()
    }
    fn tables(&self) -> usize {
        self.inner.tables()
    }
    fn memories(&self) -> usize {
        self.inner.memories()
    }
}

/// Store data of one instance.
struct State {
    host: Box<dyn Host>,
    phase: Phase,
    limiter: Limiter,
    max_random_bytes: usize,
}

impl wit::Host for State {}

impl wit_engine::Host for State {
    fn send(&mut self, data: Vec<u8>) -> wasmtime::Result<Result<(), wit::EngineError>> {
        if !self.phase.may_write() {
            return Ok(Err(wit::EngineError::NotAllowed));
        }
        Ok(self.host.send(&data).map_err(engine_error_to_wit))
    }

    fn set_timer(&mut self, delay_ms: u32) -> wasmtime::Result<Result<u32, wit::EngineError>> {
        if !self.phase.may_write() {
            return Ok(Err(wit::EngineError::NotAllowed));
        }
        Ok(self
            .host
            .set_timer(delay_ms)
            .map(|t| t.0)
            .map_err(engine_error_to_wit))
    }

    fn cancel_timer(&mut self, timer: u32) -> wasmtime::Result<()> {
        if self.phase.may_cancel() {
            self.host.cancel_timer(TimerId(timer));
        }
        Ok(())
    }

    fn log(
        &mut self,
        level: wit::LogLevel,
        message: String,
        fields: Vec<(String, String)>,
    ) -> wasmtime::Result<()> {
        let fields: Vec<(&str, &str)> = fields
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let level = match level {
            wit::LogLevel::Trace => LogLevel::Trace,
            wit::LogLevel::Debug => LogLevel::Debug,
            wit::LogLevel::Info => LogLevel::Info,
            wit::LogLevel::Warn => LogLevel::Warn,
            wit::LogLevel::Error => LogLevel::Error,
        };
        self.host.log(level, &message, &fields);
        Ok(())
    }

    fn random_u64(&mut self) -> wasmtime::Result<u64> {
        self.check_draw()?;
        Ok(self.host.random_u64())
    }

    fn random_bytes(&mut self, len: u32) -> wasmtime::Result<Vec<u8>> {
        self.check_draw()?;
        let len = len as usize;
        if len > self.max_random_bytes {
            return Err(wasmtime::format_err!(
                "random-bytes asked for {len} bytes, which exceeds the memory limit"
            ));
        }
        let mut out = vec![0u8; len];
        self.host.random_bytes(&mut out);
        Ok(out)
    }
}

impl State {
    /// The random calls have no error case, so a call in a hook that forbids
    /// them traps the guest.
    fn check_draw(&self) -> wasmtime::Result<()> {
        if self.phase.may_draw() {
            Ok(())
        } else {
            Err(wasmtime::format_err!(
                "random numbers are not allowed during {}",
                self.phase
            ))
        }
    }
}

fn engine_error_to_wit(e: EngineError) -> wit::EngineError {
    match e {
        EngineError::Closed => wit::EngineError::Closed,
        EngineError::Limit => wit::EngineError::Limit,
        EngineError::NotAllowed => wit::EngineError::NotAllowed,
    }
}

/// The wasmtime engine that compiles and runs extensions.
///
/// One value serves every extension of a process. We switch fuel metering on
/// and every proposal that the extension world does not need off. The build
/// has no compilation cache and no parallel compilation.
#[derive(Clone)]
pub struct WasmEngine {
    engine: Engine,
}

impl WasmEngine {
    /// Creates the engine.
    pub fn new() -> Result<Self, WasmFailure> {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.wasm_component_model(true);
        config.wasm_simd(false);
        config.wasm_relaxed_simd(false);
        config.wasm_multi_memory(false);
        config.wasm_memory64(false);
        // Sessions are many and short, so a small reservation per memory keeps
        // the virtual address space of thousands of instances in check.
        config.memory_reservation(16 << 20);
        config.memory_guard_size(0);
        let engine = Engine::new(&config).map_err(|e| WasmFailure::Invalid(e.to_string()))?;
        Ok(WasmEngine { engine })
    }
}

/// One entry of the template function catalog that an extension exports.
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionDesc {
    /// Function name without the module prefix.
    pub name: String,
    /// Parameters in declaration order.
    pub params: Vec<ParamDesc>,
    /// One line description.
    pub summary: String,
}

/// One parameter of a template function.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamDesc {
    /// Parameter name.
    pub name: String,
    /// Declared type.
    pub ty: ValueType,
    /// Value used when the call omits the argument. `None` makes it required.
    pub default: Option<OwnedValue>,
}

/// An owned template value, for catalog defaults.
#[derive(Clone, Debug, PartialEq)]
pub enum OwnedValue {
    /// An integer.
    Int(i64),
    /// A floating point number.
    Float(f64),
    /// A boolean.
    Boolean(bool),
    /// Text.
    Text(String),
    /// Bytes.
    Bytes(Vec<u8>),
}

/// Side of the flow program that runs a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// The client side.
    Client,
    /// The server side.
    Server,
}

/// Transport of a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// TCP.
    Tcp,
    /// UDP.
    Udp,
}

/// Identity and configuration of one session, the argument of `on-open`.
#[derive(Clone, Copy, Debug)]
pub struct SessionInfo<'a> {
    /// Session sequence number within the run.
    pub id: u64,
    /// Flow id.
    pub flow: &'a str,
    /// Side of the flow program.
    pub side: Side,
    /// Transport of the session.
    pub transport: Transport,
    /// Scenario variables after test parameter overrides.
    pub vars: &'a [(&'a str, ArgValue<'a>)],
}

/// Why a session ends. The cases match `close-reason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CloseReason {
    /// The flow program finished.
    Finished,
    /// The flow program or the extension aborted with a reset.
    Reset,
    /// The peer closed or reset the session first.
    PeerClosed,
    /// An `expect` or the session deadline timed out.
    Timeout,
    /// A hook or an action failed.
    Failed,
}

/// What the engine does with bytes that `on_data` saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataOutcome {
    /// Deliver the received bytes unchanged.
    Pass,
    /// The extension consumed the bytes.
    Consume,
    /// Deliver these bytes instead of the received ones.
    Replace(Vec<u8>),
}

fn value_to_wit(v: &ArgValue<'_>) -> wit::Value {
    match v {
        ArgValue::Int(i) => wit::Value::Int(*i),
        ArgValue::Float(f) => wit::Value::Float(*f),
        ArgValue::Boolean(b) => wit::Value::Boolean(*b),
        ArgValue::Text(t) => wit::Value::Text((*t).to_string()),
        ArgValue::Bytes(b) => wit::Value::Bytes(b.to_vec()),
        // `ArgValue` is not marked non_exhaustive, so this arm cannot exist,
        // and the compiler flags a new case here.
    }
}

fn value_from_wit(v: wit::Value) -> OwnedValue {
    match v {
        wit::Value::Int(i) => OwnedValue::Int(i),
        wit::Value::Float(f) => OwnedValue::Float(f),
        wit::Value::Boolean(b) => OwnedValue::Boolean(b),
        wit::Value::Text(t) => OwnedValue::Text(t),
        wit::Value::Bytes(b) => OwnedValue::Bytes(b),
    }
}

fn type_from_wit(t: wit::ValueType) -> ValueType {
    match t {
        wit::ValueType::Int => ValueType::Int,
        wit::ValueType::Float => ValueType::Float,
        wit::ValueType::Boolean => ValueType::Boolean,
        wit::ValueType::Text => ValueType::Text,
        wit::ValueType::Bytes => ValueType::Bytes,
        wit::ValueType::Any => ValueType::Any,
    }
}

fn hook_error(e: wit::HookError) -> WasmFailure {
    let kind = match e.kind {
        wit::HookErrorKind::InvalidArgument => FnErrorKind::InvalidArgument,
        wit::HookErrorKind::Protocol => FnErrorKind::Protocol,
        wit::HookErrorKind::Unsupported => FnErrorKind::Unsupported,
        wit::HookErrorKind::Internal => FnErrorKind::Internal,
    };
    WasmFailure::Hook(FnError::new(kind, e.message))
}

/// A loaded extension component. It is immutable and shared by every session.
pub struct WasmExtension {
    engine: WasmEngine,
    pre: bindings::ExtensionPre<State>,
    limits: Limits,
    catalog: Arc<Vec<FunctionDesc>>,
}

impl WasmExtension {
    /// Compiles `bytes`, a component binary or WAT text when the build allows
    /// it, links the engine calls, and reads the function catalog.
    ///
    /// The component may import only the `engine` interface of the extension
    /// world and must export `template` and `hooks`.
    pub fn load(engine: &WasmEngine, bytes: &[u8], limits: Limits) -> Result<Self, WasmFailure> {
        let invalid = |e: wasmtime::Error| WasmFailure::Invalid(format!("{e:#}"));
        let component = Component::new(&engine.engine, bytes).map_err(invalid)?;
        let mut linker: Linker<State> = Linker::new(&engine.engine);
        bindings::Extension::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)
            .map_err(invalid)?;
        let pre = bindings::ExtensionPre::new(linker.instantiate_pre(&component).map_err(invalid)?)
            .map_err(invalid)?;
        let mut ext = WasmExtension {
            engine: engine.clone(),
            pre,
            limits,
            catalog: Arc::new(Vec::new()),
        };
        let catalog = ext.read_catalog()?;
        ext.catalog = Arc::new(catalog);
        Ok(ext)
    }

    /// The template function catalog, read once at load.
    pub fn functions(&self) -> &[FunctionDesc] {
        &self.catalog
    }

    /// The limits that every session of this extension gets.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    fn new_store(&self, host: Box<dyn Host>, phase: Phase) -> Store<State> {
        let l = &self.limits;
        let inner = StoreLimitsBuilder::new()
            .memory_size(l.max_memory_bytes)
            .table_elements(l.max_table_elements)
            .instances(32)
            .tables(8)
            .memories(1)
            .build();
        let mut store = Store::new(
            &self.engine.engine,
            State {
                host,
                phase,
                limiter: Limiter {
                    inner,
                    denied: false,
                },
                max_random_bytes: l.max_memory_bytes,
            },
        );
        store.limiter(|s| &mut s.limiter);
        store
    }

    fn read_catalog(&self) -> Result<Vec<FunctionDesc>, WasmFailure> {
        let mut store = self.new_store(Box::new(NoHost), Phase::Catalog);
        let call = |store: &mut Store<State>| -> wasmtime::Result<Vec<template::FunctionDesc>> {
            store.set_fuel(self.limits.fuel_per_call)?;
            let instance = self.pre.instantiate(&mut *store)?;
            instance.fps_extension_template().call_functions(store)
        };
        let raw = call(&mut store).map_err(|e| classify(&store, e))?;
        let mut catalog: Vec<FunctionDesc> = Vec::with_capacity(raw.len());
        for f in raw {
            if catalog.iter().any(|c| c.name == f.name) {
                return Err(WasmFailure::Invalid(format!(
                    "the catalog lists function '{}' twice",
                    f.name
                )));
            }
            catalog.push(FunctionDesc {
                name: f.name,
                summary: f.summary,
                params: f
                    .params
                    .into_iter()
                    .map(|p| ParamDesc {
                        name: p.name,
                        ty: type_from_wit(p.type_),
                        default: p.default.map(value_from_wit),
                    })
                    .collect(),
            });
        }
        Ok(catalog)
    }

    /// Instantiates the component for one session and runs `on-open`.
    ///
    /// Every session owns an instance, so a trap or a limit that ends one
    /// session cannot touch another. `host` serves the engine calls of this
    /// session until it ends.
    pub fn open(
        &self,
        info: &SessionInfo<'_>,
        host: Box<dyn Host>,
    ) -> Result<WasmSession, WasmFailure> {
        let mut store = self.new_store(host, Phase::Open);
        let info = wit::SessionInfo {
            id: info.id,
            flow: info.flow.to_string(),
            side: match info.side {
                Side::Client => wit::Side::Client,
                Side::Server => wit::Side::Server,
            },
            transport: match info.transport {
                Transport::Tcp => wit::Transport::Tcp,
                Transport::Udp => wit::Transport::Udp,
            },
            vars: info
                .vars
                .iter()
                .map(|(name, v)| wit::NamedValue {
                    name: (*name).to_string(),
                    value: value_to_wit(v),
                })
                .collect(),
        };
        let opened =
            (|| -> wasmtime::Result<Result<(bindings::Extension, ResourceAny), wit::HookError>> {
                store.set_fuel(self.limits.fuel_per_call)?;
                let instance = self.pre.instantiate(&mut store)?;
                let res = instance
                    .fps_extension_hooks()
                    .call_on_open(&mut store, &info)?;
                Ok(res.map(|r| (instance, r)))
            })();
        let (instance, resource) = match opened {
            Ok(Ok(pair)) => pair,
            Ok(Err(e)) => return Err(hook_error(e)),
            Err(e) => return Err(classify(&store, e)),
        };
        let used = self
            .limits
            .fuel_per_call
            .saturating_sub(store.get_fuel().unwrap_or(0));
        Ok(WasmSession {
            store,
            instance,
            resource: Some(resource),
            limits: self.limits,
            catalog: Arc::clone(&self.catalog),
            failed: false,
            fuel_last_call: used,
        })
    }
}

/// Maps a wasmtime error to a failure, with the state of the limiter and the
/// trap code as evidence.
fn classify(store: &Store<State>, err: wasmtime::Error) -> WasmFailure {
    if store.data().limiter.denied || err.downcast_ref::<MemoryLimitExceeded>().is_some() {
        return WasmFailure::OutOfMemory;
    }
    if matches!(err.downcast_ref::<Trap>(), Some(Trap::OutOfFuel)) {
        return WasmFailure::OutOfFuel;
    }
    WasmFailure::Trap(format!("{err:#}"))
}

/// One session of a WebAssembly extension, with its own instance.
pub struct WasmSession {
    store: Store<State>,
    instance: bindings::Extension,
    resource: Option<ResourceAny>,
    limits: Limits,
    catalog: Arc<Vec<FunctionDesc>>,
    failed: bool,
    fuel_last_call: u64,
}

impl WasmSession {
    /// Returns the function catalog of the extension.
    pub fn functions(&self) -> &[FunctionDesc] {
        &self.catalog
    }

    /// Returns true after a call failed. A failed session accepts no more
    /// hook calls.
    pub fn is_failed(&self) -> bool {
        self.failed
    }

    /// Fuel that the last call used.
    pub fn fuel_used_by_last_call(&self) -> u64 {
        self.fuel_last_call
    }

    /// Runs one hook with a fresh fuel budget and the rules of its phase.
    fn call<T>(
        &mut self,
        phase: Phase,
        f: impl FnOnce(
            &bindings::Extension,
            &mut Store<State>,
            ResourceAny,
        ) -> wasmtime::Result<Result<T, wit::HookError>>,
    ) -> Result<T, WasmFailure> {
        if self.failed {
            return Err(WasmFailure::SessionFailed);
        }
        let Some(resource) = self.resource else {
            return Err(WasmFailure::SessionFailed);
        };
        self.store.data_mut().phase = phase;
        let budget = self.limits.fuel_per_call;
        let outcome = self
            .store
            .set_fuel(budget)
            .and_then(|()| f(&self.instance, &mut self.store, resource));
        self.fuel_last_call = budget.saturating_sub(self.store.get_fuel().unwrap_or(0));
        match outcome {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => {
                self.failed = true;
                Err(hook_error(e))
            }
            Err(e) => {
                self.failed = true;
                Err(classify(&self.store, e))
            }
        }
    }

    /// The engine received bytes, before `expect` sees them.
    pub fn on_data(&mut self, data: &[u8]) -> Result<DataOutcome, WasmFailure> {
        self.call(Phase::Data, |ext, store, res| {
            let r = ext
                .fps_extension_hooks()
                .session()
                .call_on_data(store, res, data)?;
            Ok(r.map(|o| match o {
                wit::DataOutcome::Pass => DataOutcome::Pass,
                wit::DataOutcome::Consume => DataOutcome::Consume,
                wit::DataOutcome::Replace(b) => DataOutcome::Replace(b),
            }))
        })
    }

    /// A timer of this session fired.
    pub fn on_timer(&mut self, timer: TimerId) -> Result<(), WasmFailure> {
        self.call(Phase::Timer, |ext, store, res| {
            ext.fps_extension_hooks()
                .session()
                .call_on_timer(store, res, timer.0)
        })
    }

    /// Renders template function `function` with `args` and appends the bytes
    /// to `out`. The caller has checked the arguments against the catalog.
    pub fn render(
        &mut self,
        function: &str,
        args: &CallArgs<'_>,
        out: &mut Vec<u8>,
    ) -> Result<(), WasmFailure> {
        let args = wit::CallArgs {
            positional: args.positional.iter().map(value_to_wit).collect(),
            named: args
                .named
                .iter()
                .map(|n| wit::NamedValue {
                    name: n.name.to_string(),
                    value: value_to_wit(&n.value),
                })
                .collect(),
        };
        let bytes = self.call(Phase::Render, |ext, store, res| {
            ext.fps_extension_hooks()
                .session()
                .call_render(store, res, function, &args)
        })?;
        out.extend_from_slice(&bytes);
        Ok(())
    }

    /// The session ends. The call runs once, drops the resource, and makes
    /// the session unusable. A failed session skips the call, because its
    /// instance cannot run any more.
    pub fn on_close(&mut self, reason: CloseReason) -> Result<(), WasmFailure> {
        if self.failed {
            self.resource = None;
            return Ok(());
        }
        let reason = match reason {
            CloseReason::Finished => wit::CloseReason::Finished,
            CloseReason::Reset => wit::CloseReason::Reset,
            CloseReason::PeerClosed => wit::CloseReason::PeerClosed,
            CloseReason::Timeout => wit::CloseReason::Timeout,
            CloseReason::Failed => wit::CloseReason::Failed,
        };
        let result = self.call(Phase::Close, |ext, store, res| {
            ext.fps_extension_hooks()
                .session()
                .call_on_close(store, res, reason)?;
            Ok(Ok(()))
        });
        if let Some(res) = self.resource.take() {
            let _ = res.resource_drop(&mut self.store);
        }
        self.failed = true;
        result
    }
}
