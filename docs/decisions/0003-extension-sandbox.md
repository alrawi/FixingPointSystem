# D3. Extension sandbox

- Status: Accepted
- Date: 2026-10-30
- Deciders: repository owner

## Context

Some protocols need computation that declarative flow programs cannot express, such as SSH key exchange and SMB signing.

## Options

- WebAssembly (chosen)
- Native C ABI plugins
- Embedded Lua

## Decision

Modules ship such logic as WebAssembly components that implement the hooks on_open, on_data, on_timer, on_close, and render.

## Consequences

Faulty community code cannot crash or compromise an agent. Each hook call pays a moderate speed cost.
