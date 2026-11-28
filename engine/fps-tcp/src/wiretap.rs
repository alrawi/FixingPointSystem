// SPDX-License-Identifier: GPL-3.0-only
//! A tap that reads the TCP options on the wire (TRN-26).
//!
//! The interop tests check what two stacks agreed on from the frames that they
//! exchanged, and not from the state of either stack. The parser is the one of
//! smoltcp (`smoltcp::wire`), which neither the own stack nor the kernel
//! share, so a malformed option, a wrong checksum, or an option that one side
//! sends and the other does not accept shows up as a difference. The tap reads
//! the first connection that it sees: the first SYN names the opener, and the
//! summary keeps the options of the SYN and of the SYN-ACK and, per direction,
//! how many segments carried data, timestamps, or SACK blocks and the largest
//! window that the sender advertised after scaling.

use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use fps_backend_afxdp::{FrameIo, QueueStats};
use smoltcp::phy::ChecksumCapabilities;
use smoltcp::wire::{IpAddress, IpProtocol, Ipv4Packet, TcpControl, TcpPacket, TcpRepr};

const ETHERTYPE_IPV4: [u8; 2] = [0x08, 0x00];
const ETHERTYPE_VLAN: [u8; 2] = [0x81, 0x00];

/// The options of one SYN or SYN-ACK.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SynOptions {
    /// The maximum segment size option, if present.
    pub mss: Option<u16>,
    /// The window scale shift, if present.
    pub window_scale: Option<u8>,
    /// Whether the segment carries the SACK permitted option.
    pub sack_permitted: bool,
    /// Whether the segment carries the timestamps option.
    pub timestamps: bool,
}

/// What one direction of the connection carried after the handshake.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirSummary {
    /// Segments of this direction, handshake included.
    pub segments: u64,
    /// Segments with payload.
    pub data_segments: u64,
    /// Data segments that carried the timestamps option.
    pub data_with_timestamps: u64,
    /// Segments after the handshake that carried the timestamps option.
    pub with_timestamps: u64,
    /// Segments that carried at least one SACK block.
    pub sack_segments: u64,
    /// The largest payload of a segment in bytes.
    pub max_payload: u64,
    /// The largest IP packet in bytes.
    pub max_ip_len: u64,
    /// The largest window that the sender advertised after the handshake, in
    /// bytes, scaled by the shift of its own SYN when both SYNs had the
    /// option, and unscaled otherwise.
    pub max_window: u64,
}

/// The options that two stacks used on the first connection of a tap.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WireSummary {
    /// The first SYN, from the opener.
    pub syn: Option<SynOptions>,
    /// The first SYN-ACK, from the acceptor.
    pub syn_ack: Option<SynOptions>,
    /// Segments from the opener to the acceptor.
    pub opener: DirSummary,
    /// Segments from the acceptor to the opener.
    pub acceptor: DirSummary,
    /// IPv4 TCP packets with a wrong IP or TCP checksum.
    pub bad_checksums: u64,
    /// IPv4 TCP packets that the parser refused for another reason.
    pub malformed: u64,
}

impl WireSummary {
    /// Whether both SYNs offered the window scale option.
    pub fn window_scale_agreed(&self) -> bool {
        matches!((self.syn, self.syn_ack), (Some(a), Some(b)) if a.window_scale.is_some() && b.window_scale.is_some())
    }

    /// Whether both SYNs offered SACK.
    pub fn sack_agreed(&self) -> bool {
        matches!((self.syn, self.syn_ack), (Some(a), Some(b)) if a.sack_permitted && b.sack_permitted)
    }

    /// Whether both SYNs offered timestamps.
    pub fn timestamps_agreed(&self) -> bool {
        matches!((self.syn, self.syn_ack), (Some(a), Some(b)) if a.timestamps && b.timestamps)
    }

