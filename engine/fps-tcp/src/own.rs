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
    host, net, wire, OwnConfig, OwnCounters, OwnSession, OwnStack, OwnStats, PoolError, Pools,
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
            "retransmission is minimal: one timer per connection, a fixed initial timeout that doubles, go back N, no round trip estimate and no fast retransmit, and the link loses nothing, so the measurements never retransmit".to_string(),
            "there is no congestion control, no window scaling (the window is at most 65,535 bytes), no delayed acknowledgment, and no out of order queue".to_string(),
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
        }
    }
}
