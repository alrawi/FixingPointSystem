// SPDX-License-Identifier: GPL-3.0-only
//! The impaired link of the harness (TRN-22): random loss, delay, reordering,
//! and a rate limit with a bounded queue.
//!
//! [`Impairment`] is the model of one direction of a link. A sender offers a
//! frame with [`Impairment::offer`], and the receiver takes the frames that
//! are due with [`Impairment::pop_due`]. The model has no clock of its own,
//! so a test drives it with fixed times, and the stacks that use it pass the
//! time of their [`crate::Clock`]. The same settings turn into `tc` commands
//! for the kernel stack with [`netem_commands`], so every stack is measured
//! on the same link.
//!
//! The model applies the stages in the order of the kernel setup, which is a
//! `tbf` qdisc (rate and bounded queue) with a `netem` qdisc below it.
//!
//! 1. The bottleneck. With a rate, a frame leaves after the frames before it
//!    have been serialized at that rate. A frame that finds more than
//!    `queue_bytes` waiting is dropped (tail drop).
//! 2. Random loss. A frame is lost with the probability `loss_ppm` in a
//!    million. A lost frame has used its place at the bottleneck.
//! 3. Delay and reordering. A frame takes `delay_us` more, except that with
//!    the probability `reorder_ppm` it skips the delay, so it overtakes the
//!    frames before it. This is what `netem delay D reorder P` does.
//!
//! Each direction of a link has its own model with its own random sequence.
//! The settings are per direction, as `tc qdisc` settings are, so a delay of
//! 20 ms means a round trip of 40 ms.
//!
//! The model also counts retransmissions. It reads the TCP header of every
//! frame that a sender offers, before the loss, and counts a data segment
//! that starts below the highest sequence number that the flow sent before.
//! That is how the harness counts retransmissions for a stack that does not
//! expose a counter, in the same way for every stack.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

/// The probabilities are counted in parts per million.
pub const PPM: u32 = 1_000_000;

/// The settings of one direction of an impaired link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImpairConfig {
    /// The probability that a frame is lost, in parts per million.
    pub loss_ppm: u32,
    /// The delay of every frame in microseconds.
    pub delay_us: i64,
    /// The probability that a frame skips the delay, in parts per million.
    /// It needs a delay.
    pub reorder_ppm: u32,
    /// The rate of the bottleneck in bits per second, or 0 for no limit.
    pub rate_bps: u64,
    /// The bytes that the bottleneck holds while it serializes. A frame that
    /// would exceed it is dropped. It applies only with a rate.
    pub queue_bytes: usize,
    /// The seed of the random sequence. The two directions of a link use
    /// different seeds that derive from this one.
    pub seed: u64,
}

impl Default for ImpairConfig {
    fn default() -> Self {
        Self {
            loss_ppm: 0,
            delay_us: 0,
            reorder_ppm: 0,
            rate_bps: 0,
            queue_bytes: 256 * 1024,
            seed: 0x7A11_0001,
        }
    }
}

impl ImpairConfig {
    /// Returns true when the link neither loses, delays, reorders, nor
    /// limits anything.
    pub fn is_clean(&self) -> bool {
        self.loss_ppm == 0 && self.delay_us == 0 && self.reorder_ppm == 0 && self.rate_bps == 0
    }

    /// Checks the settings.
    ///
    /// # Errors
    ///
    /// Returns a sentence that names the setting that is out of range.
    pub fn validate(&self) -> Result<(), String> {
        if self.loss_ppm > PPM || self.reorder_ppm > PPM {
            return Err("a probability is above 100 percent".into());
        }
        if self.delay_us < 0 {
            return Err("the delay is negative".into());
        }
        if self.reorder_ppm > 0 && self.delay_us == 0 {
            return Err(
                "reordering needs a delay, because a frame overtakes by skipping it".into(),
            );
        }
        if self.rate_bps > 0 && self.queue_bytes < 3_000 {
            return Err("the bottleneck queue is smaller than two frames".into());
        }
        Ok(())
    }

    /// Returns the settings of the second direction of a link.
    #[must_use]
    pub fn reverse(&self) -> Self {
        Self {
            seed: self.seed ^ 0x9E37_79B9_7F4A_7C15,
            ..*self
        }
    }