    /// The MSS that applies to the opener's sends: the smaller of the two
    /// offers, or 536 when the peer sent none (RFC 9293).
    pub fn effective_mss(&self) -> Option<u16> {
        let (a, b) = (self.syn?, self.syn_ack?);
        Some(a.mss.unwrap_or(536).min(b.mss.unwrap_or(536)))
    }

    /// One line of `key=value` words that [`WireSummary::from_line`] reads
    /// back, so a helper process can hand its tap to the test that started it.
    pub fn to_line(&self) -> String {
        fn syn(prefix: &str, s: Option<SynOptions>, out: &mut Vec<String>) {
            let s = s.unwrap_or_default();
            out.push(format!(
                "{prefix}_seen={}",
                u8::from(s != SynOptions::default())
            ));
            out.push(format!("{prefix}_mss={}", s.mss.map_or(-1, i64::from)));
            out.push(format!(
                "{prefix}_ws={}",
                s.window_scale.map_or(-1, i64::from)
            ));
            out.push(format!("{prefix}_sack={}", u8::from(s.sack_permitted)));
            out.push(format!("{prefix}_ts={}", u8::from(s.timestamps)));
        }
        fn dir(prefix: &str, d: &DirSummary, out: &mut Vec<String>) {
            out.push(format!("{prefix}_segments={}", d.segments));
            out.push(format!("{prefix}_data={}", d.data_segments));
            out.push(format!("{prefix}_data_ts={}", d.data_with_timestamps));
            out.push(format!("{prefix}_ts_all={}", d.with_timestamps));
            out.push(format!("{prefix}_sack_segments={}", d.sack_segments));
            out.push(format!("{prefix}_max_payload={}", d.max_payload));
            out.push(format!("{prefix}_max_ip={}", d.max_ip_len));
            out.push(format!("{prefix}_max_window={}", d.max_window));
        }
        let mut out = Vec::new();
        syn("syn", self.syn, &mut out);
        syn("synack", self.syn_ack, &mut out);
        dir("opener", &self.opener, &mut out);
        dir("acceptor", &self.acceptor, &mut out);
        out.push(format!("bad_checksums={}", self.bad_checksums));
        out.push(format!("malformed={}", self.malformed));
        out.join(" ")
    }

    /// Reads the output of [`WireSummary::to_line`].
    pub fn from_line(line: &str) -> Option<Self> {
        let get = |key: &str| -> Option<i64> {
            line.split_whitespace()
                .find_map(|w| w.strip_prefix(key)?.strip_prefix('=')?.parse().ok())
        };
        let syn = |prefix: &str| -> Option<Option<SynOptions>> {
            if get(&format!("{prefix}_seen"))? == 0 {
                return Some(None);
            }
            Some(Some(SynOptions {
                mss: u16::try_from(get(&format!("{prefix}_mss"))?).ok(),
                window_scale: u8::try_from(get(&format!("{prefix}_ws"))?).ok(),
                sack_permitted: get(&format!("{prefix}_sack"))? == 1,
                timestamps: get(&format!("{prefix}_ts"))? == 1,
            }))
        };
        let dir = |prefix: &str| -> Option<DirSummary> {
            let n = |k: &str| get(&format!("{prefix}_{k}")).and_then(|v| u64::try_from(v).ok());
            Some(DirSummary {
                segments: n("segments")?,
                data_segments: n("data")?,
                data_with_timestamps: n("data_ts")?,
                with_timestamps: n("ts_all")?,
                sack_segments: n("sack_segments")?,
                max_payload: n("max_payload")?,
                max_ip_len: n("max_ip")?,
                max_window: n("max_window")?,
            })
        };
        Some(Self {
            syn: syn("syn")?,
            syn_ack: syn("synack")?,
            opener: dir("opener")?,
            acceptor: dir("acceptor")?,
            bad_checksums: u64::try_from(get("bad_checksums")?).ok()?,
            malformed: u64::try_from(get("malformed")?).ok()?,
        })
    }
}

