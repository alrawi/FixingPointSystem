// SPDX-License-Identifier: GPL-3.0-only
//! The leased addresses of a side, shared between the thread that runs the
//! DHCP clients and everything that reads an address (TRN-37).
//!
//! A static pool maps a host index to an address by arithmetic. A pool that
//! leases its addresses (`HostPool::dhcp`, HST-04) has only what the servers
//! gave, and that changes while the run goes on. A [`LeaseBoard`] is the one
//! place where the lease table of a slice is published: the DHCP hook of the
//! datapath [publishes](LeaseBoard::publish) its [`LeaseTable`], and the
//! stations of the stack, the ARP responder of the splitter, and the route of
//! the arm read it. Every reader compares [`LeaseBoard::generation`] with the
//! number it saw last and refreshes only when it moved, so a renewal for the
//! same address costs nothing.
//!
//! The board also builds the [`Snapshot`] that the readers need: the lease
//! table and a [`Responder`] over a [`NeighborTable`] of the leased hosts
//! ([`NeighborTable::add_leased`]). It builds it lazily, once per generation,
//! when a reader asks, so a start of 20,000 hosts that publishes a thousand
//! times costs one build per reader pass and not one per lease.
//!
//! The own stack keeps its [`crate::HostSlice`] as plain data, because the
//! agent builds those by hand. The lease table therefore travels next to the
//! slice, in [`Leases`].

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use fps_hosts::{HostPool, HostRange, LeaseTable, MacBlock, NeighborTable, Responder};

/// Why a board could not be made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BoardError {
    /// The pool has static addresses, so it has no leases.
    #[error("host pool {0} has static addresses and no leases")]
    NotLeased(String),
    /// The MAC block is for another pool.
    #[error("host pool {pool}: the MAC block is for pool {block_pool}")]
    BlockMismatch {
        /// The pool.
        pool: String,
        /// The pool of the block.
        block_pool: String,
    },
}

/// The leases at one generation, with the responder that answers for them.
#[derive(Debug, Clone)]
pub struct Snapshot {
    leases: LeaseTable,
    responder: Responder,
    /// Why the table of the responder is empty, when the leases could not
    /// make one, for example because two hosts leased the same address.
    table_error: Option<String>,
}

impl Snapshot {
    /// The leases of the generation.
    #[must_use]
    pub fn leases(&self) -> &LeaseTable {
        &self.leases
    }

    /// The generation of the lease table.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.leases.generation()
    }

    /// The responder for the leased hosts: ARP requests for their addresses
    /// get an answer with the MAC address of the host.
    #[must_use]
    pub fn responder(&self) -> &Responder {
        &self.responder
    }

    /// The neighbor table of the responder.
    #[must_use]
    pub fn table(&self) -> &NeighborTable {
        self.responder.table()
    }

    /// The address of host `index` of the pool (an absolute index).
    #[must_use]
    pub fn addr_of(&self, index: u32) -> Option<Ipv4Addr> {
        self.leases.addr_of(index)
    }

    /// Why the neighbor table is empty although hosts hold leases, or `None`.
    #[must_use]
    pub fn table_error(&self) -> Option<&str> {
        self.table_error.as_deref()
    }
}

#[derive(Debug)]
struct State {
    leases: LeaseTable,
    snapshot: Option<Arc<Snapshot>>,
}

/// The lease table of one slice, published by the DHCP clients and read by
/// the stack, the responder, and the route.
#[derive(Debug)]
pub struct LeaseBoard {
    pool: HostPool,
    block: MacBlock,
    state: Mutex<State>,
    generation: AtomicU64,
    bound: AtomicUsize,
}

