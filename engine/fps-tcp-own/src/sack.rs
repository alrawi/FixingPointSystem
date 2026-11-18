// SPDX-License-Identifier: GPL-3.0-only
//! The sender side of selective acknowledgment (TRN-24): the scoreboard of
//! RFC 6675 over byte ranges.
//!
//! The scoreboard holds the ranges above `snd_una` that the receiver reported
//! in SACK blocks, and the ranges that the sender sent again during the
//! recovery. From them it derives what RFC 6675 section 4 calls `IsLost`,
//! `pipe`, and `NextSeg`. A range is lost when more than two segments of
//! bytes, or three separate ranges, were SACKed above it.
//!
//! The scoreboard lives in the [`crate::recovery::Recovery`] box of a
//! connection, so an idle connection holds none. All ranges are absolute
//! sequence numbers that lie within 2^31 of `snd_una`, and every method takes
//! the current `snd_una`, so the arithmetic wraps correctly.

use super::host::{seq_le, seq_lt};

/// The number of SACKed segments above a range that makes it lost
/// (`DupThresh` of RFC 6675).
pub const DUP_THRESH: u32 = 3;

/// A set of disjoint byte ranges `[start, end)`, sorted by sequence, with
/// touching ranges merged.
type Ranges = Vec<(u32, u32)>;

/// Adds `[s, e)` to `v`, whose ranges are all at or above `una`.
fn insert_range(v: &mut Ranges, una: u32, s: u32, e: u32) {
    let (mut a, mut b) = (s.wrapping_sub(una), e.wrapping_sub(una));
    let mut out = Vec::with_capacity(v.len() + 1);
    let mut placed = false;
    for &(rs, re) in v.iter() {
        let (c, d) = (rs.wrapping_sub(una), re.wrapping_sub(una));
        if d < a {
            out.push((rs, re));
        } else if c > b {
            if !placed {
                out.push((una.wrapping_add(a), una.wrapping_add(b)));
                placed = true;
            }
            out.push((rs, re));
        } else {
            a = a.min(c);
            b = b.max(d);
        }
    }
    if !placed {
        out.push((una.wrapping_add(a), una.wrapping_add(b)));
    }
    *v = out;
}

/// Drops what lies below `una` from `v`.
fn trim_ranges(v: &mut Ranges, una: u32) {
    v.retain_mut(|(s, e)| {
        if seq_le(*e, una) {
            return false;
        }
        if seq_lt(*s, una) {
            *s = una;
        }
        true
    });
}

fn total(v: &Ranges) -> u32 {
    v.iter().map(|(s, e)| e.wrapping_sub(*s)).sum()
}

/// The ranges that the receiver holds and the ranges that the sender sent
/// again, above `snd_una`.
#[derive(Debug, Default)]
pub struct Scoreboard {
    sacked: Ranges,
    /// The ranges that were sent again, with the highest sequence number
    /// that the sender had sent when it did so.
    rexmit: Vec<(u32, u32, u32)>,
    /// The end of the data that RACK (RFC 8985) calls lost, by time: a hole
    /// that starts below it is lost whatever lies above it (TRN-34).
    rack_lost: Option<u32>,
}

impl Scoreboard {
    /// True when no range is held, so the box could go.
    pub fn is_empty(&self) -> bool {
        self.sacked.is_empty() && self.rexmit.is_empty()
    }

    /// The number of SACKed ranges.
    pub fn ranges(&self) -> usize {
        self.sacked.len()
    }

    /// The SACKed bytes above `snd_una`.
    pub fn sacked_bytes(&self) -> u32 {
        total(&self.sacked)
    }

    /// Forgets everything: after a timeout, where the receiver may have
    /// dropped what it reported, and at the end of a recovery.
    pub fn clear(&mut self) {
        self.sacked.clear();
        self.rexmit.clear();
        self.rack_lost = None;
    }

    /// Sets the end of the data that RACK calls lost, or none.
    pub fn set_rack_lost(&mut self, to: Option<u32>) {
        self.rack_lost = to;
    }

    /// The end of the data that RACK calls lost.
    pub fn rack_lost(&self) -> Option<u32> {
        self.rack_lost
    }

    /// The highest SACKed byte plus one, if any block is held.
    pub fn highest_sacked(&self) -> Option<u32> {
        self.sacked.last().map(|&(_, e)| e)
    }

    /// Whether the byte at `seq` was sent again.
    pub fn is_rexmit(&self, seq: u32) -> bool {
        self.rexmit
            .iter()
            .any(|&(s, e, _)| seq_le(s, seq) && seq_lt(seq, e))
    }