fn options_of(repr: &TcpRepr<'_>) -> SynOptions {
    SynOptions {
        mss: repr.max_seg_size,
        window_scale: repr.window_scale,
        sack_permitted: repr.sack_permitted,
        timestamps: repr.timestamp.is_some(),
    }
}

/// The tap itself. It keeps counters and no frames.
#[derive(Debug, Default)]
pub struct Tap {
    summary: WireSummary,
    /// The source address and port of the first SYN, as numbers.
    opener: Option<(u32, u16)>,
    opener_shift: Option<u8>,
    acceptor_shift: Option<u8>,
}

impl Tap {
    /// Reads one Ethernet frame (with an 802.1Q tag or without) that passed
    /// the wire. Frames that are not IPv4 TCP are ignored.
    pub fn record_frame(&mut self, frame: &[u8]) {
        let Some(ethertype) = frame.get(12..14) else {
            return;
        };
        let (ethertype, offset) = if ethertype == ETHERTYPE_VLAN {
            match frame.get(16..18) {
                Some(inner) => (inner, 18),
                None => return,
            }
        } else {
            (ethertype, 14)
        };
        if ethertype == ETHERTYPE_IPV4 {
            if let Some(packet) = frame.get(offset..) {
                self.record_packet(packet);
            }
        }
    }

    /// Reads one IPv4 packet.
    pub fn record_packet(&mut self, packet: &[u8]) {
        let Ok(ip) = Ipv4Packet::new_checked(packet) else {
            return;
        };
        if ip.next_header() != IpProtocol::Tcp {
            return;
        }
        if !ip.verify_checksum() {
            self.summary.bad_checksums += 1;
            return;
        }
        let (src, dst) = (ip.src_addr(), ip.dst_addr());
        let Ok(tcp) = TcpPacket::new_checked(ip.payload()) else {
            self.summary.malformed += 1;
            return;
        };
        if !tcp.verify_checksum(&IpAddress::Ipv4(src), &IpAddress::Ipv4(dst)) {
            self.summary.bad_checksums += 1;
            return;
        }
        let parsed = TcpRepr::parse(
            &tcp,
            &IpAddress::Ipv4(src),
            &IpAddress::Ipv4(dst),
            &ChecksumCapabilities::default(),
        );
        let Ok(repr) = parsed else {
            self.summary.malformed += 1;
            return;
        };
        let (src_key, dst_key) = (
            (u32::from(src), repr.src_port),
            (u32::from(dst), repr.dst_port),
        );
        if repr.control == TcpControl::Syn && repr.ack_number.is_none() {
            if self.opener.is_none() {
                self.opener = Some(src_key);
                self.summary.syn = Some(options_of(&repr));
                self.opener_shift = repr.window_scale;
            }
        } else if repr.control == TcpControl::Syn
            && repr.ack_number.is_some()
            && self.opener == Some(dst_key)
            && self.summary.syn_ack.is_none()
        {
            self.summary.syn_ack = Some(options_of(&repr));
            self.acceptor_shift = repr.window_scale;
        }
        let from_opener = match self.opener {
            Some(key) if key == src_key => true,
            Some(key) if key == dst_key => false,
            _ => return,
        };
        let both_scaled = self.opener_shift.is_some() && self.acceptor_shift.is_some();
        let shift = if from_opener {
            self.opener_shift
        } else {
            self.acceptor_shift
        };
        let handshake = repr.control == TcpControl::Syn;
        let dir = if from_opener {
            &mut self.summary.opener
        } else {
            &mut self.summary.acceptor
        };
        dir.segments += 1;
        let payload = repr.payload.len() as u64;
        dir.max_payload = dir.max_payload.max(payload);
        dir.max_ip_len = dir.max_ip_len.max(u64::from(ip.total_len()));
        if payload > 0 {
            dir.data_segments += 1;
            if repr.timestamp.is_some() {
                dir.data_with_timestamps += 1;
            }
        }
        if handshake {
            return;
        }
        if repr.timestamp.is_some() {
            dir.with_timestamps += 1;
        }
        if repr.sack_ranges.iter().any(Option::is_some) {
            dir.sack_segments += 1;
        }
        let scaled = if both_scaled {
            u64::from(repr.window_len) << u32::from(shift.unwrap_or(0).min(14))
        } else {
            u64::from(repr.window_len)
        };
        dir.max_window = dir.max_window.max(scaled);
    }

