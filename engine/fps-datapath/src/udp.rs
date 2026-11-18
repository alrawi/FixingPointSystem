// SPDX-License-Identifier: GPL-3.0-only
//! The UDP hook: the UDP layer of `fps-udp-own` behind the [`FrameHook`] seam
//! of the splitter (TRN-29).

use fps_udp_own::UdpArm;

use crate::split::{FrameHook, HookOutcome, Replies};

/// The hook of one queue that carries IPv4 UDP frames to a [`UdpArm`] and
/// takes the frames that the sessions of the arm queued.
pub(crate) struct UdpHook(pub(crate) UdpArm);

impl FrameHook for UdpHook {
    /// The layer verifies the frame and counts what it drops, so the splitter
    /// counts every UDP frame as handled. The counters of the layer say how
    /// many were delivered.
    fn handle(&mut self, frame: &[u8], _replies: &mut Replies<'_>) -> HookOutcome {
        self.0.receive(frame);
        HookOutcome::Handled
    }

    fn poll_tx(&mut self, replies: &mut Replies<'_>) {
        let room = replies.room();
        if room > 0 {
            self.0.drain_tx(room, &mut |frame| replies.push_vec(frame));
        }
    }
}
