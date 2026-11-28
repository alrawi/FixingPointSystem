// SPDX-License-Identifier: GPL-3.0-only
//! A hashed timer wheel with a tick of one millisecond.
//!
//! An entry names a flow and a generation. Cancelling is lazy: the owner
//! changes the generation of the flow, and a due entry with an old generation
//! is dropped. A deadline further away than one turn of the wheel keeps a
//! round count.

/// Slots of the wheel, so one turn is 1,024 ms.
pub const SLOTS: usize = 1024;

/// One armed timer.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// The slot of the flow in the flow table.
    pub flow: u32,
    /// The generation that the flow had when it armed the timer.
    pub gen: u32,
    rounds: u32,
}

/// The wheel.
pub struct Wheel {
    slots: Vec<Vec<Entry>>,
    cur: u64,
    len: usize,
}

impl Default for Wheel {
    fn default() -> Self {
        Self::new()
    }
}

impl Wheel {
    /// An empty wheel at tick 0.
    pub fn new() -> Self {
        Self {
            slots: vec![Vec::new(); SLOTS],
            cur: 0,
            len: 0,
        }
    }

    /// The tick that the wheel has reached.
    pub fn cur(&self) -> u64 {
        self.cur
    }

    /// Entries in the wheel, stale ones included.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the wheel holds no entry.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Arms an entry for `tick`. A tick that is not after the current tick
    /// fires at the next one.
    pub fn insert(&mut self, flow: u32, gen: u32, tick: u64) {
        let delta = tick.saturating_sub(self.cur).max(1);
        let slot = ((self.cur + delta) % SLOTS as u64) as usize;
        let rounds = ((delta - 1) / SLOTS as u64) as u32;
        self.slots[slot].push(Entry { flow, gen, rounds });
        self.len += 1;
    }

    /// Moves the wheel by one tick and appends the entries that fall due to
    /// `due`.
    pub fn step(&mut self, due: &mut Vec<Entry>) {
        self.cur += 1;
        let slot = &mut self.slots[(self.cur % SLOTS as u64) as usize];
        let mut i = 0;
        while i < slot.len() {
            if slot[i].rounds > 0 {
                slot[i].rounds -= 1;
                i += 1;
            } else {
                due.push(slot.swap_remove(i));
                self.len -= 1;
            }
        }
    }
}