    /// Describes the settings in one line, for the reports.
    pub fn describe(&self) -> String {
        let mut text = format!(
            "loss {}%, delay {} ms, reorder {}%",
            percent_text(self.loss_ppm),
            millis_text(self.delay_us),
            percent_text(self.reorder_ppm)
        );
        if self.rate_bps > 0 {
            text.push_str(&format!(
                ", rate {} Mbit/s with a queue of {} KiB",
                self.rate_bps as f64 / 1e6,
                self.queue_bytes / 1024
            ));
        } else {
            text.push_str(", no rate limit");
        }
        text.push_str(" per direction");
        text
    }
}

/// Converts a percentage such as `0.1` to parts per million.
///
/// # Errors
///
/// Returns an error text when `value` is not a number from 0 to 100.
pub fn percent_to_ppm(value: &str) -> Result<u32, String> {
    let percent: f64 = value
        .trim()
        .parse()
        .map_err(|_| format!("{value} is not a number"))?;
    if !(0.0..=100.0).contains(&percent) {
        return Err(format!("{value} is not between 0 and 100"));
    }
    Ok((percent * 10_000.0).round() as u32)
}

/// Writes parts per million as a percentage without trailing zeros.
pub fn percent_text(ppm: u32) -> String {
    trim_number(f64::from(ppm) / 10_000.0)
}

fn millis_text(us: i64) -> String {
    trim_number(us as f64 / 1000.0)
}

fn trim_number(value: f64) -> String {
    let text = format!("{value:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// What the link did, summed over the frames that senders offered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkStats {
    /// Frames that senders offered.
    pub offered: u64,
    /// Frames that random loss removed.
    pub lost: u64,
    /// Frames that the full bottleneck queue dropped.
    pub queue_dropped: u64,
    /// Frames that skipped the delay.
    pub reordered: u64,
    /// Frames that receivers took.
    pub delivered: u64,
    /// Data segments that started below the highest sequence number that
    /// their flow had sent.
    pub retransmits: u64,
}

impl LinkStats {
    /// Adds the counters of another direction.
    #[must_use]
    pub fn plus(self, other: Self) -> Self {
        Self {
            offered: self.offered + other.offered,
            lost: self.lost + other.lost,
            queue_dropped: self.queue_dropped + other.queue_dropped,
            reordered: self.reordered + other.reordered,
            delivered: self.delivered + other.delivered,
            retransmits: self.retransmits + other.retransmits,
        }
    }
}

/// How the frames of a link are framed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    /// An IPv4 packet starts the frame (the smoltcp link).
    Ip,
    /// An Ethernet header, with an optional 802.1Q tag, comes first (the own
    /// stack link).
    Ethernet,
}

type FlowKey = (u32, u32, u16, u16);

/// A frame that waits for its release time. Ordering looks at the key only.
struct Held {
    key: Reverse<(i64, u64)>,
    frame: Vec<u8>,
}

impl PartialEq for Held {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl Eq for Held {}
impl PartialOrd for Held {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Held {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

/// One direction of an impaired link.
pub struct Impairment {
    config: ImpairConfig,
    framing: Framing,
    rng: u64,
    /// When the bottleneck has serialized everything that it accepted.
    free_at_us: i64,
    held: BinaryHeap<Held>,
    order: u64,
    stats: LinkStats,
    sent_high: HashMap<FlowKey, u32>,
}

impl Impairment {
    /// Creates the model of one direction.
    pub fn new(config: ImpairConfig, framing: Framing) -> Self {
        Self {
            config,
            framing,
            // xorshift needs a nonzero state.
            rng: config.seed | 1,
            free_at_us: 0,
            held: BinaryHeap::new(),
            order: 0,
            stats: LinkStats::default(),
            sent_high: HashMap::new(),
        }
    }

    fn draw_ppm(&mut self) -> u32 {
        // xorshift64*, which is plenty for a loss pattern.
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        let r = x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32;
        ((r * u64::from(PPM)) >> 32) as u32
    }

