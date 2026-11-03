// SPDX-License-Identifier: GPL-3.0-only
//! Flow program interpreter.
//!
//! The interpreter runs one side of one session, a compiled program from
//! `fps-flow`, over a transport that implements [`fps_io::SessionIo`]. A
//! [`Session`] renders the opening `connect` or `accept`, asks an [`Opener`]
//! for the transport, and then runs `send`, the three `expect` stop rules,
//! `delay`, `set`, `loop`, `branch`, and `close`. Every random draw comes
//! from the generator that [`session_rng`] derives from the run seed and the
//! session id, and time comes from the Tokio clock, so tests pause it.
//!
//! A failed side returns a [`Failure`], whose [`Reason`] is one value of the
//! `reason` label of `contracts/metrics/catalog.yaml`.
//!
//! UDP flows run over [`fps_io::DatagramIo`] with
//! [`Session::run_datagram_program`] and a [`DatagramOpener`]. Each `send`
//! writes one datagram, each `expect` reads and consumes exactly one
//! datagram, which also becomes `last`, and `close` releases the socket.
//! The `spawn` action belongs to a later package.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod delay;
pub mod failure;
pub mod open;
pub mod rng;
pub mod session;

pub use failure::{Failure, FailureKind, Reason};
pub use open::{DatagramOpener, OpenKind, OpenRequest, Opener};
pub use rng::{session_rng, Side};
pub use session::{Limits, Session, SessionCtx, SideStats, VECTORED_FILL_MIN};
