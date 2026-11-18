// SPDX-License-Identifier: GPL-3.0-only
//! The seam between the bench harness and a TCP stack.

use std::future::Future;
use std::io;

use fps_io::SessionIo;

/// Counters that a stack reports about its pair, for the benchmark reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StackStats {
    /// Sockets that the pair holds now, counted on both hosts.
    pub sockets_open: u64,
    /// Passes of the driver that moved at least one frame or socket event.
    /// A stack without a user space driver reports zero.
    pub pump_passes: u64,
    /// Frames that both hosts transmitted since the pair was created. A stack
    /// that cannot see its frames reports zero.
    pub frames: u64,
    /// Data segments that a sender sent again, counted at the link. A stack
    /// whose link is not impaired reports zero.
    pub retransmits: u64,
    /// Frames that the impaired link lost at random.
    pub link_lost: u64,
    /// Frames that the bounded queue of the impaired link dropped.
    pub link_queue_dropped: u64,
}

/// A TCP stack under test, seen as one client host and one server host that a
/// link joins.
///
/// Every session leaves the stack as a [`SessionIo`], which is the seam the
/// engine uses, so a measurement includes the cost of the adapter. The pair is
/// cheap to clone, and all clones share the same two hosts. A stack makes
/// progress only while a task polls the future of [`TcpStack::drive`].
pub trait TcpStack: Clone + Send + Sync + 'static {
    /// The session type that both `connect` and `accept` return.
    type Session: SessionIo + 'static;

    /// Names the stack and its version for the reports.
    fn name(&self) -> String;

    /// Lists the facts about the stack and its link that a reader needs to
    /// interpret its numbers, one sentence each.
    fn notes(&self) -> Vec<String>;

    /// Starts listening on `port` at the server host.
    ///
    /// A port may be listened on once.
    fn listen(&self, port: u16) -> io::Result<()>;

    /// Opens a session from the client host to `port` at the server host and
    /// completes when the handshake finished.
    fn connect(&self, port: u16) -> impl Future<Output = io::Result<Self::Session>> + Send;

    /// Completes with the next session that arrived at `port` on the server
    /// host.
    fn accept(&self, port: u16) -> impl Future<Output = io::Result<Self::Session>> + Send;

    /// Moves frames and timers for both hosts until the future is dropped. A
    /// stack that the operating system drives waits forever.
    fn drive(&self) -> impl Future<Output = ()> + Send;

    /// Runs one complete poll of both hosts at once and returns, so a
    /// benchmark can time the cost of holding the current sockets. A stack
    /// without a user space poll does nothing.
    fn poll_once(&self);

    /// Returns the counters of the pair.
    fn stats(&self) -> StackStats;
}
