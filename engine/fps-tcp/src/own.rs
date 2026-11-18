// SPDX-License-Identifier: GPL-3.0-only
//! The adapter of the own stack for the bench harness.
//!
//! The stack lives in the crate [`fps_tcp_own`] (TRN-21), which does not
//! depend on smoltcp, so an agent that runs the own stack does not link it.
//! This module implements [`TcpStack`] for [`OwnStack`] and keeps the paths
//! `fps_tcp::own::{host, wire}` that the tests and reports used before the
//! move.

use std::future::Future;
use std::io;

pub use fps_tcp_own::{
    host, net, wire, CongestionControl, OwnConfig, OwnCounters, OwnSession, OwnStack, OwnStats,
    PoolError, Pools,
};

use crate::stack::{StackStats, TcpStack};

/// The name that the reports use for this stack.
pub const OWN_STACK_NAME: &str = "own minimal TCP (fps-tcp)";

impl TcpStack for OwnStack {
    type Session = OwnSession;

    fn name(&self) -> String {
        OWN_STACK_NAME.to_string()
    }

    fn notes(&self) -> Vec<String> {
        vec![
            "the client side and the server side are two instances of the own TCP state machine in one process, joined by an in memory pair of frame queues of fps-backend-afxdp, so there is no kernel socket and no descriptor".to_string(),
            "frames are Ethernet (802.1Q tagged when --own-vlan is set) with IPv4 and TCP, with an IP header checksum and a TCP checksum that are computed on send and verified on receive; the MAC address of host i of a pool is mac_base + i and the neighbor of an address is read from the HST-03 table of fps-hosts".to_string(),
            "the client side is a pool of hosts (--own-pool) and the server side a pool of hosts (--own-server-hosts); the connection table is keyed by the 4-tuple of both addresses and both ports, and a connect takes the next client host in turn".to_string(),
            "loss recovery (TRN-23): a round trip estimator with the retransmission timeout of RFC 6298 (minimum 200 ms, Karn's rule, exponential backoff), fast retransmit on three duplicate acknowledgments with Limited Transmit, an out of order receive queue, and NewReno with an initial window of 10 segments; the state sits in a box that a connection holds only while data is in flight or waits behind a hole".to_string(),
            "the in memory link of the lossless workloads loses nothing, so they never retransmit".to_string(),
            "options for long fat paths (TRN-24): window scaling (RFC 7323, a 1 MiB receive buffer gives a shift of 5), timestamps (round trip samples and PAWS), selective acknowledgment (RFC 2018 blocks from the out of order queue and the scoreboard and pipe of RFC 6675 beside NewReno), and a delayed acknowledgment (every second segment or 40 ms, at once for out of order data); each is offered in the SYN and used when the peer offers it too; the scoreboard lives in the box of the connection, so an idle socket holds only 6 bytes of option state".to_string(),
            "congestion control (TRN-25): CUBIC (RFC 9438: the cubic window function of the time since the last event, a decrease to 0.7, fast convergence, and the reno friendly region; ported from smoltcp 0.14.0) is the default and NewReno stays selectable (--own-cc); the CUBIC state is part of the box of the connection, so an idle socket pays nothing for it".to_string(),
            "timers live in a heap with one entry per connection, so a poll with idle sockets does not grow with the socket count, where a stack that walks its sockets does".to_string(),
            "a closed tuple is held for one second in TIME-WAIT instead of minutes, so a rate run does not run out of ports".to_string(),
            "bytes_per_socket is the growth of the resident set of the process and includes the connection table, the buffers that the held sockets own (none while idle), and the frame link".to_string(),
        ]
    }

    fn listen(&self, port: u16) -> io::Result<()> {
        OwnStack::listen(self, port)
    }

    fn connect(&self, port: u16) -> impl Future<Output = io::Result<OwnSession>> + Send {
        OwnStack::connect(self, port)
    }

    fn accept(&self, port: u16) -> impl Future<Output = io::Result<OwnSession>> + Send {
        OwnStack::accept(self, port)
    }

    fn drive(&self) -> impl Future<Output = ()> + Send {
        OwnStack::drive(self)
    }

    fn poll_once(&self) {
        self.pump();
    }

    fn stats(&self) -> StackStats {
        let stats = OwnStack::stats(self);
        StackStats {
            sockets_open: stats.sockets_open,
            pump_passes: stats.pump_passes,
            frames: stats.frames,
            ..StackStats::default()
        }
    }
}
