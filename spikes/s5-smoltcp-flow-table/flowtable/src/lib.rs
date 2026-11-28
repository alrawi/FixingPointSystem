// SPDX-License-Identifier: GPL-3.0-only
//! Spike SPK-05: can a smoltcp TCP socket process frames outside its
//! interface loop, behind a flow table and a timer wheel of our own?
//!
//! * [`table`] is the prototype: a flow table with one socket per flow.
//! * [`wheel`] is its timer wheel.
//! * [`dev`] is the frame device that replaces the loop's device.
//! * [`baseline`] is the interface loop of smoltcp over one socket set, for
//!   comparison.
//! * [`scenario`] runs both ends of N connections through handshake, request
//!   and response, loss with retransmission, and close, and measures the cost
//!   of each event.

#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod baseline;
pub mod dev;
pub mod scenario;
pub mod stats;
pub mod table;
pub mod wheel;
