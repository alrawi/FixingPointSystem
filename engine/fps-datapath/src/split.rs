// SPDX-License-Identifier: GPL-3.0-only
//! The splitter, the layer of one queue between the wire and a stack.
//!
//! A [`Splitter`] wraps the [`FrameIo`] of one queue and is itself a
//! `FrameIo`. The stack above it sees TCP frames only, over IPv4 and over
//! IPv6. Every other frame stops here: ARP and IPv6 neighbor solicitations go
//! to the `fps-hosts` responder, whose replies the splitter sends on its own
//! (TRN-30: a peer resolves the host pools over IPv6 the way it does over
//! IPv4), and DHCP, UDP, and the neighbor discovery messages that are not
//! solicitations for a host go to a [`FrameHook`]. The UDP hook of a datapath
//! is the UDP layer of `fps-udp-own` (TRN-29, with IPv6 from TRN-30). A hook
//! that nothing replaced answers [`HookOutcome::Unsupported`]. A frame that
//! nothing takes is counted and dropped.
//!
//! A hook also sends: the splitter asks every hook for frames of its own
//! ([`FrameHook::poll_tx`]) on each pass of the queue, and sends them on the
//! queue after the replies.

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use fps_backend_afxdp::{FrameIo, QueueStats};
use fps_hosts::wire::MAX_REPLY_LEN;
use fps_hosts::{Responder, Verdict};

use crate::classify::{classify, Class};

/// The most replies that wait for a free transmit slot. A queue that stays
/// full drops the oldest replies, which a peer asks for again.
const MAX_PENDING: usize = 1024;
/// The most frames that one attempt to send replies hands to the queue.
const REPLY_BATCH: usize = 32;
/// The most frames that a hook may add on one pass of its queue.
const TX_BATCH: usize = 256;

/// What a hook did with a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookOutcome {
    /// The hook took the frame.
    Handled,
    /// Nothing handles this protocol yet.
    Unsupported,
}

/// The place where a hook puts frames that it wants to send on the queue of
/// the frame that it handles.
pub struct Replies<'a> {
    queue: &'a mut VecDeque<Vec<u8>>,
    pushed: usize,
}

impl Replies<'_> {
    /// Queues one Ethernet frame for transmission. A frame that is empty or
    /// longer than the queue allows is dropped when the splitter sends it.
    pub fn push(&mut self, frame: &[u8]) {
        self.push_vec(frame.to_vec());
    }

    /// Like [`Replies::push`], for a frame that the caller owns, so it is not
    /// copied again.
    pub fn push_vec(&mut self, frame: Vec<u8>) {
        if self.queue.len() >= MAX_PENDING {
            self.queue.pop_front();
        }
        self.queue.push_back(frame);
        self.pushed += 1;
    }

    /// How many frames fit before the oldest one would be dropped. A hook
    /// that sends on its own ([`FrameHook::poll_tx`]) stops at this number
    /// and goes on at the next pass.
    #[must_use]
    pub fn room(&self) -> usize {
        MAX_PENDING.saturating_sub(self.queue.len()).min(TX_BATCH)
    }
}