impl LeaseBoard {
    /// A board for the hosts that `block` covers in `pool`, with no lease yet.
    ///
    /// # Errors
    ///
    /// Returns [`BoardError::NotLeased`] for a pool with static addresses and
    /// [`BoardError::BlockMismatch`] for the block of another pool.
    pub fn new(pool: HostPool, block: MacBlock) -> Result<Self, BoardError> {
        if !pool.is_dhcp() {
            return Err(BoardError::NotLeased(pool.name().to_owned()));
        }
        if block.pool() != pool.name() {
            return Err(BoardError::BlockMismatch {
                pool: pool.name().to_owned(),
                block_pool: block.pool().to_owned(),
            });
        }
        let leases = LeaseTable::new(block.range());
        Ok(Self {
            pool,
            block,
            generation: AtomicU64::new(leases.generation()),
            bound: AtomicUsize::new(0),
            state: Mutex::new(State {
                leases,
                snapshot: None,
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The pool.
    #[must_use]
    pub fn pool(&self) -> &HostPool {
        &self.pool
    }

    /// The MAC addresses of the hosts of the slice.
    #[must_use]
    pub fn block(&self) -> &MacBlock {
        &self.block
    }

    /// The hosts of the slice, as indexes of the pool.
    #[must_use]
    pub fn range(&self) -> HostRange {
        self.block.range()
    }

    /// The number of hosts of the slice, bound or not.
    #[must_use]
    pub fn count(&self) -> usize {
        usize::try_from(self.block.range().count).unwrap_or(usize::MAX)
    }

    /// The number of hosts that hold an address now.
    #[must_use]
    pub fn bound(&self) -> usize {
        self.bound.load(Ordering::Acquire)
    }

    /// Whether every host of the slice holds an address.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.bound() == self.count()
    }

    /// The generation of the published table. It moves when an address is
    /// added, replaced, or lost, and stays when a lease is renewed for the same
    /// address. A reader that saw this number has the current addresses.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Publishes `leases`, the table of the clients of this slice. It returns
    /// whether the readers have something new to read, which is when the
    /// address set or the number of bound hosts differs from the table that
    /// the board holds. A table for another range is refused and returns
    /// `false`.
    pub fn publish(&self, leases: &LeaseTable) -> bool {
        if leases.range() != self.block.range() {
            return false;
        }
        let mut state = self.lock();
        if leases.generation() == state.leases.generation() && leases.bound() == state.leases.bound()
        {
            return false;
        }
        state.leases = leases.clone();
        state.snapshot = None;
        self.bound.store(leases.bound(), Ordering::Release);
        self.generation
            .store(leases.generation(), Ordering::Release);
        true
    }

    /// The address of host `index` (an absolute index of the pool), without
    /// building a snapshot.
    #[must_use]
    pub fn addr_of(&self, index: u32) -> Option<Ipv4Addr> {
        self.lock().leases.addr_of(index)
    }

    /// The host that holds `addr`, as an absolute index of the pool.
    #[must_use]
    pub fn index_of(&self, addr: Ipv4Addr) -> Option<u32> {
        self.lock().leases.index_of(addr)
    }

    /// The leases and the responder at the current generation. The snapshot
    /// is built when the first reader asks after a change and shared after
    /// that.
    #[must_use]
    pub fn snapshot(&self) -> Arc<Snapshot> {
        let mut state = self.lock();
        if let Some(snapshot) = &state.snapshot {
            return Arc::clone(snapshot);
        }
        let mut table = NeighborTable::new();
        let mut table_error = None;
        if let Err(e) = table.add_leased(&self.pool, &self.block, &state.leases) {
            // Two hosts with one address, say. The responder answers for
            // nobody then, which is safer than a wrong MAC address.
            table = NeighborTable::new();
            table_error = Some(e.to_string());
        }
        let snapshot = Arc::new(Snapshot {
            leases: state.leases.clone(),
            responder: Responder::new(table),
            table_error,
        });
        state.snapshot = Some(Arc::clone(&snapshot));
        snapshot
    }
}

/// The boards of the two sides of a stack. A side whose pool has static
/// addresses has none.
#[derive(Clone, Debug, Default)]
pub struct Leases {
    /// The board of the client side.
    pub client: Option<Arc<LeaseBoard>>,
    /// The board of the server side.
    pub server: Option<Arc<LeaseBoard>>,
}

impl Leases {
    /// Whether neither side leases.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.client.is_none() && self.server.is_none()
    }
}

/// What a reader keeps of a board between two looks: the snapshot it used and
/// the generation of it, so that it refreshes only when the board moved.
#[derive(Debug, Clone)]
pub struct Follower {
    board: Arc<LeaseBoard>,
    seen: std::cell::Cell<u64>,
    snapshot: std::cell::RefCell<Arc<Snapshot>>,
}

impl Follower {
    /// A follower of `board` that starts at its current state.
    #[must_use]
    pub fn new(board: Arc<LeaseBoard>) -> Self {
        let snapshot = board.snapshot();
        Self {
            seen: std::cell::Cell::new(snapshot.generation()),
            snapshot: std::cell::RefCell::new(snapshot),
            board,
        }
    }

    /// The board that the follower reads.
    #[must_use]
    pub fn board(&self) -> &Arc<LeaseBoard> {
        &self.board
    }

    /// The snapshot at the current generation.
    #[must_use]
    pub fn current(&self) -> Arc<Snapshot> {
        if self.board.generation() != self.seen.get() {
            let fresh = self.board.snapshot();
            self.seen.set(fresh.generation());
            *self.snapshot.borrow_mut() = fresh;
        }
        Arc::clone(&self.snapshot.borrow())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use fps_hosts::{DhcpConfig, DhcpHosts, MacAllocator};
    use std::time::Duration;

    fn board(count: u32) -> LeaseBoard {
        let pool = HostPool::dhcp("c", count, Some("02:aa:00:00:00:00")).unwrap();
        let block = MacAllocator::new().claim(&pool, pool.range()).unwrap();
        LeaseBoard::new(pool, block).unwrap()
    }

    #[test]
    fn a_static_pool_has_no_board() {
        let pool = fps_hosts::HostPool::parse(&fps_hosts::PoolSpec {
            name: "s",
            ipv4: Some("10.0.0.0/24"),
            count: 2,
            ..fps_hosts::PoolSpec::default()
        })
        .unwrap();
        let block = MacAllocator::new().claim(&pool, pool.range()).unwrap();
        assert!(matches!(
            LeaseBoard::new(pool, block),
            Err(BoardError::NotLeased(_))
        ));
    }

    #[test]
    fn an_empty_board_has_no_address_and_a_publish_of_nothing_changes_nothing() {
        let b = board(3);
        assert_eq!((b.bound(), b.count(), b.is_complete()), (0, 3, false));
        let hosts = DhcpHosts::new(
            b.pool(),
            b.block(),
            &DhcpConfig::default(),
            Duration::ZERO,
        )
        .unwrap();
        assert!(!b.publish(hosts.leases()), "nothing moved");
        assert!(b.snapshot().addr_of(0).is_none());
    }

    #[test]
    fn a_table_for_another_range_is_refused() {
        let b = board(3);
        let other = LeaseTable::new(HostRange::new(1, 2));
        assert!(!b.publish(&other));
    }
}
