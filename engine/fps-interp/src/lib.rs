// SPDX-License-Identifier: GPL-3.0-only
//! Flow program interpreter.
//!
//! The interpreter runs one side of one session, a compiled program from
//! `fps-flow`, over a transport that implements `fps_io::SessionIo`. The
//! crate is empty until ENG-04 ports the interpreter of spike S1.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
