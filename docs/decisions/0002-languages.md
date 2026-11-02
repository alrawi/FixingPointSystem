# D2. Implementation languages

- Status: Accepted
- Date: 2026-10-30
- Deciders: repository owner

## Context

The agent parses untrusted responses from devices under test at high rates. The controller must interoperate with the Open Traffic Generator ecosystem.

## Options

- Rust agent, Go controller (chosen)
- C++ agent, Go controller
- Rust agent, Python controller
- Go for both

## Decision

We write the agent in Rust and the controller in Go. Tooling scripts use Python.

## Consequences

Memory safety in the data plane, mature DPDK bindings in Rust, and gosnappi compatibility in Go. Contributors need two primary languages.
