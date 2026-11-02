# Extension interface v0.1

The file `fps-extension.wit` defines the WIT package `fps:extension@0.1.0`. A module ships an extension when a protocol needs computation that flow programs cannot express, such as SSH key exchange or SMB signing. Community modules ship the extension as a WebAssembly component that targets the world `extension` (decision D3). First party modules may instead compile the same interface into the agent as a Rust trait, which decision record [0010](../../docs/decisions/0010-native-extensions.md) proposes.

## The world

The world imports one interface and exports two.

| Interface | Direction | Contents |
|---|---|---|
| `types` | shared | Values, session identity, call arguments, outcomes, and errors |
| `engine` | import | `send`, `set-timer`, `cancel-timer`, `log`, `random-u64`, and `random-bytes` |
| `template` | export | `functions`, the catalog of template functions with typed parameters |
| `hooks` | export | `on-open`, and the `session` resource with `on-data`, `on-timer`, `on-close`, and `render` |

The engine imports nothing else into the component. An extension therefore has no clock, no file system, no sockets, and no source of randomness beyond the session generator. Replays of a run reproduce every extension decision, because timers run on the injected engine clock and randomness derives from the run seed and the session id.

## Session lifecycle

The engine calls the hooks of one session in a fixed order.

1. `on-open` runs after `connect` or `accept` succeeds and before the first action of the flow program. It returns the `session` resource or an error.
2. `on-data` runs for every chunk of bytes that the engine reads for the session, before `expect` sees them. The outcome `pass` delivers the bytes unchanged, `consume` hides them from the flow program, and `replace` delivers other bytes in their place.
3. `on-timer` runs when a timer that the session set fires.
4. `render` runs when a `send` or `set` action renders a template that calls a function of the module.
5. `on-close` runs exactly once when the session ends, with the reason. The engine then drops the resource.

The manifest field `extension.hooks` lists the hooks that carry logic. The engine always calls `on-open` and calls `render` and `on-timer` only when a template or a timer needs them. It skips `on-data` and `on-close` for a module that does not list them, which saves one boundary crossing per received chunk. The world still exports every hook, and an unused hook returns at once.

A hook error or a trap fails only the calling session. The engine records the failure label `extension_<kind>`, for example `extension_protocol`, and closes the session with reason `failed`.

| Hook | `send` | `set-timer` | `cancel-timer` | `log` | `random-*` |
|---|---|---|---|---|---|
| `on-open` | yes | yes | yes | yes | yes |
| `on-data` | yes | yes | yes | yes | yes |
| `on-timer` | yes | yes | yes | yes | yes |
| `render` | `not-allowed` | `not-allowed` | yes | yes | yes |
| `on-close` | `not-allowed` | `not-allowed` | `not-allowed` | yes | `not-allowed` |

## Template functions

A template calls an extension function as `<module>.<name>(...)`, for example `ssh.kexinit()`. The engine calls `functions` once when it loads the module and checks every call in every scenario against the catalog before the run starts. The check covers the function name, the argument count, keyword names, and the declared `value-type` of each parameter. A parameter without a `default` is required. Keyword argument syntax in templates arrives with contract CON-04, and v0.1 already carries keyword arguments in `call-args.named`.

The engine passes the function name without the module prefix. The bytes that `render` returns replace the placeholder. A `set` action parses those bytes as text with the same rules as a command line override, so `42` binds an integer.

## Mapping to the Rust trait

The agent exposes the same interface to native extensions as the traits below. The WebAssembly host (ENG-11) adapts a component to these traits, so the interpreter calls one code path for both kinds of extension.

```rust
/// A loaded extension. One instance serves every session of a worker.
pub trait Extension: Send + Sync {
    /// The template function catalog. Called once per load.
    fn functions(&self) -> Vec<FunctionDesc>;
    /// Open the per session state.
    fn on_open(&self, info: &SessionInfo<'_>, host: &mut dyn Host)
        -> Result<Box<dyn ExtensionSession>, HookError>;
}

/// Per session state of an extension.
pub trait ExtensionSession: Send {
    fn on_data(&mut self, data: &[u8], host: &mut dyn Host) -> Result<DataOutcome, HookError>;
    fn on_timer(&mut self, timer: TimerId, host: &mut dyn Host) -> Result<(), HookError>;
    fn on_close(&mut self, reason: CloseReason, host: &mut dyn Host);
    /// Append the rendered bytes to `out`.
    fn render(&mut self, function: &str, args: &CallArgs<'_>, out: &mut Vec<u8>, host: &mut dyn Host)
        -> Result<(), HookError>;
}

/// Engine calls, bound to the session whose hook is running.
pub trait Host {
    fn send(&mut self, data: &[u8]) -> Result<(), EngineError>;
    fn set_timer(&mut self, delay_ms: u32) -> Result<TimerId, EngineError>;
    fn cancel_timer(&mut self, timer: TimerId);
    fn log(&mut self, level: LogLevel, message: &str, fields: &[(&str, &str)]);
    fn random_u64(&mut self) -> u64;
    fn random_bytes(&mut self, out: &mut [u8]);
}
```

