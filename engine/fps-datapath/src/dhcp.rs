// SPDX-License-Identifier: GPL-3.0-only
//! The DHCP hook: the clients of `fps-hosts` behind the [`FrameHook`] seam of
//! the splitter (TRN-37).
//!
//! The arm of a lane whose pool leases its addresses ([`HostPool::dhcp`])
//! runs one [`DhcpHosts`] on its queue. The splitter hands every DHCP frame
//! (UDP port 67 or 68) to [`DhcpHook::handle`], and on every pass of the
//! queue [`DhcpHook::poll_tx`] sends what is due and publishes the lease
//! table on the [`LeaseBoard`] of the arm. The responder, the stations of the
//! stack, and the route of the arm read the board, so they follow the leases.
//!
//! `DhcpHosts::send_due` wants a [`FrameIo`], and the queue belongs to the
//! splitter, so the hook gives it a small queue that collects the frames, and
//! pushes them as replies.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use fps_backend_afxdp::{FrameIo, QueueStats};
use fps_hosts::{DhcpConfig, DhcpHosts};
use fps_tcp_own::clock::Clock;
use fps_tcp_own::LeaseBoard;

use crate::split::{FrameHook, HookOutcome, Replies};

/// How the hosts of a leasing pool ask for their addresses, and how long the
/// lease phase may take.
#[derive(Clone, Debug)]
pub struct DhcpSettings {
    /// The timers, the burst, and the seed of the clients. The VLAN of the
    /// clients is the VLAN of the pools of the lane, and the seed of each
    /// arm is derived from this one, so a run repeats.
    pub config: DhcpConfig,
    /// How long [`crate::Datapath::wait_leases`] waits for every lease.
    pub timeout: Duration,
}

impl Default for DhcpSettings {
    /// The default clients and a lease phase of 60 seconds, which is what
    /// 20,000 hosts need when the server answers at line rate.
    fn default() -> Self {
        Self {
            config: DhcpConfig::default(),
            timeout: Duration::from_secs(60),
        }
    }
}

/// A queue that keeps the frames that were sent on it, up to a limit, for the
/// hook to push as replies.
struct Collect {
    frames: Vec<Vec<u8>>,
    room: usize,
    max: usize,
}

impl FrameIo for Collect {
    fn queue_id(&self) -> u32 {
        0
    }

    fn max_frame_len(&self) -> usize {
        self.max
    }

    fn recv_batch(&mut self, _max: usize, _frame: &mut dyn FnMut(&[u8])) -> io::Result<usize> {
        Ok(0)
    }

    fn send_batch_with(
        &mut self,
        count: usize,
        fill: &mut dyn FnMut(usize, &mut [u8]) -> usize,
    ) -> io::Result<usize> {
        let mut taken = 0;
        for i in 0..count {
            if self.frames.len() >= self.room {
                break;
            }
            let mut buf = vec![0u8; self.max];
            let len = fill(i, &mut buf);
            if len == 0 || len > self.max {
                break;
            }
            buf.truncate(len);
            self.frames.push(buf);
            taken += 1;
        }
        Ok(taken)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn wait_readable(&mut self, _timeout: Duration) -> io::Result<bool> {
        Ok(false)
    }

    fn stats(&self) -> QueueStats {
        QueueStats::default()
    }
}

/// The hook of one arm that runs the DHCP clients of its hosts.
pub(crate) struct DhcpHook {
    hosts: DhcpHosts,
    board: Arc<LeaseBoard>,
    clock: Arc<dyn Clock>,
    max_frame: usize,
    /// The generation and the bound count that the board holds.
    published: (u64, usize),
}

impl DhcpHook {
    pub(crate) fn new(
        hosts: DhcpHosts,
        board: Arc<LeaseBoard>,
        clock: Arc<dyn Clock>,
        max_frame: usize,
    ) -> Self {
        Self {
            hosts,
            board,
            clock,
            max_frame,
            published: (u64::MAX, usize::MAX),
        }
    }

    fn now(&self) -> Duration {
        Duration::from_micros(u64::try_from(self.clock.now_micros()).unwrap_or(0))
    }

    fn publish(&mut self) {
        let table = self.hosts.leases();
        let state = (table.generation(), table.bound());
        if state != self.published {
            self.board.publish(table);
            self.published = state;
        }
    }
}

impl FrameHook for DhcpHook {
    /// Every DHCP frame is the hook's: a reply goes to its client, and a
    /// frame for the server port (a client of another host) needs no answer.
    fn handle(&mut self, frame: &[u8], _replies: &mut Replies<'_>) -> HookOutcome {
        let now = self.now();
        self.hosts.handle_frame(frame, now);
        HookOutcome::Handled
    }

    fn poll_tx(&mut self, replies: &mut Replies<'_>) {
        let now = self.now();
        // Publish before sending, so the replies of this pass are visible.
        self.publish();
        let room = replies.room();
        if room > 0 {
            let mut out = Collect {
                frames: Vec::new(),
                room,
                max: self.max_frame,
            };
            // The collector never fails.
            let _ = self.hosts.send_due(&mut out, now);
            for frame in out.frames {
                replies.push_vec(frame);
            }
        }
        self.publish();
    }
}