    /// Forgets the sent again ranges, which a new recovery starts without.
    pub fn clear_rexmit(&mut self) {
        self.rexmit.clear();
    }

    /// Drops what `snd_una` passed.
    pub fn advance(&mut self, snd_una: u32) {
        if self.rack_lost.is_some_and(|to| seq_le(to, snd_una)) {
            self.rack_lost = None;
        }
        trim_ranges(&mut self.sacked, snd_una);
        self.rexmit.retain_mut(|(s, e, _)| {
            if seq_le(*e, snd_una) {
                return false;
            }
            if seq_lt(*s, snd_una) {
                *s = snd_una;
            }
            true
        });
    }

    /// Takes the SACK blocks of one ACK. A block counts only when it lies
    /// above `snd_una` and no further than `snd_max`; a duplicate block or
    /// one below `snd_una` (RFC 2883) is ignored. Returns true when the
    /// blocks told something new.
    pub fn update(&mut self, snd_una: u32, snd_max: u32, blocks: &[(u32, u32)]) -> bool {
        let before = self.sacked_bytes();
        for &(s, e) in blocks {
            if !seq_lt(snd_una, s) || !seq_lt(s, e) || seq_lt(snd_max, e) {
                continue;
            }
            insert_range(&mut self.sacked, snd_una, s, e);
        }
        self.sacked_bytes() > before
    }

    /// Calls `f(start, end, sacked_above, ranges_above)` for every hole
    /// between `snd_una` and the highest SACKed byte, lowest first, until `f`
    /// returns true. Returns whether `f` did.
    fn each_hole(&self, snd_una: u32, mut f: impl FnMut(u32, u32, u32, u32) -> bool) -> bool {
        let mut above = self.sacked_bytes();
        let mut count = self.sacked.len() as u32;
        let mut cursor = snd_una;
        for &(s, e) in &self.sacked {
            if seq_lt(cursor, s) && f(cursor, s, above, count) {
                return true;
            }
            above -= e.wrapping_sub(s);
            count -= 1;
            cursor = e;
        }
        false
    }

    /// RFC 6675 `IsLost` for a hole with `above` SACKed bytes in `ranges`
    /// ranges above it.
    fn lost(above: u32, ranges: u32, mss: u32) -> bool {
        ranges >= DUP_THRESH || above > (DUP_THRESH - 1) * mss
    }

    /// `IsLost` for a hole that starts at `start`, with RACK: the count
    /// rule, or a time that RACK found to be over for the segment.
    fn hole_lost(&self, start: u32, above: u32, ranges: u32, mss: u32) -> bool {
        Self::lost(above, ranges, mss) || self.rack_lost.is_some_and(|to| seq_lt(start, to))
    }

    /// Whether the first unacknowledged byte is presumed lost, which starts a
    /// recovery without waiting for the third duplicate acknowledgment.
    pub fn lost_at_una(&self, snd_una: u32, mss: u32) -> bool {
        self.sacked
            .first()
            .is_some_and(|&(s, _)| seq_lt(snd_una, s))
            && (Self::lost(self.sacked_bytes(), self.sacked.len() as u32, mss)
                || self.rack_lost.is_some_and(|to| seq_lt(snd_una, to)))
    }

    /// Records that `[start, start + len)` was sent again.
    pub fn mark_rexmit(&mut self, snd_una: u32, start: u32, len: u32) {
        self.mark_rexmit_at(snd_una, start, len, start.wrapping_add(len));
    }

    /// Notes that `len` bytes at `start` were sent again while `snd_max` was
    /// the highest sequence number sent, so that
    /// [`Scoreboard::detect_lost_rexmit`] can tell when the copy was lost.
    pub fn mark_rexmit_at(&mut self, snd_una: u32, start: u32, len: u32, snd_max: u32) {
        if len > 0 && !seq_lt(start, snd_una) {
            let end = start.wrapping_add(len);
            // A range that is already marked stays one entry, so the pipe
            // never counts a byte twice.
            self.rexmit
                .retain(|&(rs, re, _)| seq_le(re, start) || seq_le(end, rs));
            self.rexmit.push((start, end, snd_max));
            // The walks of `next_hole` rely on the order of the sequence.
            self.rexmit
                .sort_by_key(|&(rs, _, _)| rs.wrapping_sub(snd_una));
        }
    }