    /// The summary so far.
    pub fn summary(&self) -> WireSummary {
        self.summary.clone()
    }
}

/// A tap that several owners share.
#[derive(Clone, Debug, Default)]
pub struct SharedTap(Arc<Mutex<Tap>>);

impl SharedTap {
    /// A new, empty tap.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Tap> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Reads one Ethernet frame.
    pub fn record_frame(&self, frame: &[u8]) {
        self.lock().record_frame(frame);
    }

    /// Reads one IPv4 packet.
    pub fn record_packet(&self, packet: &[u8]) {
        self.lock().record_packet(packet);
    }

    /// The summary so far.
    pub fn summary(&self) -> WireSummary {
        self.lock().summary()
    }
}

/// What the own stack was set to do in a run, for [`violations`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnSide {
    /// Whether the own stack opened the connection.
    pub opener: bool,
    /// Its MTU, so its MSS is this minus 40.
    pub mtu: usize,
    /// Whether it offers window scaling.
    pub window_scale: bool,
    /// Whether it offers SACK.
    pub sack: bool,
    /// Whether it offers timestamps.
    pub timestamps: bool,
    /// Its receive buffer in bytes, the most that it advertises.
    pub rcv_buffer: usize,
}

/// Compares a summary with what the RFCs ask of two stacks, one of them the
/// own stack described by `own`, and returns what differs. `loss` says the
/// run lost frames, so SACK blocks are due; `peer_rcv_buffer` is the receive
/// buffer of the peer when it is known and bounds its advertised window.
///
/// The checks hold for any peer: an acceptor answers an option only when the
/// SYN had it (RFC 7323, RFC 2018); the own stack offers what it is set to
/// offer and, as an acceptor, answers what it offers and the opener offered;
/// no segment is larger than the MSS or the MTU; timestamps appear on every
/// data segment when both SYNs had them and on none when they did not; a
/// window above 65,535 bytes needs the scale on both SYNs; and after a loss
/// the receiver of the bulk reports holes with SACK blocks when both SYNs
/// had SACK, and never when they did not.
pub fn violations(
    wire: &WireSummary,
    own: &OwnSide,
    loss: bool,
    peer_rcv_buffer: Option<u64>,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut check = |ok: bool, what: &str| {
        if !ok {
            out.push(what.to_string());
        }
    };
    check(wire.bad_checksums == 0, "a segment had a wrong checksum");
    check(
        wire.malformed == 0,
        "a segment had options that smoltcp cannot parse",
    );
    let (Some(syn), Some(syn_ack)) = (wire.syn, wire.syn_ack) else {
        out.push("the tap saw no handshake".to_string());
        return out;
    };
    check(
        !syn_ack.timestamps || syn.timestamps,
        "the SYN-ACK has timestamps that the SYN did not offer",
    );
    check(
        syn_ack.window_scale.is_none() || syn.window_scale.is_some(),
        "the SYN-ACK has a window scale that the SYN did not offer",
    );
    check(
        !syn_ack.sack_permitted || syn.sack_permitted,
        "the SYN-ACK has SACK permitted that the SYN did not offer",
    );
    let (own_syn, peer_syn) = if own.opener {
        (syn, syn_ack)
    } else {
        (syn_ack, syn)
    };
    let mss = u16::try_from(own.mtu.saturating_sub(40)).ok();
    check(
        own_syn.mss == mss,
        "the MSS of the own stack is not its MTU minus 40",
    );
    let offered = |configured: bool, peer: bool| configured && (own.opener || peer);
    check(
        own_syn.window_scale.is_some()
            == offered(own.window_scale, peer_syn.window_scale.is_some()),
        "the window scale of the own stack differs from its setting",
    );
    check(
        own_syn.sack_permitted == offered(own.sack, peer_syn.sack_permitted),
        "SACK permitted of the own stack differs from its setting",
    );
    check(
        own_syn.timestamps == offered(own.timestamps, peer_syn.timestamps),
        "the timestamps of the own stack differ from its setting",
    );
    let effective = u64::from(wire.effective_mss().unwrap_or(536));
    for (name, dir) in [("opener", wire.opener), ("acceptor", wire.acceptor)] {
        check(dir.segments > 1, &format!("the {name} sent nothing"));
        check(
            dir.max_payload <= effective,
            &format!("the {name} sent a segment above the MSS"),
        );
        check(
            dir.max_ip_len <= own.mtu as u64,
            &format!("the {name} sent a packet above the MTU"),
        );
    }
    if wire.timestamps_agreed() {
        for (name, dir) in [("opener", wire.opener), ("acceptor", wire.acceptor)] {
            check(
                dir.data_with_timestamps == dir.data_segments,
                &format!("a data segment of the {name} has no timestamps"),
            );
            check(
                dir.with_timestamps > 0,
                &format!("the {name} never sent timestamps"),
            );
        }
    } else {
        check(
            wire.opener.with_timestamps == 0 && wire.acceptor.with_timestamps == 0,
            "timestamps on the wire without agreement",
        );
    }
    let (own_dir, peer_dir) = if own.opener {
        (wire.opener, wire.acceptor)
    } else {
        (wire.acceptor, wire.opener)
    };
    if wire.window_scale_agreed() {
        check(
            own_dir.max_window > 65_535,
            "the scale is agreed but never used",
        );
        check(
            own_dir.max_window <= own.rcv_buffer as u64,
            "the own stack advertises more than its buffer",
        );
        if let Some(cap) = peer_rcv_buffer {
            check(
                peer_dir.max_window <= cap,
                "the peer advertises more than its buffer",
            );
        }
    } else {
        check(
            own_dir.max_window <= 65_535 && peer_dir.max_window <= 65_535,
            "a window above 65,535 bytes without a scale",
        );
    }
    // The opener sends the bulk, so the acceptor reports holes.
    if wire.sack_agreed() && loss {
        check(
            wire.acceptor.sack_segments > 0,
            "no SACK blocks after a loss although both offered SACK",
        );
    }
    if !wire.sack_agreed() {
        check(
            wire.acceptor.sack_segments == 0 && wire.opener.sack_segments == 0,
            "SACK blocks without agreement",
        );
    }
    // The opener sends the bulk and the acceptor sends little: an answer and
    // a FIN. On a clean link the opener has no hole to report, so it sends no
    // SACK block. On a link that drops frames the acceptor's own segments can
    // be lost too: a lost answer followed by an arriving FIN (an own acceptor
    // always sends the FIN in a segment of its own, after the data) is a hole,
    // and a repeated segment is reported as a duplicate (RFC 2883), both with
    // a SACK block. Each acceptor segment that takes up sequence space
    // raises at most one such ACK, so the opener may send at most as many as
    // the acceptor sent data segments (retransmissions included), plus the
    // FIN and one FIN again.
    if loss {
        let due = wire.acceptor.data_segments + 2;
        check(
            wire.opener.sack_segments <= due,
            "the opener reports more holes than the acceptor sent segments",
        );
    } else {
        check(
            wire.opener.sack_segments == 0,
            "the opener has no holes to report",
        );
    }
    out
}

