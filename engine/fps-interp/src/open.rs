// SPDX-License-Identifier: GPL-3.0-only
//! The `connect` and `accept` seam.
//!
//! The interpreter renders the host and port of the opening action and asks
//! an [`Opener`] for the session transport. A transport backend implements
//! the trait over its sockets, and tests hand out ends of the in memory
//! duplex of `fps-io`.

use std::future::Future;
use std::io;

use fps_io::SessionIo;

/// The action that opens a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenKind {
    /// The client side dials the peer.
    Connect,
    /// The server side accepts one session.
    Accept,
}

/// A rendered `connect` or `accept` action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpenRequest<'a> {
    /// The opening action.
    pub kind: OpenKind,
    /// The rendered host of `connect`, or `None` when the engine picks an
    /// address from the server pool.
    pub host: Option<&'a [u8]>,
    /// The rendered port.
    pub port: u16,
}

/// Opens the transport of one session side.
///
/// The interpreter bounds the returned future with its open timeout and
/// classifies errors by kind. [`io::ErrorKind::ConnectionRefused`] fails the
/// session with reason `connect_refused`, and [`io::ErrorKind::TimedOut`]
/// with reason `connect_timeout`.
pub trait Opener {
    /// The transport of an opened session.
    type Io: SessionIo;

    /// Opens the session that `request` describes.
    fn open(&mut self, request: OpenRequest<'_>) -> impl Future<Output = io::Result<Self::Io>>;
}
