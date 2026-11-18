// SPDX-License-Identifier: GPL-3.0-only
//! The own stack over an impaired link (TRN-22).
//!
//! The in memory frame link of the own stack lives in `fps-backend-afxdp`, and
//! the own stack crate is not part of this package, so the impairment is a
//! wrapper: [`LossyFrames`] implements [`FrameIo`] for one end of a link whose
//! two directions are [`Impairment`] models, and [`LossyOwn`] builds an
//! [`OwnStack`] on two such ends and implements [`TcpStack`] for it.
//!
//! A delayed frame must reach the receiving host when its time comes, even
//! when no session acts. The driver of the own stack sleeps until its next
//! timer, so [`LossyOwn::drive`] also runs a ticker that pumps the stack when
//! a held frame falls due, and a sender wakes the ticker when it offers a
//! frame.

use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use fps_backend_afxdp::{FrameIo, QueueStats};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::clock::Clock;
use crate::impair::{Framing, ImpairConfig, ImpairedLink, LinkStats};
use crate::own::{OwnConfig, OwnSession, OwnStack, OWN_STACK_NAME};
use crate::stack::{StackStats, TcpStack};

/// Bytes that an Ethernet header and an 802.1Q tag add to the IP MTU.
const FRAME_OVERHEAD: usize = 14 + 4;
/// The frames that one direction may hold at once.
const DEPTH: usize = 65_536;

struct State {
    link: ImpairedLink,
}

struct Shared {
    state: Mutex<State>,
    clock: Arc<dyn Clock>,
    wake: Notify,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One end of an impaired in memory link, as a frame queue of a host.
pub struct LossyFrames {
    shared: Arc<Shared>,
    /// 0 for the client end, 1 for the server end.
    side: usize,
    max_len: usize,
    scratch: Vec<u8>,
    stats: QueueStats,
}

impl LossyFrames {
    /// Creates the client end and the server end of a link whose frames are
    /// at most `max_len` bytes. Time comes from `clock`.
    fn pair(
        config: ImpairConfig,
        max_len: usize,
        clock: Arc<dyn Clock>,
    ) -> (Self, Self, Arc<Shared>) {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                link: ImpairedLink::new(config, Framing::Ethernet),
            }),
            clock,
            wake: Notify::new(),
        });
        let end = |side| Self {
            shared: Arc::clone(&shared),
            side,
            max_len,
            scratch: vec![0; max_len],
            stats: QueueStats::default(),
        };
        (end(0), end(1), shared)
    }
}

impl FrameIo for LossyFrames {
    fn queue_id(&self) -> u32 {
        0
    }

    fn max_frame_len(&self) -> usize {
        self.max_len
    }

    fn recv_batch(&mut self, max: usize, frame: &mut dyn FnMut(&[u8])) -> io::Result<usize> {
        let now = self.shared.clock.now_micros();
        let mut taken = Vec::new();
        {
            let mut state = self.shared.lock();
            let from = &mut state.link.dirs[1 - self.side];
            while taken.len() < max {
                match from.pop_due(now) {
                    Some(f) => taken.push(f),
                    None => break,
                }
            }
        }
        for f in &taken {
            frame(f);
            self.stats.rx_frames += 1;
            self.stats.rx_bytes += f.len() as u64;
        }
        Ok(taken.len())
    }