    /// Forgets the sent again ranges whose copy the receiver cannot have
    /// held: data sent after the copy, by more than two segments, has been
    /// selectively acknowledged, so the copy was lost. The range is then a
    /// hole that [`Scoreboard::next_hole`] offers again, and the pipe stops
    /// counting it. This is the detection of a lost retransmission that the
    /// kernel makes (`tcp_mark_lost_retrans`), which RFC 6675 leaves to the
    /// retransmission timer. Returns the number of ranges that were lost.
    pub fn detect_lost_rexmit(&mut self, mss: u32) -> u32 {
        let Some(&(_, high)) = self.sacked.last() else {
            return 0;
        };
        let margin = (DUP_THRESH - 1) * mss;
        let before = self.rexmit.len();
        self.rexmit
            .retain(|&(_, _, at)| !seq_lt(at.wrapping_add(margin), high));
        (before - self.rexmit.len()) as u32
    }

    /// RFC 6675 `pipe`: the bytes that the sender believes are in the
    /// network. Bytes above the highest SACKed byte and bytes of holes that
    /// are not lost count once, bytes of lost holes count only when sent
    /// again, and the copies sent again count.
    ///
    /// With `una_lost` the hole at `snd_una` counts as lost whatever lies
    /// above it. A recovery sets it, because the acknowledgment that
    /// brought `snd_una` to this hole was a partial one, which NewReno reads
    /// as a lost segment (RFC 6582), and a tail with fewer than three
    /// segments above its last hole would otherwise wait for the timeout.
    pub fn pipe(&self, snd_una: u32, snd_max: u32, mss: u32, una_lost: bool) -> u32 {
        let mut pipe = 0u32;
        self.each_hole(snd_una, |s, e, above, ranges| {
            if !(self.hole_lost(s, above, ranges, mss) || (una_lost && s == snd_una)) {
                pipe += e.wrapping_sub(s);
            }
            pipe += self.rexmit_in(s, e);
            false
        });
        let tail = self.sacked.last().map_or(snd_una, |&(_, e)| e);
        if seq_lt(tail, snd_max) {
            pipe += snd_max.wrapping_sub(tail);
            // Sent again data above the highest SACKed byte.
            pipe += self.rexmit_in(tail, snd_max);
        }
        pipe
    }

    /// The bytes of `[s, e)` that were sent again.
    fn rexmit_in(&self, s: u32, e: u32) -> u32 {
        let mut n = 0u32;
        for &(rs, re, _) in &self.rexmit {
            let lo = if seq_lt(rs, s) { s } else { rs };
            let hi = if seq_lt(e, re) { e } else { re };
            if seq_lt(lo, hi) {
                n += hi.wrapping_sub(lo);
            }
        }
        n
    }