The table maps every WIT item to its Rust form. Names follow the usual wit-bindgen conversion, kebab case to snake case for functions and fields and to upper camel case for types.

| WIT item | Rust form | Note |
|---|---|---|
| `package fps:extension@0.1.0` | crate `fps-extension`, version 0.1.0 | The crate re-exports the traits and types below |
| `world extension` | `Extension` plus `ExtensionSession` | A native module registers one `Extension` in the agent's module table |
| `s64`, `u64`, `u32`, `f64`, `bool` | `i64`, `u64`, `u32`, `f64`, `bool` | |
| `string` (argument) | `&str` | Borrowed, no copy on the native path |
| `string` (record field) | `String`, or `&'a str` inside `SessionInfo` and `CallArgs` | |
| `list<u8>` (argument) | `&[u8]` | Borrowed from the receive buffer |
| `list<u8>` (render result) | bytes appended to `out: &mut Vec<u8>` | The native path writes into the session send buffer and allocates nothing per action |
| `list<u8>` (`random-bytes` result) | fills `out: &mut [u8]` | The caller sizes the slice |
| `list<T>` | `Vec<T>`, or `&[T]` when borrowed | |
| `tuple<string, string>` | `(&str, &str)` | Log fields |
| `option<T>` | `Option<T>` | |
| `result<T, E>` | `Result<T, E>` | |
| `result<_, E>` | `Result<(), E>` | |
| `enum side`, `transport`, `close-reason`, `hook-error-kind`, `engine-error`, `log-level`, `value-type` | `#[non_exhaustive] enum` with the same cases in upper camel case | |
| `variant value` | `enum Value<'a> { Int(i64), Float(f64), Boolean(bool), Text(&'a str), Bytes(&'a [u8]) }` | Owned form `OwnedValue` for catalog defaults |
| `variant data-outcome` | `enum DataOutcome { Pass, Consume, Replace(Vec<u8>) }` | `Pass` avoids a copy on both paths |
| `record named-value` | `struct NamedValue<'a> { name: &'a str, value: Value<'a> }` | |
| `record session-info` | `struct SessionInfo<'a>` | `vars` borrows the scenario variables |
| `record call-args` | `struct CallArgs<'a> { positional: &'a [Value<'a>], named: &'a [NamedValue<'a>] }` | |
| `record param-desc`, `function-desc` | `struct ParamDesc`, `struct FunctionDesc` with owned fields | Field `type` becomes `ty` |
| `record hook-error` | `struct HookError { kind: HookErrorKind, message: String }` implementing `std::error::Error` | |
| `type timer-id` | `struct TimerId(u32)` | A newtype keeps timer ids apart from other integers |
| `interface engine` | trait `Host` | The engine implements it per worker |
| `interface template`, `functions` | `Extension::functions` | |
| `interface hooks`, `on-open` | `Extension::on_open` | Returns the boxed session state |
| `resource session` | `Box<dyn ExtensionSession>` | Dropping the box is the resource drop |
| `on-data`, `on-timer`, `on-close`, `render` | `ExtensionSession` methods | Each method also receives `host`, because a native call has no ambient session context |

Two differences between the paths are deliberate. Native calls borrow their inputs and write render output into a caller buffer, while the component model copies every list across the sandbox boundary. A trap in a component fails one session. The agent catches a panic in a native extension at the hook boundary and fails the session in the same way, but no fuel or memory limit guards native code against an endless loop or a memory safety defect in a dependency (decision record 0010).

## Validation

```bash
cargo install wasm-tools --locked
wasm-tools component wit contracts/extension/fps-extension.wit
```

The command parses and resolves the package and prints it back. A nonzero exit code means the file is invalid.

## Versioning

The package version follows the contract rules in `CONTRIBUTING.md`. A minor version may add engine calls, because a component that does not import a call is unaffected. Any other change alters the component type that older components were built against, including a new case in a type that crosses the boundary. Such a change needs a new minor version, and the engine keeps one adapter for every minor version that the `engine_api` ranges of installed modules still allow. Removing or renaming an item needs a major version.
