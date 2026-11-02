# D10. Native first party extensions

- Status: Proposed
- Date: 2026-11-02
- Deciders: repository owner

## Context

Decision D3 runs module extensions as WebAssembly components, which protects agents from faulty community code at a moderate cost per hook call. First party modules such as SSH and SMB need large protocol libraries and call hooks on every received chunk and every rendered message. These modules come from this repository, pass the same review as the agent, and ship under GPL-3.0. For them the sandbox adds copies across the boundary and a compile step to WebAssembly, and it excludes Rust libraries that do not build for a WebAssembly target. The extension interface in `contracts/extension/fps-extension.wit` defines one set of hooks, so the question is whether first party code may implement that interface without the sandbox.

## Options

- WebAssembly for every module, first party included
- Native Rust trait compiled into the agent for first party modules, WebAssembly for all other modules (proposed)
- Native shared libraries loaded at run time through a C ABI
- Extensions in a separate process that the agent calls over a local socket

## Decision

We propose that first party modules may ship native extensions. A native extension is a Rust crate in the module's `native/` directory that implements the `Extension` and `ExtensionSession` traits, which mirror the WIT world item for item as `contracts/extension/README.md` maps it. The agent links the crate at build time and registers it in a static module table. A later manifest schema change, which this record leaves to a follow up contract package, lets the `extension` object name the crate in place of the `wasm` file. Every other module, and any module installed from the registry, runs as a WebAssembly component through the same traits by way of the ENG-11 adapter.

Five rules bound the native path.

1. Only modules in this repository qualify, and an owner approved pull request adds each one to the module table.
2. The crate imports nothing beyond the `Host` trait for network, time, and randomness, and its dependencies carry GPL-3.0 compatible licenses.
3. The crate follows the Rust coding standards with `#![forbid(unsafe_code)]`, and bytes from the peer never cause a panic.
4. The interface conformance tests that ENG-11 adds run against both the native crate and the WebAssembly adapter.
5. The agent build catches a panic at the hook boundary and fails the session, so a defect fails sessions instead of the worker. The release profile therefore keeps the default `panic = "unwind"`.

## Consequences

First party hook calls cost the same as any Rust function call, and render output goes straight into the session send buffer without a copy. First party modules may use Rust protocol libraries that do not build for WebAssembly. Two implementations of one interface stay in step, because the interpreter calls only the traits and the conformance tests run on both paths.

We give up the sandbox for first party code. A memory safety defect in a dependency or an infinite loop in a native crate can still harm a worker, because fuel limits and memory limits apply only to components. Native modules ship only with agent releases, so a fix to one needs a new agent build rather than a registry update. The trust boundary moves from the sandbox to code review, which adds load at the gates.

The rejected options carry these costs. WebAssembly everywhere keeps one trust model but pays the boundary cost on the heaviest protocols and blocks some libraries. C ABI shared libraries repeat the risk that D3 rejected and add an unstable binary interface. A separate process isolates faults but adds a context switch to every hook call, which costs more than the sandbox.
