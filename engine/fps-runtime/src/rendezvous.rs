// SPDX-License-Identifier: GPL-3.0-only
//! Where the sessions that a shared port receives meet the instances that wait
//! for them (ENG-26).
//!
//! Two concurrent instances of a worker can draw the same data port, because
//! the draw is a `rand_int` of the scenario, and both sides of a flow draw it
//! from the same generator. One listener or one socket per port then serves
//! both instances, and the port has to hand each incoming session to the
//! instance that owns it. An instance owns the sessions of the host that its
//! entry session came from, because every flow side of an instance dials from
//! the source address of the instance. The rendezvous therefore matches an
//! incoming session to the oldest waiter that expects its source address, or
//! to the oldest waiter without an expectation.
//!
//! Two instances that share a source address cannot be told apart by the
//! rendezvous, and the older waiter takes the first session. Source pools
//! that are larger than the number of concurrent sessions avoid this.
//!
//! A session that arrives before its waiter registers, which happens when the
//! peer dials before the instance binds, waits for [`PENDING_GRACE`].

use std::collections::VecDeque;
use std::net::IpAddr;
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::instance::OPEN_GRACE;

/// How long a session waits for a waiter. It equals the grace of an open, so
/// a peer that dials early meets an instance that retries its bind for as
/// long.
pub(crate) const PENDING_GRACE: Duration = OPEN_GRACE;

struct Waiter<T> {
    id: u64,
    expect: Option<IpAddr>,
    tx: oneshot::Sender<T>,
}

struct Pending<T> {
    from: IpAddr,
    item: T,
    deadline: Instant,
}

/// The waiters and the early sessions of one port.
pub(crate) struct Rendezvous<T> {
    waiters: Vec<Waiter<T>>,
    pending: VecDeque<Pending<T>>,
    next_id: u64,
}

fn matches(expect: Option<IpAddr>, from: IpAddr) -> bool {
    expect.is_none_or(|e| e == from)
}

impl<T> Rendezvous<T> {
    pub(crate) fn new() -> Self {
        Self {
            waiters: Vec::new(),
            pending: VecDeque::new(),
            next_id: 0,
        }
    }

    /// Hands `item`, which came from `from`, to the oldest waiter that expects
    /// it, or holds it until one registers.
    pub(crate) fn offer(&mut self, from: IpAddr, mut item: T, now: Instant) {
        while let Some(i) = self.waiters.iter().position(|w| matches(w.expect, from)) {
            let waiter = self.waiters.remove(i);
            match waiter.tx.send(item) {
                Ok(()) => return,
                // The waiter gave up, so the next one may take the item.
                Err(back) => item = back,
            }
        }
        self.pending.push_back(Pending {
            from,
            item,
            deadline: now + PENDING_GRACE,
        });
    }

    /// Registers a waiter for a session from `expect`, or from any source
    /// when `expect` is `None`. The receiver completes with the session, which
    /// may already be waiting.
    pub(crate) fn register(&mut self, expect: Option<IpAddr>) -> (u64, oneshot::Receiver<T>) {
        let (tx, rx) = oneshot::channel();
        let id = self.next_id;
        self.next_id += 1;
        let early = self
            .pending
            .iter()
            .position(|p| matches(expect, p.from))
            .and_then(|i| self.pending.remove(i));
        match early {
            Some(p) => {
                // The receiver is still here, so the send cannot fail.
                let _ = tx.send(p.item);
            }
            None => self.waiters.push(Waiter { id, expect, tx }),
        }
        (id, rx)
    }

    /// Removes a waiter that gave up, or that was served.
    pub(crate) fn cancel(&mut self, id: u64) {
        self.waiters.retain(|w| w.id != id);
    }

    /// Drops the sessions that waited longer than [`PENDING_GRACE`].
    pub(crate) fn expire(&mut self, now: Instant) {
        self.pending.retain(|p| p.deadline > now);
    }

    /// Returns when the oldest early session expires.
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.pending.front().map(|p| p.deadline)
    }

    /// Returns true when nobody waits and nothing waits for anybody.
    pub(crate) fn is_idle(&self) -> bool {
        self.waiters.is_empty() && self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(last: u8) -> IpAddr {
        IpAddr::from([127, 0, 0, last])
    }

    #[test]
    fn an_offer_goes_to_the_waiter_that_expects_its_source() {
        let now = Instant::now();
        let mut rv = Rendezvous::new();
        let (_, mut first) = rv.register(Some(ip(1)));
        let (_, mut second) = rv.register(Some(ip(2)));
        rv.offer(ip(2), "b", now);
        assert_eq!(second.try_recv(), Ok("b"));
        assert!(first.try_recv().is_err());
        rv.offer(ip(1), "a", now);
        assert_eq!(first.try_recv(), Ok("a"));
        assert!(rv.is_idle());
    }

    #[test]
    fn an_early_session_waits_for_its_waiter() {
        let now = Instant::now();
        let mut rv = Rendezvous::new();
        let (_, mut other) = rv.register(Some(ip(1)));
        rv.offer(ip(2), "b", now);
        assert!(!rv.is_idle());
        assert!(other.try_recv().is_err(), "a session of another source");
        let (_, mut mine) = rv.register(Some(ip(2)));
        assert_eq!(mine.try_recv(), Ok("b"));
    }

    #[test]
    fn a_waiter_without_expectation_takes_any_source_in_order() {
        let now = Instant::now();
        let mut rv = Rendezvous::new();
        let (_, mut a) = rv.register(None);
        let (_, mut b) = rv.register(None);
        rv.offer(ip(5), 1, now);
        rv.offer(ip(5), 2, now);
        assert_eq!((a.try_recv(), b.try_recv()), (Ok(1), Ok(2)));
    }

    #[test]
    fn a_cancelled_waiter_is_skipped_and_an_old_session_expires() {
        let now = Instant::now();
        let mut rv = Rendezvous::new();
        let (id, rx) = rv.register(None);
        drop(rx);
        rv.offer(ip(1), "x", now);
        assert!(!rv.is_idle(), "the session waits, the waiter had gone");
        rv.cancel(id);
        assert_eq!(rv.next_deadline(), Some(now + PENDING_GRACE));
        rv.expire(now + PENDING_GRACE);
        assert!(rv.is_idle());
    }
}