    /// RFC 6675 `NextSeg` rule 1: the lowest range of a lost hole that was
    /// not sent again yet, as the start and at most `mss` bytes.
    pub fn next_hole(&self, snd_una: u32, mss: u32, una_lost: bool) -> Option<(u32, u32)> {
        let mut found = None;
        self.each_hole(snd_una, |s, e, above, ranges| {
            if !(self.hole_lost(s, above, ranges, mss) || (una_lost && s == snd_una)) {
                return false;
            }
            let mut cursor = s;
            for &(rs, re, _) in &self.rexmit {
                if seq_le(rs, cursor) && seq_lt(cursor, re) {
                    cursor = re;
                }
            }
            if seq_lt(cursor, e) {
                // The next sent again range above the cursor bounds the piece.
                let mut end = e;
                for &(rs, _, _) in &self.rexmit {
                    if seq_lt(cursor, rs) && seq_lt(rs, end) {
                        end = rs;
                    }
                }
                found = Some((cursor, end.wrapping_sub(cursor).min(mss)));
                return true;
            }
            false
        });
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: u32 = 1000;

    fn board(una: u32, blocks: &[(u32, u32)]) -> Scoreboard {
        let mut sb = Scoreboard::default();
        sb.update(una, una.wrapping_add(100 * M), blocks);
        sb
    }

    #[test]
    fn blocks_are_kept_sorted_and_touching_blocks_merge() {
        let mut sb = Scoreboard::default();
        let una = 10_000;
        let max = una + 50 * M;
        assert!(sb.update(una, max, &[(una + 5 * M, una + 6 * M)]));
        assert!(sb.update(una + 2 * M, max, &[(una + 2 * M + 1, una + 3 * M)]));
        assert!(sb.update(una, max, &[(una + 6 * M, una + 7 * M)]));
        assert_eq!(sb.ranges(), 2);
        assert_eq!(sb.sacked_bytes(), 2 * M + (M - 1));
        // A block that bridges both ranges leaves one.
        assert!(sb.update(una, max, &[(una + 3 * M, una + 5 * M)]));
        assert_eq!(sb.ranges(), 1);
        assert_eq!(sb.sacked_bytes(), 5 * M - 1);
    }

    #[test]
    fn a_repeated_block_tells_nothing_new() {
        let mut sb = Scoreboard::default();
        assert!(sb.update(0, 10_000, &[(2000, 3000)]));
        assert!(!sb.update(0, 10_000, &[(2000, 3000)]));
        assert!(!sb.update(0, 10_000, &[(2200, 2800)]));
    }

    #[test]
    fn blocks_below_snd_una_or_beyond_snd_max_are_ignored() {
        let mut sb = Scoreboard::default();
        // A block at or below snd_una is a D-SACK of RFC 2883, a block
        // beyond snd_max is wrong, an empty or reversed block is noise.
        assert!(!sb.update(5000, 9000, &[(4000, 4500), (5000, 5500), (8000, 9500)]));
        assert!(!sb.update(5000, 9000, &[(6000, 6000), (7000, 6500)]));
        assert!(sb.is_empty());
    }

    #[test]
    fn advancing_snd_una_trims_and_drops_ranges() {
        let mut sb = board(0, &[(2000, 4000), (6000, 7000)]);
        sb.mark_rexmit(0, 0, 1000);
        sb.advance(3000);
        assert_eq!(sb.sacked_bytes(), 2000);
        sb.advance(6500);
        assert_eq!(sb.sacked_bytes(), 500);
        sb.advance(7000);
        assert!(sb.is_empty());
    }

    #[test]
    fn a_hole_is_lost_after_three_ranges_or_more_than_two_segments_above() {
        // Two ranges of one segment each above the hole: not lost yet.
        let sb = board(0, &[(2 * M, 3 * M), (4 * M, 5 * M)]);
        assert!(!sb.lost_at_una(0, M));
        // A third separate range.
        let sb = board(0, &[(2 * M, 3 * M), (4 * M, 5 * M), (6 * M, 7 * M)]);
        assert!(sb.lost_at_una(0, M));
        // One range of more than two segments.
        let sb = board(0, &[(2 * M, 4 * M + 1)]);
        assert!(sb.lost_at_una(0, M));
        let sb = board(0, &[(2 * M, 4 * M)]);
        assert!(!sb.lost_at_una(0, M));
        // Nothing is lost when snd_una itself was SACKed away from the hole.
        assert!(!Scoreboard::default().lost_at_una(0, M));
    }

    #[test]
    fn the_pipe_counts_what_is_not_lost_and_what_was_sent_again() {
        // Flight of 12 segments, segments 0 and 4 missing, 1-3 and 5-7 held,
        // 8-11 not yet reported (above the highest SACK).
        let mut sb = board(0, &[(M, 4 * M), (5 * M, 8 * M)]);
        // Hole 0 has 6 segments above in 2 ranges: lost (above > 2 MSS).
        // Hole 1 (segment 4) has 3 above: lost. Tail: 4 segments in flight.
        assert_eq!(sb.pipe(0, 12 * M, M, false), 4 * M);
        sb.mark_rexmit(0, 0, M);
        assert_eq!(sb.pipe(0, 12 * M, M, false), 5 * M);
        sb.mark_rexmit(0, 4 * M, M);
        assert_eq!(sb.pipe(0, 12 * M, M, false), 6 * M);
        // The copy of segment 0 arrives and the cumulative ACK moves to the
        // first SACKed byte: that copy no longer counts.
        sb.advance(M);
        assert_eq!(sb.pipe(M, 12 * M, M, false), 5 * M);
    }

    #[test]
    fn a_hole_that_is_not_lost_counts_in_the_pipe() {
        let sb = board(0, &[(M, 2 * M)]);
        // The hole of segment 0 has one segment above: not lost, in flight.
        assert_eq!(sb.pipe(0, 5 * M, M, false), M + 3 * M);
        assert_eq!(sb.next_hole(0, M, false), None);
    }

    #[test]
    fn next_hole_walks_the_lost_holes_in_order_in_pieces_of_one_segment() {
        let mut sb = board(0, &[(3 * M, 6 * M), (7 * M, 10 * M)]);
        // Hole 0 is 3 segments, hole 1 is one segment, both lost.
        assert_eq!(sb.next_hole(0, M, false), Some((0, M)));
        sb.mark_rexmit(0, 0, M);
        assert_eq!(sb.next_hole(0, M, false), Some((M, M)));
        sb.mark_rexmit(0, M, M);
        sb.mark_rexmit(0, 2 * M, M);
        assert_eq!(sb.next_hole(0, M, false), Some((6 * M, M)));
        sb.mark_rexmit(0, 6 * M, M);
        assert_eq!(sb.next_hole(0, M, false), None);
    }

    #[test]
    fn a_piece_is_cut_at_a_range_that_was_sent_again() {
        let mut sb = board(0, &[(5 * M, 9 * M)]);
        sb.mark_rexmit(0, 500, 200);
        // [0, 500) first, then [700, ...).
        assert_eq!(sb.next_hole(0, M, false), Some((0, 500)));
        sb.mark_rexmit(0, 0, 500);
        assert_eq!(sb.next_hole(0, M, false), Some((700, M)));
    }

    #[test]
    fn the_scoreboard_works_across_the_sequence_wrap() {
        let una = u32::MAX - 2 * M;
        let mut sb = Scoreboard::default();
        let max = una.wrapping_add(12 * M);
        sb.update(
            una,
            max,
            &[
                (una.wrapping_add(M), una.wrapping_add(4 * M)),
                (una.wrapping_add(5 * M), una.wrapping_add(8 * M)),
            ],
        );
        assert_eq!(sb.sacked_bytes(), 6 * M);
        assert_eq!(sb.pipe(una, max, M, false), 4 * M);
        assert_eq!(sb.next_hole(una, M, false), Some((una, M)));
        sb.advance(una.wrapping_add(4 * M));
        assert_eq!(sb.sacked_bytes(), 3 * M);
    }

    #[test]
    fn a_recovery_counts_the_hole_at_snd_una_as_lost() {
        // Two segments above the hole are not enough by themselves.
        let mut sb = board(0, &[(M, 3 * M)]);
        assert_eq!(sb.next_hole(0, M, false), None);
        assert_eq!(sb.pipe(0, 4 * M, M, false), M + M);
        // After a partial acknowledgment the first hole is the next loss.
        assert_eq!(sb.next_hole(0, M, true), Some((0, M)));
        assert_eq!(sb.pipe(0, 4 * M, M, true), M);
        sb.mark_rexmit(0, 0, M);
        assert_eq!(sb.next_hole(0, M, true), None);
        assert_eq!(sb.pipe(0, 4 * M, M, true), 2 * M);
    }

    #[test]
    fn clear_forgets_everything() {
        let mut sb = board(0, &[(M, 5 * M)]);
        sb.mark_rexmit(0, 0, M);
        sb.clear();
        assert!(sb.is_empty());
        assert_eq!(sb.pipe(0, 8 * M, M, false), 8 * M);
    }

    #[test]
    fn a_copy_is_lost_when_data_sent_after_it_was_selectively_acknowledged() {
        // The first segment was sent again while 20 segments were out. The
        // copy went to the tail of a full queue and was dropped.
        let mut sb = board(0, &[(5 * M, 9 * M)]);
        sb.mark_rexmit_at(0, 0, M, 20 * M);
        assert_eq!(sb.pipe(0, 30 * M, M, true), 22 * M);
        assert_eq!(sb.next_hole(0, M, true), Some((M, M)));
        // Data below the mark says nothing about the copy.
        assert_eq!(sb.detect_lost_rexmit(M), 0);
        // The data that came after the copy is acknowledged, but by two
        // segments only, which reordering explains.
        sb.update(0, 30 * M, &[(20 * M, 22 * M)]);
        assert_eq!(sb.detect_lost_rexmit(M), 0);
        // A third segment beyond the mark: the copy is lost. The first
        // segment is a hole again and stops counting in the pipe.
        sb.update(0, 30 * M, &[(20 * M, 23 * M)]);
        assert_eq!(sb.detect_lost_rexmit(M), 1);
        assert_eq!(sb.next_hole(0, M, true), Some((0, M)));
        assert_eq!(sb.pipe(0, 30 * M, M, true), 7 * M);
    }

    #[test]
    fn a_copy_that_was_delivered_is_not_counted_twice() {
        let mut sb = board(0, &[(5 * M, 9 * M)]);
        sb.mark_rexmit_at(0, 0, M, 20 * M);
        // The same range sent again once more replaces the entry.
        sb.mark_rexmit_at(0, 0, M, 25 * M);
        assert_eq!(sb.pipe(0, 30 * M, M, true), 22 * M);
        sb.advance(M);
        assert_eq!(sb.detect_lost_rexmit(M), 0);
    }
}
