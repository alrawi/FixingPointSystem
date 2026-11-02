# D4. Core license

- Status: Accepted
- Date: 2026-10-30
- Deciders: repository owner

## Context

The project needs a license before the first public commit.

## Options

- Apache 2.0 (recommended default)
- MIT
- GPL-3.0 (chosen)

## Decision

We license the core under GPL-3.0. Every module and every library linked into the agent or controller must carry a license compatible with GPL-3.0. `deny.toml` enforces this rule for Rust crates.

## Consequences

Derivatives stay open. Tools with incompatible licenses, such as Scapy, run only as separate processes. Some vendors may decline to contribute.