    fn send_batch_with(
        &mut self,
        count: usize,
        fill: &mut dyn FnMut(usize, &mut [u8]) -> usize,
    ) -> io::Result<usize> {
        let now = self.shared.clock.now_micros();
        let mut queued = 0;
        {
            let mut state = self.shared.lock();
            let to = &mut state.link.dirs[self.side];
            let room = DEPTH.saturating_sub(to.held());
            if room < count {
                self.stats.tx_full += 1;
            }
            for i in 0..count.min(room) {
                let len = fill(i, &mut self.scratch);
                let Some(bytes) = self.scratch.get(..len).filter(|b| !b.is_empty()) else {
                    break;
                };
                to.offer(now, bytes.to_vec());
                self.stats.tx_frames += 1;
                self.stats.tx_bytes += len as u64;
                queued += 1;
            }
        }
        if queued > 0 {
            self.shared.wake.notify_one();
        }
        Ok(queued)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn wait_readable(&mut self, timeout: Duration) -> io::Result<bool> {
        let start = self.shared.clock.now_micros();
        let limit = start.saturating_add(i64::try_from(timeout.as_micros()).unwrap_or(i64::MAX));
        loop {
            let now = self.shared.clock.now_micros();
            let next = self.shared.lock().link.dirs[1 - self.side].next_release();
            match next {
                Some(at) if at <= now => return Ok(true),
                _ if now >= limit => return Ok(false),
                _ => std::thread::sleep(Duration::from_micros(100)),
            }
        }
    }

    fn stats(&self) -> QueueStats {
        self.stats
    }
}

/// Aborts a task when dropped.
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The own stack with its client and server joined by an impaired link.
#[derive(Clone)]
pub struct LossyOwn {
    stack: OwnStack,
    shared: Arc<Shared>,
    config: ImpairConfig,
}

impl LossyOwn {
    /// Creates the pair. Its time, and the time of the link, come from
    /// `clock`.
    ///
    /// # Errors
    ///
    /// Returns a sentence when the link settings are invalid.
    pub fn new(own: OwnConfig, link: ImpairConfig, clock: Arc<dyn Clock>) -> Result<Self, String> {
        link.validate()?;
        let (client, server, shared) =
            LossyFrames::pair(link, own.mtu + FRAME_OVERHEAD, Arc::clone(&clock));
        let stack = OwnStack::with_links(Box::new(client), Box::new(server), own, clock);
        Ok(Self {
            stack,
            shared,
            config: link,
        })
    }

    /// Returns what the link did so far.
    pub fn link_stats(&self) -> LinkStats {
        self.shared.lock().link.stats()
    }

    /// Returns the stack, so a test can read its own counters.
    pub fn stack(&self) -> &OwnStack {
        &self.stack
    }

    /// Pumps the stack whenever a held frame falls due.
    async fn tick(self) {
        loop {
            let now = self.shared.clock.now_micros();
            let next = self.shared.lock().link.next_release();
            let wait = next.map_or(50_000, |at| (at - now).clamp(0, 50_000));
            if wait == 0 {
                self.stack.pump();
                tokio::task::yield_now().await;
            } else {
                let sleep = Duration::from_micros(u64::try_from(wait).unwrap_or(0));
                let _ = tokio::time::timeout(sleep, self.shared.wake.notified()).await;
            }
        }
    }
}

impl TcpStack for LossyOwn {
    type Session = OwnSession;

    fn name(&self) -> String {
        OWN_STACK_NAME.to_string()
    }

    fn notes(&self) -> Vec<String> {
        let mut notes: Vec<String> = TcpStack::notes(&self.stack)
            .into_iter()
            .map(|note| {
                if note.starts_with("retransmission is minimal") {
                    note.replace(
                        ", and the link loses nothing, so the measurements never retransmit",
                        "",
                    )
                } else {
                    note
                }
            })
            .collect();
        notes.push(format!(
            "the link is the impaired link of TRN-22 ({}), which wraps the frame queues of the stack; retransmissions are counted at the link as data segments that start below the highest sequence number sent before",
            self.config.describe()
        ));
        notes
    }

    fn listen(&self, port: u16) -> io::Result<()> {
        self.stack.listen(port)
    }

    fn connect(&self, port: u16) -> impl Future<Output = io::Result<OwnSession>> + Send {
        self.stack.connect(port)
    }

    fn accept(&self, port: u16) -> impl Future<Output = io::Result<OwnSession>> + Send {
        self.stack.accept(port)
    }

    fn drive(&self) -> impl Future<Output = ()> + Send {
        let me = self.clone();
        async move {
            let _ticker = AbortOnDrop(tokio::spawn(me.clone().tick()));
            me.stack.drive().await;
        }
    }

    fn poll_once(&self) {
        self.stack.pump();
    }

    fn stats(&self) -> StackStats {
        let mut stats = TcpStack::stats(&self.stack);
        let link = self.link_stats();
        stats.retransmits = link.retransmits;
        stats.link_lost = link.lost;
        stats.link_queue_dropped = link.queue_dropped;
        stats
    }
}