/// A [`FrameIo`] that shows every frame that it moves to a tap, and passes
/// the frames on unchanged.
pub struct TappedFrames {
    inner: Box<dyn FrameIo>,
    tap: SharedTap,
}

impl TappedFrames {
    /// Wraps `inner`.
    pub fn new(inner: Box<dyn FrameIo>, tap: SharedTap) -> Self {
        Self { inner, tap }
    }
}

impl FrameIo for TappedFrames {
    fn queue_id(&self) -> u32 {
        self.inner.queue_id()
    }

    fn max_frame_len(&self) -> usize {
        self.inner.max_frame_len()
    }

    fn recv_batch(&mut self, max: usize, frame: &mut dyn FnMut(&[u8])) -> io::Result<usize> {
        let tap = &self.tap;
        self.inner.recv_batch(max, &mut |f| {
            tap.record_frame(f);
            frame(f);
        })
    }

    fn send_batch_with(
        &mut self,
        count: usize,
        fill: &mut dyn FnMut(usize, &mut [u8]) -> usize,
    ) -> io::Result<usize> {
        let tap = &self.tap;
        self.inner.send_batch_with(count, &mut |i, buf| {
            let len = fill(i, buf);
            if let Some(sent) = buf.get(..len) {
                tap.record_frame(sent);
            }
            len
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }

    fn wait_readable(&mut self, timeout: Duration) -> io::Result<bool> {
        self.inner.wait_readable(timeout)
    }

    fn stats(&self) -> QueueStats {
        self.inner.stats()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_summary_survives_its_line() {
        let summary = WireSummary {
            syn: Some(SynOptions {
                mss: Some(1460),
                window_scale: Some(5),
                sack_permitted: true,
                timestamps: true,
            }),
            syn_ack: Some(SynOptions {
                mss: Some(1400),
                window_scale: None,
                sack_permitted: true,
                timestamps: false,
            }),
            opener: DirSummary {
                segments: 9,
                data_segments: 4,
                data_with_timestamps: 0,
                with_timestamps: 0,
                sack_segments: 0,
                max_payload: 1400,
                max_ip_len: 1452,
                max_window: 65_535,
            },
            acceptor: DirSummary::default(),
            bad_checksums: 1,
            malformed: 2,
        };
        assert_eq!(WireSummary::from_line(&summary.to_line()), Some(summary));
        assert_eq!(WireSummary::from_line("nothing"), None);
    }

    #[test]
    fn the_opener_reports_holes_only_for_what_the_acceptor_sent() {
        let both = SynOptions {
            mss: Some(1460),
            window_scale: Some(7),
            sack_permitted: true,
            timestamps: true,
        };
        let own = OwnSide {
            opener: false,
            mtu: 1500,
            window_scale: true,
            sack: true,
            timestamps: true,
            rcv_buffer: 1024 * 1024,
        };
        let mut wire = WireSummary {
            syn: Some(both),
            syn_ack: Some(both),
            ..WireSummary::default()
        };
        wire.acceptor.data_segments = 1;
        wire.acceptor.sack_segments = 1;
        let hole = "opener has no holes";
        let many = "more holes than";
        let text = |wire: &WireSummary, loss| violations(wire, &own, loss, None).join("|");
        // A clean link: no hole, no block.
        wire.opener.sack_segments = 1;
        assert!(text(&wire, false).contains(hole));
        // A lost answer with the FIN arriving: a block or two.
        assert!(!text(&wire, true).contains(many) && !text(&wire, true).contains(hole));
        wire.opener.sack_segments = 3;
        assert!(!text(&wire, true).contains(many));
        // More blocks than the acceptor had segments to report on.
        wire.opener.sack_segments = 4;
        assert!(text(&wire, true).contains(many));
    }

    #[test]
    fn agreement_needs_both_syns() {
        let both = SynOptions {
            mss: Some(1460),
            window_scale: Some(7),
            sack_permitted: true,
            timestamps: true,
        };
        let plain = SynOptions {
            mss: Some(1200),
            ..SynOptions::default()
        };
        let mut s = WireSummary {
            syn: Some(both),
            syn_ack: Some(both),
            ..WireSummary::default()
        };
        assert!(s.window_scale_agreed() && s.sack_agreed() && s.timestamps_agreed());
        assert_eq!(s.effective_mss(), Some(1460));
        s.syn_ack = Some(plain);
        assert!(!s.window_scale_agreed() && !s.sack_agreed() && !s.timestamps_agreed());
        assert_eq!(s.effective_mss(), Some(1200));
        s.syn_ack = None;
        assert_eq!(s.effective_mss(), None);
    }

    /// A SYN with MSS, SACK permitted, timestamps, and window scale, built
    /// byte by byte, from 10.0.0.1:1000 to 10.0.1.1:80.
    fn syn_frame() -> Vec<u8> {
        let mut options = vec![
            2, 4, 0x05, 0xB4, // MSS 1460
            4, 2, // SACK permitted
            8, 10, 0, 0, 0, 1, 0, 0, 0, 0, // timestamps
            1, 3, 3, 7, // NOP, window scale 7
        ];
        while options.len() % 4 != 0 {
            options.push(1);
        }
        let tcp_len = 20 + options.len();
        let mut tcp = vec![0u8; tcp_len];
        tcp[0..2].copy_from_slice(&1000u16.to_be_bytes());
        tcp[2..4].copy_from_slice(&80u16.to_be_bytes());
        tcp[4..8].copy_from_slice(&1u32.to_be_bytes());
        tcp[12] = u8::try_from(tcp_len / 4).unwrap_or(5) << 4;
        tcp[13] = 0x02;
        tcp[14..16].copy_from_slice(&65_535u16.to_be_bytes());
        tcp[20..].copy_from_slice(&options);
        let src = smoltcp::wire::Ipv4Address::new(10, 0, 0, 1);
        let dst = smoltcp::wire::Ipv4Address::new(10, 0, 1, 1);
        let mut packet = TcpPacket::new_unchecked(&mut tcp[..]);
        packet.fill_checksum(&IpAddress::Ipv4(src), &IpAddress::Ipv4(dst));
        let total = 20 + tcp_len;
        let mut ip = vec![0u8; total];
        ip[0] = 0x45;
        ip[2..4].copy_from_slice(&u16::try_from(total).unwrap_or(0).to_be_bytes());
        ip[8] = 64;
        ip[9] = 6;
        ip[12..16].copy_from_slice(&src.octets());
        ip[16..20].copy_from_slice(&dst.octets());
        ip[20..].copy_from_slice(&tcp);
        Ipv4Packet::new_unchecked(&mut ip[..]).fill_checksum();
        let mut frame = vec![0u8; 14];
        frame[12..14].copy_from_slice(&ETHERTYPE_IPV4);
        frame.extend_from_slice(&ip);
        frame
    }

    #[test]
    fn a_syn_is_read_with_all_four_options() {
        let mut tap = Tap::default();
        tap.record_frame(&syn_frame());
        let syn = tap.summary().syn.unwrap_or_default();
        assert_eq!(syn.mss, Some(1460));
        assert_eq!(syn.window_scale, Some(7));
        assert!(syn.sack_permitted && syn.timestamps);
        assert_eq!(tap.summary().bad_checksums, 0);
    }

    #[test]
    fn a_wrong_checksum_is_counted_and_not_read() {
        let mut frame = syn_frame();
        let last = frame.len() - 1;
        frame[last] ^= 0xFF;
        let mut tap = Tap::default();
        tap.record_frame(&frame);
        assert_eq!(tap.summary().bad_checksums, 1);
        assert_eq!(tap.summary().syn, None);
    }

    #[test]
    fn other_frames_are_ignored() {
        let mut tap = Tap::default();
        tap.record_frame(&[0u8; 10]);
        tap.record_frame(&[0u8; 60]);
        assert_eq!(tap.summary(), WireSummary::default());
    }
}
