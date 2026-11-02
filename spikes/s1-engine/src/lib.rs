//! Phase 0 spike S1: a flow program interpreter over kernel sockets.
//!
//! The crate measures how fast a Rust agent runs contract v0.1 scenarios on
//! the kernel socket backend, and how much the interpreter costs relative to
//! a hand written client and server that send the same bytes.

pub mod engine;
pub mod interp;
pub mod program;
pub mod template;