    /// Offers a frame at time `now_us`.
    pub fn offer(&mut self, now_us: i64, frame: Vec<u8>) {
        self.stats.offered += 1;
        if let Some((flow, seq, len)) = tcp_data(&frame, self.framing) {
            self.note_segment(flow, seq, len);
        }
        let mut leave = now_us;
        if self.config.rate_bps > 0 {
            let rate = self.config.rate_bps as i128;
            let backlog_us = (self.free_at_us - now_us).max(0);
            let backlog_bytes = i128::from(backlog_us) * rate / 8_000_000;
            if backlog_bytes + frame.len() as i128 > self.config.queue_bytes as i128 {
                self.stats.queue_dropped += 1;
                return;
            }
            let serialize = (frame.len() as i128 * 8_000_000 + rate - 1) / rate;
            self.free_at_us = self.free_at_us.max(now_us) + serialize as i64;
            leave = self.free_at_us;
        }
        if self.config.loss_ppm > 0 && self.draw_ppm() < self.config.loss_ppm {
            self.stats.lost += 1;
            return;
        }
        let mut release = leave + self.config.delay_us;
        if self.config.reorder_ppm > 0 && self.draw_ppm() < self.config.reorder_ppm {
            self.stats.reordered += 1;
            release = leave;
        }
        self.order += 1;
        self.held.push(Held {
            key: Reverse((release, self.order)),
            frame,
        });
    }

    fn note_segment(&mut self, flow: FlowKey, seq: u32, len: u32) {
        let end = seq.wrapping_add(len);
        match self.sent_high.get_mut(&flow) {
            Some(high) => {
                if (seq.wrapping_sub(*high) as i32) < 0 {
                    self.stats.retransmits += 1;
                }
                if (end.wrapping_sub(*high) as i32) > 0 {
                    *high = end;
                }
            }
            None => {
                self.sent_high.insert(flow, end);
            }
        }
    }

    /// Takes the next frame whose release time is not after `now_us`.
    pub fn pop_due(&mut self, now_us: i64) -> Option<Vec<u8>> {
        if self.held.peek()?.key.0 .0 > now_us {
            return None;
        }
        self.stats.delivered += 1;
        self.held.pop().map(|held| held.frame)
    }

    /// Returns the release time of the earliest held frame.
    pub fn next_release(&self) -> Option<i64> {
        self.held.peek().map(|held| held.key.0 .0)
    }

    /// Returns the frames that wait.
    pub fn held(&self) -> usize {
        self.held.len()
    }

    /// Returns the counters of this direction.
    pub fn stats(&self) -> LinkStats {
        self.stats
    }
}

/// Finds the flow, the sequence number, and the payload length of a TCP data
/// segment in `frame`, or `None` for anything else.
fn tcp_data(frame: &[u8], framing: Framing) -> Option<(FlowKey, u32, u32)> {
    let ip = match framing {
        Framing::Ip => frame,
        Framing::Ethernet => {
            let mut at = 12;
            let mut kind = u16::from_be_bytes([*frame.get(at)?, *frame.get(at + 1)?]);
            if kind == 0x8100 {
                at += 4;
                kind = u16::from_be_bytes([*frame.get(at)?, *frame.get(at + 1)?]);
            }
            if kind != 0x0800 {
                return None;
            }
            frame.get(at + 2..)?
        }
    };
    if *ip.first()? >> 4 != 4 || *ip.get(9)? != 6 {
        return None;
    }
    let ihl = usize::from(ip[0] & 0x0F) * 4;
    if ihl < 20 || ip.len() < 20 {
        return None;
    }
    let total = usize::from(u16::from_be_bytes([ip[2], ip[3]]));
    let tcp = ip.get(ihl..total.min(ip.len()))?;
    let offset = usize::from(*tcp.get(12)? >> 4) * 4;
    let payload = tcp.len().checked_sub(offset)?;
    if payload == 0 || tcp.len() < 20 {
        return None;
    }
    let word = |b: &[u8], i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let flow = (
        word(ip, 12),
        word(ip, 16),
        u16::from_be_bytes([tcp[0], tcp[1]]),
        u16::from_be_bytes([tcp[2], tcp[3]]),
    );
    Some((flow, word(tcp, 4), u32::try_from(payload).ok()?))
}

/// Both directions of a link, with the same settings and their own random
/// sequences.
pub struct ImpairedLink {
    /// Frames from the client to the server, then from the server to the
    /// client.
    pub dirs: [Impairment; 2],
}

impl ImpairedLink {
    /// Creates a link.
    pub fn new(config: ImpairConfig, framing: Framing) -> Self {
        Self {
            dirs: [
                Impairment::new(config, framing),
                Impairment::new(config.reverse(), framing),
            ],
        }
    }