/// The handler of one protocol that the datapath does not serve itself.
///
/// TRN-29 plugs UDP into this seam (`fps_udp_own::UdpArm`), HST-04 DHCP
/// (`fps_hosts::DhcpHosts`), and a later package IPv6 neighbor discovery. A
/// hook runs on the thread of its queue, so it must not block.
pub trait FrameHook: Send {
    /// Handles one frame of the class that the hook serves.
    fn handle(&mut self, frame: &[u8], replies: &mut Replies<'_>) -> HookOutcome;

    /// Offers the hook a turn to send frames that it did not make in answer
    /// to a frame, for example the datagrams that tasks queued. The splitter
    /// calls it on every pass of the queue, after the frames of the pass were
    /// handled. A hook adds at most [`Replies::room`] frames per call. The
    /// default sends nothing.
    fn poll_tx(&mut self, _replies: &mut Replies<'_>) {}
}

/// A hook for a protocol that is not served yet.
#[derive(Clone, Copy, Debug, Default)]
pub struct Unsupported;

impl FrameHook for Unsupported {
    fn handle(&mut self, _frame: &[u8], _replies: &mut Replies<'_>) -> HookOutcome {
        HookOutcome::Unsupported
    }
}

/// The hooks of one side.
pub struct Hooks {
    /// IPv6 neighbor discovery that the responder does not answer: router
    /// solicitations and advertisements, neighbor advertisements, and
    /// redirects.
    pub nd: Box<dyn FrameHook>,
    /// DHCP on UDP ports 67 and 68.
    pub dhcp: Box<dyn FrameHook>,
    /// All other UDP, over IPv4 and IPv6.
    pub udp: Box<dyn FrameHook>,
}

impl Default for Hooks {
    fn default() -> Self {
        Self {
            nd: Box::new(Unsupported),
            dhcp: Box::new(Unsupported),
            udp: Box::new(Unsupported),
        }
    }
}

/// What a splitter did with the frames of its queue.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameCounters {
    /// Frames taken from the queue.
    pub rx_frames: u64,
    /// TCP frames, IPv4 and IPv6, passed to the stack.
    pub tcp: u64,
    /// ARP requests answered.
    pub arp_replied: u64,
    /// ARP frames that need no answer: requests for other stations,
    /// announcements, and replies.
    pub arp_ignored: u64,
    /// Neighbor solicitations for a host that the splitter answered.
    pub nd_replied: u64,
    /// Solicitations for a station that is not a host of the port, and
    /// announcements.
    pub nd_ignored: u64,
    /// ND frames that a hook took.
    pub nd_handled: u64,
    /// ND frames that nothing handles yet.
    pub nd_unsupported: u64,
    /// DHCP frames that a hook took.
    pub dhcp_handled: u64,
    /// DHCP frames that nothing handles yet.
    pub dhcp_unsupported: u64,
    /// UDP frames that a hook took.
    pub udp_handled: u64,
    /// UDP frames that nothing handles yet.
    pub udp_unsupported: u64,
    /// Frames of another protocol or Ethernet type.
    pub other_dropped: u64,
    /// Frames with another VLAN tag than the port has.
    pub vlan_dropped: u64,
    /// Frames too short for their headers, and neighbor requests that break
    /// their protocol.
    pub malformed: u64,
    /// Replies sent by the splitter.
    pub replies_sent: u64,
    /// Replies that were dropped, because the queue refused them or stayed
    /// full.
    pub replies_dropped: u64,
}

/// The counters behind a [`FrameCounters`]. The splitter lives inside the
/// stack, so the datapath reads the counters through a shared handle.
#[derive(Debug, Default)]
pub struct SharedCounters {
    rx_frames: AtomicU64,
    tcp: AtomicU64,
    arp_replied: AtomicU64,
    arp_ignored: AtomicU64,
    nd_replied: AtomicU64,
    nd_ignored: AtomicU64,
    nd_handled: AtomicU64,
    nd_unsupported: AtomicU64,
    dhcp_handled: AtomicU64,
    dhcp_unsupported: AtomicU64,
    udp_handled: AtomicU64,
    udp_unsupported: AtomicU64,
    other_dropped: AtomicU64,
    vlan_dropped: AtomicU64,
    malformed: AtomicU64,
    replies_sent: AtomicU64,
    replies_dropped: AtomicU64,
}

fn bump(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

impl SharedCounters {
    /// Reads all counters.
    #[must_use]
    pub fn snapshot(&self) -> FrameCounters {
        let get = |c: &AtomicU64| c.load(Ordering::Relaxed);
        FrameCounters {
            rx_frames: get(&self.rx_frames),
            tcp: get(&self.tcp),
            arp_replied: get(&self.arp_replied),
            arp_ignored: get(&self.arp_ignored),
            nd_replied: get(&self.nd_replied),
            nd_ignored: get(&self.nd_ignored),
            nd_handled: get(&self.nd_handled),
            nd_unsupported: get(&self.nd_unsupported),
            dhcp_handled: get(&self.dhcp_handled),
            dhcp_unsupported: get(&self.dhcp_unsupported),
            udp_handled: get(&self.udp_handled),
            udp_unsupported: get(&self.udp_unsupported),
            other_dropped: get(&self.other_dropped),
            vlan_dropped: get(&self.vlan_dropped),
            malformed: get(&self.malformed),
            replies_sent: get(&self.replies_sent),
            replies_dropped: get(&self.replies_dropped),
        }
    }
}

/// The layer of one queue that sends each frame where it belongs.
pub struct Splitter {
    inner: Box<dyn FrameIo>,
    responder: Responder,
    vlan: Option<u16>,
    hooks: Hooks,
    pending: VecDeque<Vec<u8>>,
    counters: Arc<SharedCounters>,
}

impl Splitter {
    /// Wraps `inner` for a port that serves the hosts of `responder` and has
    /// the VLAN `vlan`. The returned counters stay readable after the
    /// splitter moved into a stack.
    #[must_use]
    pub fn new(
        inner: Box<dyn FrameIo>,
        responder: Responder,
        vlan: Option<u16>,
        hooks: Hooks,
    ) -> (Self, Arc<SharedCounters>) {
        let counters = Arc::new(SharedCounters::default());
        let splitter = Self {
            inner,
            responder,
            vlan,
            hooks,
            pending: VecDeque::new(),
            counters: Arc::clone(&counters),
        };
        (splitter, counters)
    }

    /// Sends the replies that wait, as many as the queue takes.
    fn drain_pending(&mut self) {
        drain(&mut *self.inner, &mut self.pending, &self.counters);
    }
}

fn drain(inner: &mut dyn FrameIo, pending: &mut VecDeque<Vec<u8>>, counters: &SharedCounters) {
    let mut sent_any = false;
    while !pending.is_empty() {
        let result = {
            let frames: Vec<&[u8]> = pending
                .iter()
                .take(REPLY_BATCH)
                .map(Vec::as_slice)
                .collect();
            inner.send_batch(&frames)
        };
        match result {
            Ok(0) => break,
            Ok(count) => {
                pending.drain(..count);
                counters
                    .replies_sent
                    .fetch_add(count as u64, Ordering::Relaxed);
                sent_any = true;
            }
            Err(_) => {
                // A reply that the queue refuses can never be sent.
                pending.pop_front();
                bump(&counters.replies_dropped);
            }
        }
    }
    if sent_any {
        let _ = inner.flush();
    }
}

fn run_hook(
    hook: &mut dyn FrameHook,
    frame: &[u8],
    pending: &mut VecDeque<Vec<u8>>,
    (handled, unsupported): (&AtomicU64, &AtomicU64),
) {
    let mut replies = Replies {
        queue: pending,
        pushed: 0,
    };
    match hook.handle(frame, &mut replies) {
        HookOutcome::Handled => bump(handled),
        HookOutcome::Unsupported => bump(unsupported),
    }
}

/// Gives every hook its turn to send, and returns the frames that they queued.
fn poll_hooks(hooks: &mut Hooks, pending: &mut VecDeque<Vec<u8>>) -> usize {
    let mut queued = 0;
    for hook in [&mut hooks.nd, &mut hooks.dhcp, &mut hooks.udp] {
        let mut replies = Replies {
            queue: pending,
            pushed: 0,
        };
        hook.poll_tx(&mut replies);
        queued += replies.pushed;
    }
    queued
}

/// Queues a reply of the responder, and drops the oldest one when the queue
/// stays full.
fn queue_reply(pending: &mut VecDeque<Vec<u8>>, reply: &[u8], counters: &SharedCounters) {
    if pending.len() >= MAX_PENDING {
        pending.pop_front();
        bump(&counters.replies_dropped);
    }
    pending.push_back(reply.to_vec());
}

/// Sends one frame where its class says.
fn dispatch(
    frame: &[u8],
    vlan: Option<u16>,
    responder: &Responder,
    hooks: &mut Hooks,
    pending: &mut VecDeque<Vec<u8>>,
    counters: &SharedCounters,
    to_stack: &mut dyn FnMut(&[u8]),
) {
    bump(&counters.rx_frames);
    let parsed = classify(frame);
    if parsed.class == Class::Malformed {
        bump(&counters.malformed);
        return;
    }
    if !parsed.tagging.matches(vlan) {
        bump(&counters.vlan_dropped);
        return;
    }
    match parsed.class {
        Class::Ipv4Tcp | Class::Ipv6Tcp => {
            bump(&counters.tcp);
            to_stack(frame);
        }
        Class::Arp => {
            let mut out = [0u8; MAX_REPLY_LEN];
            match responder.process(frame, &mut out) {
                Verdict::Reply(_, len) => {
                    if let Some(reply) = out.get(..len) {
                        queue_reply(pending, reply, counters);
                        bump(&counters.arp_replied);
                    }
                }
                Verdict::Malformed(_) => bump(&counters.malformed),
                Verdict::Pass | Verdict::NotForUs(_) => bump(&counters.arp_ignored),
            }
        }
        Class::Nd => {
            // The responder answers the solicitations for the hosts of the
            // port, as it answers ARP. The hook gets what it passes.
            let mut out = [0u8; MAX_REPLY_LEN];
            match responder.process(frame, &mut out) {
                Verdict::Reply(_, len) => {
                    if let Some(reply) = out.get(..len) {
                        queue_reply(pending, reply, counters);
                        bump(&counters.nd_replied);
                    }
                }
                Verdict::Malformed(_) => bump(&counters.malformed),
                Verdict::NotForUs(_) => bump(&counters.nd_ignored),
                Verdict::Pass => run_hook(
                    &mut *hooks.nd,
                    frame,
                    pending,
                    (&counters.nd_handled, &counters.nd_unsupported),
                ),
            }
        }
        Class::Dhcp => run_hook(
            &mut *hooks.dhcp,
            frame,
            pending,
            (&counters.dhcp_handled, &counters.dhcp_unsupported),
        ),
        Class::Ipv4Udp | Class::Ipv6Udp => run_hook(
            &mut *hooks.udp,
            frame,
            pending,
            (&counters.udp_handled, &counters.udp_unsupported),
        ),
        Class::Ipv4Other(_) | Class::Ipv6Other | Class::Unknown(_) => {
            bump(&counters.other_dropped);
        }
        Class::Malformed => {}
    }
}

impl FrameIo for Splitter {
    fn queue_id(&self) -> u32 {
        self.inner.queue_id()
    }

    fn max_frame_len(&self) -> usize {
        self.inner.max_frame_len()
    }

    /// Takes frames from the queue and passes the IPv4 TCP frames to
    /// `frame`. The count is the number of frames that came off the queue
    /// plus the frames that hooks queued for sending, so a pass that only saw
    /// ARP, or only sent datagrams, still counts as work.
    fn recv_batch(&mut self, max: usize, frame: &mut dyn FnMut(&[u8])) -> io::Result<usize> {
        let Self {
            inner,
            responder,
            vlan,
            hooks,
            pending,
            counters,
        } = self;
        let taken = inner.recv_batch(max, &mut |f| {
            dispatch(f, *vlan, responder, hooks, pending, counters, frame);
        })?;
        let queued = poll_hooks(hooks, pending);
        if !pending.is_empty() {
            drain(&mut **inner, pending, counters);
        }
        Ok(taken + queued)
    }

    fn send_batch_with(
        &mut self,
        count: usize,
        fill: &mut dyn FnMut(usize, &mut [u8]) -> usize,
    ) -> io::Result<usize> {
        self.drain_pending();
        if !self.pending.is_empty() {
            // The queue is full. The caller flushes and tries again.
            return Ok(0);
        }
        self.inner.send_batch_with(count, fill)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.drain_pending();
        self.inner.flush()
    }

    fn wait_readable(&mut self, timeout: Duration) -> io::Result<bool> {
        self.inner.wait_readable(timeout)
    }

    fn stats(&self) -> QueueStats {
        self.inner.stats()
    }
}