    /// Returns the counters of both directions.
    pub fn stats(&self) -> LinkStats {
        self.dirs[0].stats().plus(self.dirs[1].stats())
    }

    /// Returns the earliest release time over both directions.
    pub fn next_release(&self) -> Option<i64> {
        match (self.dirs[0].next_release(), self.dirs[1].next_release()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

/// The `tc` commands that set up one end of the kernel link, as argument
/// lists that follow `tc`, for the device `dev`.
///
/// With a rate the first command adds a `tbf` qdisc at the root and the
/// `netem` qdisc goes below it (`parent 1:1`), so the frames pass the
/// bottleneck first, as in the model. Without a rate `netem` is the root.
/// A link that is clean gives no command. The token bucket of `tbf` holds
/// 5 ms of the rate, at least 6,000 bytes, because `tbf` cannot pass more
/// than one bucket per timer tick.
pub fn netem_commands(config: &ImpairConfig, dev: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let strings =
        |parts: &[&str]| -> Vec<String> { parts.iter().map(|s| (*s).to_string()).collect() };
    let netem_needed = config.loss_ppm > 0 || config.delay_us > 0 || config.reorder_ppm > 0;
    let mut place = vec!["qdisc", "add", "dev", dev];
    if config.rate_bps > 0 {
        let burst = (config.rate_bps / 8 / 200).max(6_000);
        let mut args = strings(&place);
        args.extend(strings(&["root", "handle", "1:", "tbf"]));
        args.extend([
            "rate".to_string(),
            format!("{}bit", config.rate_bps),
            "burst".to_string(),
            burst.to_string(),
            "limit".to_string(),
            config.queue_bytes.to_string(),
        ]);
        out.push(args);
        place.extend(["parent", "1:1", "handle", "10:"]);
    } else {
        place.push("root");
    }
    if netem_needed {
        let mut args = strings(&place);
        args.push("netem".into());
        // The packet limit of netem bounds the frames in flight, so it must
        // hold the bandwidth delay product of a fast link.
        args.extend(["limit".to_string(), "100000".to_string()]);
        if config.delay_us > 0 {
            args.extend(["delay".to_string(), format!("{}us", config.delay_us)]);
        }
        if config.loss_ppm > 0 {
            args.extend([
                "loss".to_string(),
                format!("{}%", percent_text(config.loss_ppm)),
            ]);
        }
        if config.reorder_ppm > 0 {
            args.extend([
                "reorder".to_string(),
                format!("{}%", percent_text(config.reorder_ppm)),
            ]);
        }
        out.push(args);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_frame(seq: u32, len: usize) -> Vec<u8> {
        let total = 20 + 20 + len;
        let mut ip = vec![0u8; total];
        ip[0] = 0x45;
        ip[2..4].copy_from_slice(&(total as u16).to_be_bytes());
        ip[9] = 6;
        ip[12..16].copy_from_slice(&[10, 0, 0, 1]);
        ip[16..20].copy_from_slice(&[10, 0, 0, 2]);
        ip[20..22].copy_from_slice(&1000u16.to_be_bytes());
        ip[22..24].copy_from_slice(&80u16.to_be_bytes());
        ip[24..28].copy_from_slice(&seq.to_be_bytes());
        ip[32] = 5 << 4;
        ip
    }

    fn drain(link: &mut Impairment, now: i64) -> usize {
        let mut n = 0;
        while link.pop_due(now).is_some() {
            n += 1;
        }
        n
    }

    #[test]
    fn a_clean_link_delivers_at_once_and_in_order() {
        let mut link = Impairment::new(ImpairConfig::default(), Framing::Ip);
        for i in 0..5u8 {
            link.offer(100, vec![i]);
        }
        let got: Vec<u8> = std::iter::from_fn(|| link.pop_due(100))
            .map(|f| f[0])
            .collect();
        assert_eq!(got, vec![0, 1, 2, 3, 4]);
        assert_eq!(link.stats().lost, 0);
    }

    #[test]
    fn delay_holds_a_frame_until_its_time() {
        let config = ImpairConfig {
            delay_us: 20_000,
            ..ImpairConfig::default()
        };
        let mut link = Impairment::new(config, Framing::Ip);
        link.offer(1_000, vec![1]);
        assert_eq!(link.next_release(), Some(21_000));
        assert!(link.pop_due(20_999).is_none());
        assert!(link.pop_due(21_000).is_some());
    }

    #[test]
    fn loss_rate_matches_the_setting_and_is_repeatable() {
        let run = |seed: u64| {
            let config = ImpairConfig {
                loss_ppm: 10_000,
                seed,
                ..ImpairConfig::default()
            };
            let mut link = Impairment::new(config, Framing::Ip);
            for _ in 0..200_000 {
                link.offer(0, vec![0]);
            }
            link.stats().lost
        };
        let lost = run(7);
        // 1 percent of 200,000 is 2,000 with a spread of about 45.
        assert!((1_700..=2_300).contains(&lost), "lost {lost}");
        assert_eq!(lost, run(7));
        assert_ne!(lost, run(8));
    }

    #[test]
    fn zero_and_full_loss_are_exact() {
        let mut never = Impairment::new(ImpairConfig::default(), Framing::Ip);
        let all = ImpairConfig {
            loss_ppm: PPM,
            ..ImpairConfig::default()
        };
        let mut always = Impairment::new(all, Framing::Ip);
        for _ in 0..1000 {
            never.offer(0, vec![0]);
            always.offer(0, vec![0]);
        }
        assert_eq!(never.stats().lost, 0);
        assert_eq!(always.stats().lost, 1000);
        assert_eq!(always.held(), 0);
    }

    #[test]
    fn reordered_frames_skip_the_delay_and_overtake() {
        let config = ImpairConfig {
            delay_us: 10_000,
            reorder_ppm: PPM / 2,
            ..ImpairConfig::default()
        };
        let mut link = Impairment::new(config, Framing::Ip);
        for i in 0..200u8 {
            link.offer(i64::from(i), vec![i]);
        }
        let mut seen = Vec::new();
        for now in 0..10_300 {
            while let Some(f) = link.pop_due(now) {
                seen.push(f[0]);
            }
        }
        assert_eq!(seen.len(), 200);
        assert!(link.stats().reordered > 60 && link.stats().reordered < 140);
        assert!(seen.windows(2).any(|w| w[0] > w[1]), "no frame overtook");
    }

    #[test]
    fn the_rate_serializes_frames_and_the_queue_drops_the_excess() {
        // 8 Mbit/s moves 1,000 bytes per millisecond.
        let config = ImpairConfig {
            rate_bps: 8_000_000,
            queue_bytes: 5_000,
            ..ImpairConfig::default()
        };
        let mut link = Impairment::new(config, Framing::Ip);
        for _ in 0..10 {
            link.offer(0, vec![0; 1000]);
        }
        // The queue holds 5,000 bytes, so five frames fit and five drop.
        assert_eq!(link.stats().queue_dropped, 5);
        assert_eq!(link.next_release(), Some(1_000));
        assert_eq!(drain(&mut link, 1_000), 1);
        assert_eq!(drain(&mut link, 5_000), 4);
        // Once it has drained, the bottleneck accepts frames again.
        link.offer(6_000, vec![0; 1000]);
        assert_eq!(link.stats().queue_dropped, 5);
        assert_eq!(link.next_release(), Some(7_000));
    }

    #[test]
    fn rate_delay_and_loss_combine_in_the_kernel_order() {
        let config = ImpairConfig {
            rate_bps: 8_000_000,
            delay_us: 20_000,
            loss_ppm: PPM,
            ..ImpairConfig::default()
        };
        let mut link = Impairment::new(config, Framing::Ip);
        link.offer(0, vec![0; 1000]);
        // The lost frame still used the bottleneck for one millisecond.
        link.offer(0, vec![0; 1000]);
        assert_eq!(link.stats().lost, 2);
        assert_eq!(link.free_at_us, 2_000);
    }

    #[test]
    fn retransmissions_are_data_segments_below_the_highest_sequence() {
        let mut link = Impairment::new(ImpairConfig::default(), Framing::Ip);
        link.offer(0, data_frame(1000, 100));
        link.offer(0, data_frame(1100, 100));
        link.offer(0, data_frame(1200, 100));
        assert_eq!(link.stats().retransmits, 0);
        // The sender goes back to 1100 and sends 1100 and 1200 again.
        link.offer(0, data_frame(1100, 100));
        link.offer(0, data_frame(1200, 100));
        assert_eq!(link.stats().retransmits, 2);
        // New data after the resend is not a retransmission.
        link.offer(0, data_frame(1300, 100));
        assert_eq!(link.stats().retransmits, 2);
    }

    #[test]
    fn sequence_numbers_wrap_without_a_false_retransmission() {
        let mut link = Impairment::new(ImpairConfig::default(), Framing::Ip);
        link.offer(0, data_frame(u32::MAX - 49, 100));
        link.offer(0, data_frame(50, 100));
        assert_eq!(link.stats().retransmits, 0);
        link.offer(0, data_frame(50, 100));
        assert_eq!(link.stats().retransmits, 1);
    }

    #[test]
    fn ethernet_framing_skips_the_header_and_the_vlan_tag() {
        let ip = data_frame(5, 10);
        for tagged in [false, true] {
            let mut frame = vec![0u8; 12];
            if tagged {
                frame.extend([0x81, 0x00, 0x00, 0x64]);
            }
            frame.extend([0x08, 0x00]);
            frame.extend(&ip);
            let mut link = Impairment::new(ImpairConfig::default(), Framing::Ethernet);
            link.offer(0, frame.clone());
            link.offer(0, frame);
            assert_eq!(link.stats().retransmits, 1, "tagged {tagged}");
        }
    }

    #[test]
    fn validation_rejects_reordering_without_delay() {
        let bad = ImpairConfig {
            reorder_ppm: 1,
            ..ImpairConfig::default()
        };
        assert!(bad.validate().is_err());
        assert!(ImpairConfig::default().validate().is_ok());
    }

    #[test]
    fn percentages_round_trip() {
        assert_eq!(percent_to_ppm("0.1"), Ok(1_000));
        assert_eq!(percent_to_ppm("1"), Ok(10_000));
        assert!(percent_to_ppm("101").is_err());
        assert!(percent_to_ppm("x").is_err());
        assert_eq!(percent_text(1_000), "0.1");
        assert_eq!(percent_text(10_000), "1");
        assert_eq!(percent_text(0), "0");
    }

    #[test]
    fn netem_commands_follow_the_kernel_order() {
        let config = ImpairConfig {
            loss_ppm: 1_000,
            delay_us: 20_000,
            reorder_ppm: 250_000,
            ..ImpairConfig::default()
        };
        let one = netem_commands(&config, "vc");
        assert_eq!(one.len(), 1);
        assert_eq!(
            one[0].join(" "),
            "qdisc add dev vc root netem limit 100000 delay 20000us loss 0.1% reorder 25%"
        );
        let rated = ImpairConfig {
            rate_bps: 100_000_000,
            queue_bytes: 131_072,
            ..config
        };
        let two = netem_commands(&rated, "vc");
        assert_eq!(two.len(), 2);
        assert_eq!(
            two[0].join(" "),
            "qdisc add dev vc root handle 1: tbf rate 100000000bit burst 62500 limit 131072"
        );
        assert!(two[1]
            .join(" ")
            .starts_with("qdisc add dev vc parent 1:1 handle 10: netem"));
        assert!(netem_commands(&ImpairConfig::default(), "vc").is_empty());
        let only_rate = ImpairConfig {
            rate_bps: 1_000_000,
            ..ImpairConfig::default()
        };
        assert_eq!(netem_commands(&only_rate, "vc").len(), 1);
    }
}
