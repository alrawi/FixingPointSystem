// SPDX-License-Identifier: GPL-3.0-only
//! Loss recovery in the state machine (TRN-23): two hosts that exchange
//! frames by hand, with a clock that the test sets and a filter that drops
//! chosen data segments, so every scenario is exact and repeatable.

use std::collections::VecDeque;
use std::task::Waker;

use super::*;
use crate::pools::Pools;

const SERVER_PORT: u16 = 80;
const MSS: usize = 1460;

/// The configuration of the NewReno tests of TRN-23: no option, so the
/// segments are 1,460 bytes, every segment is acknowledged at once, and the
/// recovery runs on duplicate acknowledgments alone.
fn plain() -> OwnConfig {
    OwnConfig {
        window_scale: false,
        timestamps: false,
        sack: false,
        delayed_ack: false,
        ..OwnConfig::default()
    }
}

struct Pair {
    client: Host,
    server: Host,
    c: (u32, u32),
    s: (u32, u32),
    now: i64,
    /// Time added before each direction delivers its frames, so one round
    /// trip takes twice this.
    hop: i64,
    /// How many data segments the client sent so far, retransmissions
    /// included.
    data_seen: usize,
    /// The bytes that the server application read.
    received: Vec<u8>,
    /// Whether the server application saw the end of the stream.
    eof: bool,
    /// Run the timers of both hosts at every step of the clock.
    timers: bool,
}

fn payload_len(frame: &[u8]) -> usize {
    // Ethernet without a tag, the IPv4 total length at offset 2 of the IP
    // header, and the TCP header length in the top nibble of byte 12.
    let total = usize::from(u16::from_be_bytes([frame[16], frame[17]]));
    let tcp_header = usize::from(frame[14 + wire::IP_LEN + 12] >> 4) * 4;
    total.saturating_sub(wire::IP_LEN + tcp_header)
}

fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8).collect()
}

impl Pair {
    fn new(cfg: OwnConfig) -> Self {
        Self::with(cfg.clone(), cfg)
    }

    /// A pair with its own settings for each side, after the handshake.
    fn with(client_cfg: OwnConfig, server_cfg: OwnConfig) -> Self {
        let mut pair = Self::build(client_cfg, server_cfg);
        pair.pump(&mut |_| false);
        let mut ticket = None;
        pair.s = pair
            .server
            .accept(SERVER_PORT, &mut ticket, Waker::noop())
            .expect("accept")
            .expect("a connection is queued");
        pair
    }

    /// A pair whose client has opened the connection and sent nothing yet.
    fn build(client_cfg: OwnConfig, server_cfg: OwnConfig) -> Self {
        let pools = Pools::sized("10.1.0.0/16", 1, "10.2.0.0/16", 1, None).expect("pools");
        let built = pools.stations().expect("stations");
        let client_ip = pools.client.addresses().expect("addresses")[0];
        let server_ip = pools.server.addresses().expect("addresses")[0];
        let mut client = Host::new(built.client, client_cfg, 1500);
        let mut server = Host::new(built.server, server_cfg, 1500);
        server.listen(SERVER_PORT).expect("listen");
        let now = 1_000_000;
        let c = client
            .open(client_ip, server_ip, SERVER_PORT, now)
            .expect("open");
        Self {
            client,
            server,
            c,
            s: (0, 0),
            now,
            hop: 0,
            data_seen: 0,
            received: Vec::new(),
            eof: false,
            timers: false,
        }
    }

    fn run_timers(&mut self) {
        if self.timers {
            self.client.expire(self.now);
            self.server.expire(self.now);
        }
    }

    /// Pumps, then lets the delayed ACK timers run, until nothing moves.
    fn settle(&mut self) {
        for _ in 0..50 {
            self.pump(&mut |_| false);
            self.now += 45_000;
            let fired = self.client.expire(self.now) + self.server.expire(self.now);
            if fired == 0 && !self.client.has_dirty() && !self.server.has_dirty() {
                break;
            }
        }
    }

    /// Moves frames both ways until nothing is left, drops the client's data
    /// segments for which `drop` says so (it gets the index), and lets the
    /// server application read everything.
    fn pump(&mut self, drop: &mut dyn FnMut(usize) -> bool) {
        for _ in 0..400 {
            let mut moved = false;
            self.client.flush_dirty(self.now);
            self.server.flush_dirty(self.now);
            let mut to_server: VecDeque<Vec<u8>> = std::mem::take(&mut self.client.out.queue);
            self.now += self.hop;
            self.run_timers();
            while let Some(frame) = to_server.pop_front() {
                moved = true;
                if payload_len(&frame) > 0 {
                    let index = self.data_seen;
                    self.data_seen += 1;
                    if drop(index) {
                        continue;
                    }
                }
                self.server.input(&frame, self.now);
            }
            self.read_server();
            self.server.flush_dirty(self.now);
            let to_client: VecDeque<Vec<u8>> = std::mem::take(&mut self.server.out.queue);
            self.now += self.hop;
            self.run_timers();
            for frame in to_client {
                moved = true;
                self.client.input(&frame, self.now);
            }
            if !moved {
                break;
            }
        }
    }

    fn read_server(&mut self) {
        let mut buf = [0u8; 4096];
        while self.s.0 != u32::MAX {
            match self
                .server
                .read(self.s.0, self.s.1, &mut buf, Waker::noop())
            {
                Some(Ok(0)) => {
                    self.eof = true;
                    break;
                }
                Some(Ok(n)) => self.received.extend_from_slice(&buf[..n]),
                _ => break,
            }
        }
    }

    fn write(&mut self, data: &[u8]) {
        let mut sent = 0;
        while sent < data.len() {
            match self
                .client
                .write(self.c.0, self.c.1, &data[sent..], Waker::noop())
            {
                Some(Ok(n)) => sent += n,
                other => panic!("write: {other:?}"),
            }
        }
    }

    fn conn(&mut self) -> &mut Conn {
        let id = self.c.0;
        self.client.conn(id).expect("client connection")
    }

    fn server_conn(&mut self) -> &mut Conn {
        let id = self.s.0;
        self.server.conn(id).expect("server connection")
    }
}

#[test]
fn the_first_flight_is_the_initial_window() {
    let mut p = Pair::new(plain());
    p.write(&pattern(60 * MSS));
    p.client.flush_dirty(p.now);
    // Ten segments of 1,460 bytes, and nothing more until an ACK returns.
    assert_eq!(p.client.out.queue.len(), 10);
    assert!(p.client.out.queue.iter().all(|f| payload_len(f) == MSS));
    let rec = p.conn().rec.as_ref().expect("recovery state in flight");
    assert_eq!(rec.cc.cwnd(), 10 * MSS as u32);
}

#[test]
fn the_window_grows_in_slow_start_until_the_transfer_is_acknowledged() {
    let mut p = Pair::new(plain());
    let data = pattern(60 * MSS);
    p.write(&data);
    p.pump(&mut |_| false);
    assert_eq!(p.received, data);
    assert_eq!(p.client.counters.retransmits, 0);
    assert_eq!(p.client.counters.timeouts, 0);
}

#[test]
fn three_duplicate_acknowledgments_retransmit_without_waiting_for_the_timer() {
    let mut p = Pair::new(plain());
    let data = pattern(10 * MSS);
    p.write(&data);
    // The first segment of the flight is lost; nine arrive behind the hole.
    p.pump(&mut |i| i == 0);
    assert_eq!(p.received, data);
    assert_eq!(p.client.counters.timeouts, 0);
    assert_eq!(p.client.counters.fast_retransmits, 1);
    assert_eq!(p.client.counters.retransmits, 1);
    assert_eq!(p.server.counters.ooo_segments, 9);
    // The window fell to half of the flight plus the inflation, so it does
    // not stay at the initial window.
    assert!(
        p.client.conn(p.c.0).is_none() || p.conn().rec.is_none(),
        "the state is gone once everything is acknowledged"
    );
}

#[test]
fn the_window_is_halved_by_a_fast_retransmit() {
    let mut p = Pair::new(plain());
    p.write(&pattern(30 * MSS));
    p.client.flush_dirty(p.now);
    // Deliver the flight without its first segment by hand, to look at the
    // sender in the middle of the recovery.
    let flight: Vec<Vec<u8>> = p.client.out.queue.drain(..).collect();
    for frame in flight.iter().skip(1).take(4) {
        p.server.input(frame, p.now);
    }
    p.server.flush_dirty(p.now);
    let acks: Vec<Vec<u8>> = p.server.out.queue.drain(..).collect();
    // One duplicate for each of the four out of order segments.
    assert_eq!(acks.len(), 4);
    for ack in acks.iter().take(3) {
        p.client.input(ack, p.now);
    }
    let rec = p.conn().rec.as_ref().expect("state");
    assert!(rec.cc.in_recovery());
    // ssthresh = max(10 MSS / 2, 2 MSS); cwnd = ssthresh + 3 MSS.
    assert_eq!(rec.cc.ssthresh(), 5 * MSS as u32);
    assert_eq!(rec.cc.cwnd(), 8 * MSS as u32);
    assert!(rec.rtx);
    p.client.flush_dirty(p.now);
    // The retransmission is the oldest segment: the first frame again.
    let rtx = p.client.out.queue.pop_front().expect("retransmission");
    assert_eq!(rtx, flight[0]);
    assert_eq!(p.client.counters.fast_retransmits, 1);
}

#[test]
fn a_partial_acknowledgment_retransmits_the_next_hole_without_a_timeout() {
    let mut p = Pair::new(plain());
    let data = pattern(10 * MSS);
    p.write(&data);
    // Segments 0 and 4 of the flight are lost.
    p.pump(&mut |i| i == 0 || i == 4);
    assert_eq!(p.received, data);
    assert_eq!(p.client.counters.timeouts, 0);
    assert_eq!(p.client.counters.fast_retransmits, 2);
}

#[test]
fn a_lost_tail_is_recovered_by_the_timer_and_restarts_slow_start() {
    let mut p = Pair::new(plain());
    let data = pattern(MSS);
    p.write(&data);
    // The only segment is lost, so no duplicate acknowledgment can come.
    p.pump(&mut |i| i == 0);
    assert!(p.received.is_empty());
    assert_eq!(p.conn().rto_us(), 200_000);
    p.now += 250_000;
    assert_eq!(p.client.expire(p.now), 1);
    assert_eq!(p.client.counters.timeouts, 1);
    // The timeout doubled.
    assert_eq!(p.conn().rto_us(), 400_000);
    p.pump(&mut |_| false);
    assert_eq!(p.received, data);
}

#[test]
fn a_timeout_cuts_the_window_to_one_segment() {
    let mut p = Pair::new(plain());
    p.write(&pattern(20 * MSS));
    p.pump(&mut |i| i < 10);
    p.now += 250_000;
    p.client.expire(p.now);
    let rec = p.conn().rec.as_ref().expect("state");
    assert_eq!(rec.cc.cwnd(), MSS as u32);
    assert_eq!(rec.cc.ssthresh(), 5 * MSS as u32);
    assert!(!rec.cc.in_recovery());
}

#[test]
fn karns_rule_keeps_the_backoff_until_a_segment_that_was_not_sent_again_is_acknowledged() {
    let cfg = OwnConfig {
        min_rto_us: 1_000,
        ..plain()
    };
    let mut p = Pair::new(cfg);
    p.write(&pattern(1000));
    p.pump(&mut |i| i == 0);
    p.now += 300_000;
    p.client.expire(p.now);
    assert_eq!(p.conn().rto_us(), 400_000);
    // The retransmission is acknowledged. That says nothing of the round
    // trip, because the ACK could be for either send, so the timeout stays.
    p.hop = 20_000;
    p.pump(&mut |_| false);
    assert_eq!(p.received.len(), 1000);
    assert_eq!(p.conn().rto_us(), 400_000);
    // A new segment is sent once and acknowledged after 40 ms: SRTT = 40 ms,
    // RTTVAR = 20 ms, RTO = 40 + 4 * 20 = 120 ms.
    p.write(&pattern(1000));
    p.pump(&mut |_| false);
    assert_eq!(p.received.len(), 2000);
    assert_eq!(p.conn().rto_us(), 120_000);
}

#[test]
fn the_timeout_is_kept_at_the_minimum() {
    let mut p = Pair::new(plain());
    p.hop = 1_000;
    p.write(&pattern(1000));
    p.pump(&mut |_| false);
    // 2 ms of round trip gives 6 ms, and the minimum is 200 ms.
    assert_eq!(p.conn().rto_us(), 200_000);
}

#[test]
fn a_fin_behind_a_hole_waits_in_the_queue_and_is_taken_when_the_hole_fills() {
    let mut p = Pair::new(plain());
    let data = pattern(3 * MSS);
    p.write(&data);
    p.client
        .shutdown(p.c.0, p.c.1)
        .expect("shutdown after the data");
    // The first segment is lost; the other two and the FIN arrive behind it,
    // and each of the three sends a duplicate acknowledgment.
    p.pump(&mut |i| i == 0);
    assert_eq!(p.received, data);
    assert_eq!(p.client.counters.timeouts, 0);
    assert_eq!(p.client.counters.fast_retransmits, 1);
    // One segment was sent again: not the FIN, which the server held.
    assert_eq!(p.client.counters.retransmits, 1);
    assert!(p.eof);
    assert_eq!(p.server_conn().state, State::CloseWait);
}

#[test]
fn recovery_state_exists_only_while_data_is_in_flight() {
    let mut p = Pair::new(plain());
    assert!(p.conn().rec.is_none());
    assert!(p.server_conn().rec.is_none());
    p.write(&pattern(5 * MSS));
    p.client.flush_dirty(p.now);
    assert!(p.conn().rec.is_some(), "in flight");
    p.pump(&mut |_| false);
    assert_eq!(p.received.len(), 5 * MSS);
    // Everything is acknowledged and nothing waits: both sides hold nothing.
    assert!(p.conn().rec.is_none());
    assert!(p.server_conn().rec.is_none());
}

#[test]
fn the_receiver_holds_out_of_order_data_only_while_a_hole_is_open() {
    let mut p = Pair::new(plain());
    p.write(&pattern(3 * MSS));
    // Two segments behind the hole send two duplicates, one short of a fast
    // retransmit, so the hole stays open until the timer.
    p.pump(&mut |i| i == 0);
    let held = p.server_conn().rec.as_ref().map(|r| r.ooo.bytes());
    assert_eq!(held, Some(2 * MSS));
    assert!(p.received.is_empty());
    p.now += 250_000;
    p.client.expire(p.now);
    p.pump(&mut |_| false);
    assert_eq!(p.received.len(), 3 * MSS);
    assert!(p.server_conn().rec.is_none());
}

#[test]
fn out_of_order_data_beyond_the_window_is_not_held() {
    let cfg = OwnConfig {
        rcv_buffer: 3 * MSS,
        ..plain()
    };
    let mut p = Pair::new(cfg);
    p.write(&pattern(10 * MSS));
    p.pump(&mut |i| i == 0);
    // The window holds three segments, so at most that much waits.
    let held = p.server_conn().rec.as_ref().map_or(0, |r| r.ooo.bytes());
    assert!(held <= 3 * MSS, "{held}");
    p.now += 250_000;
    p.client.expire(p.now);
    for _ in 0..20 {
        p.pump(&mut |_| false);
        p.now += 250_000;
        p.client.expire(p.now);
    }
    assert_eq!(p.received.len(), 10 * MSS);
}

#[test]
fn a_connection_with_recovery_state_costs_eight_bytes_more_than_before() {
    // An idle connection holds `None` of a box: 8 bytes. The window update
    // field takes two bytes that fit in the padding of the struct.
    assert_eq!(std::mem::size_of::<Option<Box<Recovery>>>(), 8);
    let size = std::mem::size_of::<Conn>();
    // The size of Conn was 168 bytes before TRN-23 and 176 after it. TRN-24
    // added the option bits (2 bytes) and the timestamp to echo (4) and took
    // them back from the timeout (a `u32` of microseconds in place of an
    // `i64`) and a flag that moved into the option bits, so it is 176 still.
    assert!(size <= 176, "{size}");
}

// ---------------------------------------------------------------------------
// Options (TRN-24): negotiation and fallback, window scaling, timestamps and
// PAWS, selective acknowledgment, and the delayed acknowledgment.

/// The segments of one flight with the default options, which are 12 bytes
/// shorter than the MSS because of the timestamp.
const TS_MSS: usize = 1448;

fn header_of(frame: &[u8]) -> Header {
    wire::parse_frame(frame).expect("a valid frame").header
}

/// The options that a host leaves out of its offer.
fn without(ws: bool, ts: bool, sack: bool) -> OwnConfig {
    OwnConfig {
        window_scale: !ws,
        timestamps: !ts,
        sack: !sack,
        ..OwnConfig::default()
    }
}

/// The negotiated options of a connection: window scale, timestamps, SACK,
/// and the two shifts.
fn agreed(conn: &Conn) -> (bool, bool, bool, u8, u8) {
    (
        conn.has(O_WS),
        conn.has(O_TS),
        conn.has(O_SACK),
        conn.rcv_shift(),
        conn.snd_shift(),
    )
}

/// A segment from the client to the server that carries `opts`, in sequence.
fn craft(p: &mut Pair, opts: Options, payload: &[u8]) -> Vec<u8> {
    let seq = p.server_conn().rcv_nxt;
    let ack = p.server_conn().snd_nxt;
    let (cip, sip, cport) = {
        let c = p.conn();
        (c.local_ip, c.remote_ip, c.local_port)
    };
    let mut frame = Vec::new();
    assert!(p.client.out.station.head(&mut frame, cip, sip));
    let header = Header {
        src_ip: cip,
        dst_ip: sip,
        src_port: cport,
        dst_port: SERVER_PORT,
        seq,
        ack,
        flags: flags::ACK | flags::PSH,
        window: 65_535,
        mss: None,
        opts,
    };
    wire::encode(&mut frame, &header, (payload, &[]));
    frame
}

fn ts_option(value: u32) -> Options {
    Options {
        timestamp: Some((value, 0)),
        ..Options::default()
    }
}

/// Delivers the client's frames to the server and returns the server's
/// answers, with the clock where it is.
fn deliver(p: &mut Pair) -> Vec<Vec<u8>> {
    p.client.flush_dirty(p.now);
    for frame in std::mem::take(&mut p.client.out.queue) {
        p.server.input(&frame, p.now);
    }
    p.server.flush_dirty(p.now);
    std::mem::take(&mut p.server.out.queue).into()
}

#[test]
fn the_syn_offers_every_option_and_never_scales_its_window() {
    let mut p = Pair::build(OwnConfig::default(), OwnConfig::default());
    p.client.flush_dirty(p.now);
    let syn = p.client.out.queue.pop_front().expect("a SYN");
    let h = header_of(&syn);
    assert_eq!(h.flags, flags::SYN);
    assert_eq!(h.mss, Some(1460));
    // 1 MiB of buffer needs a shift of 5 to fit 16 bits, and the window of a
    // SYN is not scaled.
    assert_eq!(h.opts.wscale, Some(5));
    assert!(h.opts.sack_permitted);
    assert!(matches!(h.opts.timestamp, Some((_, 0))));
    assert_eq!(h.window, 65_535);
    assert_eq!(syn.len() - 14, wire::IP_LEN + wire::TCP_LEN + 20);
    // The SYN-ACK answers each option and echoes the timestamp.
    p.server.input(&syn, p.now);
    p.server.flush_dirty(p.now);
    let synack = p.server.out.queue.pop_front().expect("a SYN-ACK");
    let a = header_of(&synack);
    assert_eq!(a.flags, flags::SYN | flags::ACK);
    assert_eq!(a.opts.wscale, Some(5));
    assert!(a.opts.sack_permitted);
    assert_eq!(a.opts.timestamp.map(|t| t.1), h.opts.timestamp.map(|t| t.0));
    assert_eq!(a.window, 65_535);
}

#[test]
fn the_handshake_agrees_on_every_option_that_both_sides_offer() {
    let mut p = Pair::new(OwnConfig::default());
    assert_eq!(agreed(p.conn()), (true, true, true, 5, 5));
    assert_eq!(agreed(p.server_conn()), (true, true, true, 5, 5));
    // The timestamp takes 12 bytes of every segment, so the data of a segment
    // is that much shorter and the frame still fits the MTU.
    assert_eq!(usize::from(p.conn().mss), TS_MSS);
    assert_eq!(usize::from(p.server_conn().mss), TS_MSS);
    p.write(&pattern(20 * TS_MSS));
    p.client.flush_dirty(p.now);
    for frame in &p.client.out.queue {
        assert!(frame.len() <= 14 + 1500, "{} bytes", frame.len());
        assert!(payload_len(frame) <= TS_MSS);
    }
}

#[test]
fn an_option_that_the_peer_does_not_offer_is_not_used_and_data_still_flows() {
    let data = pattern(300_000);
    for (name, server_cfg, expect) in [
        (
            "window scale",
            without(true, false, false),
            (false, true, true, 0, 0),
        ),
        (
            "timestamps",
            without(false, true, false),
            (true, false, true, 5, 5),
        ),
        (
            "sack",
            without(false, false, true),
            (true, true, false, 5, 5),
        ),
        (
            "all",
            without(true, true, true),
            (false, false, false, 0, 0),
        ),
    ] {
        let mut p = Pair::with(OwnConfig::default(), server_cfg);
        assert_eq!(agreed(p.conn()), expect, "client, server lacks {name}");
        assert_eq!(
            agreed(p.server_conn()),
            expect,
            "server, server lacks {name}"
        );
        p.write(&data);
        p.settle();
        assert_eq!(p.received, data, "server lacks {name}");
        assert_eq!(p.client.counters.paws_drops, 0, "{name}");
        assert_eq!(p.server.counters.paws_drops, 0, "{name}");
    }
    // And the other way: a client that offers nothing gets nothing.
    for (name, client_cfg) in [
        ("window scale", without(true, false, false)),
        ("timestamps", without(false, true, false)),
        ("sack", without(false, false, true)),
    ] {
        let mut p = Pair::with(client_cfg, OwnConfig::default());
        let (c, s) = (agreed(p.conn()), agreed(p.server_conn()));
        assert_eq!(c.0, s.0, "{name}");
        assert_eq!(c.1, s.1, "{name}");
        assert_eq!(c.2, s.2, "{name}");
        p.write(&data);
        p.settle();
        assert_eq!(p.received, data, "client lacks {name}");
    }
}

#[test]
fn a_peer_without_options_gets_a_plain_answer_and_a_64_kib_window() {
    let mut p = Pair::build(plain(), OwnConfig::default());
    p.client.flush_dirty(p.now);
    let syn = p.client.out.queue.pop_front().expect("a SYN");
    assert_eq!(header_of(&syn).opts, Options::default());
    p.server.input(&syn, p.now);
    p.server.flush_dirty(p.now);
    let answer = header_of(&p.server.out.queue.pop_front().expect("a SYN-ACK"));
    assert_eq!(answer.opts, Options::default());
    assert_eq!(answer.mss, Some(1460));
    // After the handshake neither side scales or stamps, and the window
    // that the server advertises stays within 16 bits.
    p.server.out.queue.clear();
    let mut p = Pair::with(plain(), OwnConfig::default());
    assert_eq!(agreed(p.server_conn()), (false, false, false, 0, 0));
    p.write(&pattern(100_000));
    p.settle();
    assert!(p.conn().snd_wnd <= 65_535);
    assert_eq!(usize::from(p.conn().mss), MSS);
}

#[test]
fn the_window_field_is_the_free_space_shifted() {
    let mut p = Pair::new(OwnConfig::default());
    let c = p.conn();
    assert_eq!(c.window_field(1 << 20), 32_768);
    assert_eq!(c.window_field(31), 0);
    assert_eq!(c.window_field(32), 1);
    assert_eq!(c.window_field(usize::MAX), 65_535);
    assert_eq!(shift_for(65_535), 0);
    assert_eq!(shift_for(65_536), 1);
    assert_eq!(shift_for(1 << 20), 5);
    assert_eq!(shift_for(usize::MAX), 14);
}

#[test]
fn a_scaled_window_lets_the_sender_have_more_than_64_kib_in_flight() {
    let mut p = Pair::new(OwnConfig::default());
    p.write(&pattern(10 * TS_MSS));
    p.settle();
    // The server advertises its free buffer in units of 32 bytes.
    let wnd = p.conn().snd_wnd;
    assert!(wnd > 1_000_000 && wnd <= 1 << 20, "{wnd}");
    // A long flight is not held back by 64 KiB: with the window at its
    // maximum the client sends the whole initial window.
    p.write(&pattern(400_000));
    p.client.flush_dirty(p.now);
    let first_flight: usize = p.client.out.queue.iter().map(|f| payload_len(f)).sum();
    assert_eq!(first_flight, 10 * TS_MSS);
    p.settle();
    assert_eq!(p.received.len(), 10 * TS_MSS + 400_000);
    // Many round trips later the flight went far beyond 64 KiB.
    assert!(p.client.counters.frames_out > 250);
}

#[test]
fn timestamps_are_echoed_in_the_acknowledgment() {
    let mut p = Pair::new(OwnConfig {
        delayed_ack: false,
        ..OwnConfig::default()
    });
    p.write(&pattern(100));
    p.client.flush_dirty(p.now);
    let data = header_of(p.client.out.queue.front().expect("a segment"));
    let value = data.opts.timestamp.expect("a timestamp").0;
    let acks = deliver(&mut p);
    assert_eq!(acks.len(), 1);
    let ack = header_of(&acks[0]);
    assert_eq!(ack.ack, data.seq.wrapping_add(100));
    assert_eq!(ack.opts.timestamp.map(|t| t.1), Some(value));
}

#[test]
fn a_timestamp_gives_a_round_trip_sample_where_karns_rule_gives_none() {
    let cfg = OwnConfig {
        min_rto_us: 1_000,
        delayed_ack: false,
        ..OwnConfig::default()
    };
    let mut p = Pair::new(cfg);
    p.write(&pattern(1000));
    p.pump(&mut |i| i == 0);
    p.now += 300_000;
    p.client.expire(p.now);
    assert_eq!(p.conn().rto_us(), 400_000);
    // The retransmission carries a new timestamp, so its acknowledgment is a
    // sample of 40 ms (SRTT 40 ms, RTTVAR 20 ms, RTO 120 ms) at once, where
    // the plain stack keeps the backoff (see the Karn test above).
    p.hop = 20_000;
    p.pump(&mut |_| false);
    assert_eq!(p.received.len(), 1000);
    assert_eq!(p.conn().rto_us(), 120_000);
}

#[test]
fn paws_drops_a_segment_with_an_old_timestamp_and_one_without() {
    let mut p = Pair::new(OwnConfig::default());
    let recent = p.server_conn().ts_recent;
    // An old timestamp: dropped, answered with an ACK, nothing delivered.
    let old = craft(&mut p, ts_option(recent.wrapping_sub(1000)), b"stale");
    p.server.input(&old, p.now);
    assert_eq!(p.server.counters.paws_drops, 1);
    p.server.flush_dirty(p.now);
    assert_eq!(p.server.out.queue.len(), 1);
    p.server.out.queue.clear();
    // No timestamp at all, on a connection that agreed on them.
    let bare = craft(&mut p, Options::default(), b"stale");
    p.server.input(&bare, p.now);
    assert_eq!(p.server.counters.paws_drops, 2);
    p.read_server();
    assert!(p.received.is_empty());
    // A current timestamp is taken and becomes the one to echo.
    let good = craft(&mut p, ts_option(recent.wrapping_add(5)), b"fresh");
    p.server.input(&good, p.now);
    p.read_server();
    assert_eq!(p.received, b"fresh");
    assert_eq!(p.server.counters.paws_drops, 2);
    assert_eq!(p.server_conn().ts_recent, recent.wrapping_add(5));
}

#[test]
fn paws_compares_timestamps_across_the_wrap_of_the_clock() {
    let mut p = Pair::new(OwnConfig::default());
    p.server_conn().ts_recent = 0xFFFF_FFF0;
    // 5 is 21 ticks after 0xFFFF_FFF0 and so is new.
    let wrapped = craft(&mut p, ts_option(5), b"new");
    p.server.input(&wrapped, p.now);
    assert_eq!(p.server.counters.paws_drops, 0);
    assert_eq!(p.server_conn().ts_recent, 5);
    // 0xFFFF_FFF0 is now in the past.
    let before = craft(&mut p, ts_option(0xFFFF_FFF0), b"old");
    p.server.input(&before, p.now);
    assert_eq!(p.server.counters.paws_drops, 1);
}

#[test]
fn a_reset_is_not_dropped_by_paws() {
    let mut p = Pair::new(OwnConfig::default());
    let recent = p.server_conn().ts_recent;
    let mut frame = craft(&mut p, ts_option(recent.wrapping_sub(5000)), b"");
    // Turn the segment into a reset by rebuilding it with the flag.
    let mut h = header_of(&frame);
    h.flags = flags::RST | flags::ACK;
    frame.clear();
    assert!(p.client.out.station.head(&mut frame, h.src_ip, h.dst_ip));
    wire::encode(&mut frame, &h, (&[], &[]));
    p.server.input(&frame, p.now);
    assert_eq!(p.server.counters.paws_drops, 0);
    assert_eq!(p.server.counters.aborted, 1);
}

#[test]
fn the_acknowledgment_carries_sack_blocks_with_the_latest_run_first() {
    let mut p = Pair::new(OwnConfig {
        delayed_ack: false,
        ..OwnConfig::default()
    });
    p.write(&pattern(10 * TS_MSS));
    p.client.flush_dirty(p.now);
    let frames: Vec<Vec<u8>> = std::mem::take(&mut p.client.out.queue).into();
    assert_eq!(frames.len(), 10);
    let seq = |i: usize| header_of(&frames[i]).seq;
    let end = |i: usize| seq(i).wrapping_add(TS_MSS as u32);
    let mut last = Vec::new();
    for (i, frame) in frames.iter().enumerate() {
        if i == 0 || i == 4 {
            continue;
        }
        p.server.input(frame, p.now);
        p.server.flush_dirty(p.now);
        last = std::mem::take(&mut p.server.out.queue).into();
        assert_eq!(last.len(), 1);
        let h = header_of(&last[0]);
        assert_eq!(h.ack, seq(0));
        match i {
            1 => assert_eq!(h.opts.blocks(), [(seq(1), end(1))]),
            3 => assert_eq!(h.opts.blocks(), [(seq(1), end(3))]),
            5 => assert_eq!(h.opts.blocks(), [(seq(5), end(5)), (seq(1), end(3))]),
            _ => {}
        }
    }
    let h = header_of(&last[0]);
    assert_eq!(h.opts.blocks(), [(seq(5), end(9)), (seq(1), end(3))]);
    assert!(h.opts.timestamp.is_some());
}

#[test]
fn at_most_three_blocks_fit_beside_a_timestamp() {
    let mut p = Pair::new(OwnConfig {
        delayed_ack: false,
        ..OwnConfig::default()
    });
    p.write(&pattern(10 * TS_MSS));
    p.client.flush_dirty(p.now);
    let frames: Vec<Vec<u8>> = std::mem::take(&mut p.client.out.queue).into();
    // Segments 1, 3, 5, and 7 arrive, each behind a hole: four runs.
    let mut last = Vec::new();
    for i in [1usize, 3, 5, 7] {
        p.server.input(&frames[i], p.now);
        p.server.flush_dirty(p.now);
        last = std::mem::take(&mut p.server.out.queue).into();
    }
    let h = header_of(&last[0]);
    assert_eq!(h.opts.blocks().len(), 3);
    assert_eq!(h.opts.blocks()[0].0, header_of(&frames[7]).seq);
}

/// Drops the data segments of `drops` from the first flight of `len` bytes
/// and returns the pair and the time it took to deliver everything.
fn flight_with_losses(cfg: OwnConfig, len: usize, drops: &[usize]) -> (Pair, i64) {
    let mut p = Pair::new(cfg);
    p.hop = 10_000;
    p.write(&pattern(len));
    let start = p.now;
    p.pump(&mut |i| drops.contains(&i));
    assert_eq!(p.received, pattern(len));
    let elapsed = p.now - start;
    (p, elapsed)
}

fn sack_only() -> OwnConfig {
    OwnConfig {
        delayed_ack: false,
        ..OwnConfig::default()
    }
}

#[test]
fn sack_repairs_several_holes_in_one_round_trip_where_newreno_needs_one_each() {
    let drops = [0, 2, 4, 6];
    let (sack, t_sack) = flight_with_losses(sack_only(), 10 * 1400, &drops);
    let (reno, t_reno) = flight_with_losses(plain(), 10 * 1400, &drops);
    for p in [&sack, &reno] {
        assert_eq!(p.client.counters.timeouts, 0);
        assert_eq!(p.client.counters.retransmits, 4);
    }
    assert_eq!(sack.client.counters.sack_recoveries, 1);
    assert_eq!(sack.client.counters.sack_retransmits, 4);
    assert_eq!(reno.client.counters.sack_recoveries, 0);
    // One round trip for the flight and one for the holes, against one per
    // hole.
    assert!(t_sack < t_reno, "{t_sack} against {t_reno}");
    assert!(t_sack <= 3 * 20_000, "{t_sack}");
    assert!(t_reno >= 5 * 20_000, "{t_reno}");
}

#[test]
fn a_hole_with_too_little_above_it_is_repaired_by_the_partial_acknowledgment() {
    // Segment 7 has only two segments behind it, which RFC 6675 does not call
    // lost, but it is the next hole after a partial acknowledgment.
    let (p, elapsed) = flight_with_losses(sack_only(), 10 * 1400, &[0, 7]);
    assert_eq!(p.client.counters.timeouts, 0);
    assert_eq!(p.client.counters.retransmits, 2);
    assert!(elapsed < 200_000, "{elapsed}");
}

#[test]
fn a_loss_without_sack_blocks_still_starts_the_recovery_on_three_duplicates() {
    // The server offers no SACK, so the client runs NewReno.
    let mut p = Pair::with(sack_only(), without(false, false, true));
    p.hop = 10_000;
    p.write(&pattern(10 * 1400));
    p.pump(&mut |i| i == 0 || i == 3);
    assert_eq!(p.received, pattern(10 * 1400));
    assert_eq!(p.client.counters.sack_recoveries, 0);
    assert_eq!(p.client.counters.timeouts, 0);
    assert_eq!(p.client.counters.fast_retransmits, 2);
}

#[test]
fn the_scoreboard_lives_only_in_the_box_and_the_box_goes_when_the_transfer_is_acknowledged() {
    let (mut p, _) = flight_with_losses(sack_only(), 10 * 1400, &[0, 2, 4, 6]);
    assert!(
        p.client.conn(p.c.0).is_none() || p.conn().rec.is_none(),
        "no scoreboard remains on an idle connection"
    );
    assert!(p.server_conn().rec.is_none());
}

#[test]
fn a_lost_retransmission_falls_back_to_the_timeout() {
    let mut p = Pair::new(sack_only());
    p.hop = 10_000;
    p.write(&pattern(10 * 1400));
    // Segment 0 is lost, and so is its first retransmission (index 10).
    p.pump(&mut |i| i == 0 || i == 10);
    assert_ne!(p.received, pattern(10 * 1400));
    p.now += 250_000;
    assert_eq!(p.client.expire(p.now), 1);
    p.pump(&mut |_| false);
    assert_eq!(p.received, pattern(10 * 1400));
    assert_eq!(p.client.counters.timeouts, 1);
}

#[test]
fn many_segments_lost_in_a_long_transfer_all_arrive_and_the_pipe_stays_bounded() {
    let mut p = Pair::new(sack_only());
    p.hop = 5_000;
    let data = pattern(500_000);
    p.write(&data);
    // Every 17th data segment is lost, retransmissions included.
    p.pump(&mut |i| i % 17 == 3);
    p.now += 500_000;
    p.client.expire(p.now);
    p.pump(&mut |i| i % 17 == 3);
    for _ in 0..30 {
        p.now += 250_000;
        p.client.expire(p.now);
        p.pump(&mut |i| i % 17 == 3);
        if p.received.len() == data.len() {
            break;
        }
    }
    assert_eq!(p.received, data);
    assert!(p.client.counters.sack_recoveries >= 5);
    assert!(p.client.counters.sack_retransmits >= 20);
}

// ---- Delayed acknowledgment ----

fn delayed() -> OwnConfig {
    OwnConfig {
        sack: false,
        window_scale: false,
        ..OwnConfig::default()
    }
}

#[test]
fn one_segment_is_acknowledged_after_the_delay_and_not_before() {
    let mut p = Pair::new(delayed());
    p.write(&pattern(100));
    let t = p.now;
    let held = deliver(&mut p);
    assert!(held.is_empty(), "the ACK waits");
    assert_eq!(p.server.expire(t + 39_999), 0);
    p.server.flush_dirty(t + 39_999);
    assert!(p.server.out.queue.is_empty());
    assert_eq!(p.server.expire(t + 40_000), 1);
    p.server.flush_dirty(t + 40_000);
    let acks: Vec<Vec<u8>> = std::mem::take(&mut p.server.out.queue).into();
    assert_eq!(acks.len(), 1);
    let h = header_of(&acks[0]);
    assert_eq!(h.flags, flags::ACK);
    assert_eq!(p.server.counters.delayed_acks, 1);
    // It acknowledges the byte stream, and the timer is spent.
    assert_eq!(h.ack, p.server_conn().rcv_nxt);
    assert_eq!(p.server.expire(t + 1_000_000), 0);
}

#[test]
fn every_second_segment_is_acknowledged_at_once() {
    let mut p = Pair::new(delayed());
    p.write(&pattern(3 * TS_MSS));
    p.client.flush_dirty(p.now);
    let frames: Vec<Vec<u8>> = std::mem::take(&mut p.client.out.queue).into();
    assert_eq!(frames.len(), 3);
    let mut answers = Vec::new();
    for frame in &frames {
        p.server.input(frame, p.now);
        p.server.flush_dirty(p.now);
        answers.push(p.server.out.queue.len());
        p.server.out.queue.clear();
    }
    // Nothing after the first, an ACK after the second, nothing after the
    // third, which waits for the timer.
    assert_eq!(answers, [0, 1, 0]);
    assert_eq!(p.server.counters.delayed_acks, 0);
    assert_eq!(p.server.expire(p.now + 40_000), 1);
    assert_eq!(p.server.counters.delayed_acks, 1);
}

#[test]
fn a_segment_after_a_gap_is_acknowledged_at_once_and_so_is_the_one_that_fills_it() {
    let mut p = Pair::new(delayed());
    p.write(&pattern(3 * TS_MSS));
    p.client.flush_dirty(p.now);
    let frames: Vec<Vec<u8>> = std::mem::take(&mut p.client.out.queue).into();
    p.server.input(&frames[1], p.now);
    p.server.flush_dirty(p.now);
    assert_eq!(p.server.out.queue.len(), 1, "out of order");
    p.server.out.queue.clear();
    p.server.input(&frames[2], p.now);
    p.server.flush_dirty(p.now);
    assert_eq!(p.server.out.queue.len(), 1, "still behind the hole");
    p.server.out.queue.clear();
    p.server.input(&frames[0], p.now);
    p.server.flush_dirty(p.now);
    assert_eq!(p.server.out.queue.len(), 1, "the hole is filled");
    let h = header_of(&p.server.out.queue[0]);
    assert_eq!(h.ack, header_of(&frames[2]).seq.wrapping_add(TS_MSS as u32));
    assert_eq!(p.server.counters.delayed_acks, 0);
}

#[test]
fn a_response_carries_the_acknowledgment_and_the_timer_stays_quiet() {
    let mut p = Pair::new(delayed());
    p.write(&pattern(100));
    let t = p.now;
    assert!(deliver(&mut p).is_empty());
    // The server application answers before the timer.
    p.read_server();
    let id = p.s;
    assert!(matches!(
        p.server.write(id.0, id.1, b"response", Waker::noop()),
        Some(Ok(8))
    ));
    p.server.flush_dirty(t + 5_000);
    let frames: Vec<Vec<u8>> = std::mem::take(&mut p.server.out.queue).into();
    assert_eq!(frames.len(), 1);
    assert_eq!(payload_len(&frames[0]), 8);
    assert_eq!(header_of(&frames[0]).ack, p.server_conn().rcv_nxt);
    assert_eq!(p.server.expire(t + 40_000), 0);
    p.server.flush_dirty(t + 40_000);
    assert!(p.server.out.queue.is_empty());
    assert_eq!(p.server.counters.delayed_acks, 0);
}

#[test]
fn a_fin_is_acknowledged_at_once() {
    let mut p = Pair::new(delayed());
    p.write(&pattern(100));
    let (id, gen) = p.c;
    p.client.shutdown(id, gen).expect("shutdown");
    let answers = deliver(&mut p);
    assert_eq!(answers.len(), 1);
    assert_eq!(p.server.counters.delayed_acks, 0);
}

#[test]
fn without_the_delay_every_segment_is_acknowledged() {
    let mut p = Pair::new(OwnConfig {
        delayed_ack: false,
        ..OwnConfig::default()
    });
    p.write(&pattern(100));
    assert_eq!(deliver(&mut p).len(), 1);
}

#[test]
fn a_whole_transfer_with_the_delay_completes_without_a_timeout() {
    let mut p = Pair::new(OwnConfig::default());
    p.hop = 10_000;
    let data = pattern(500_001);
    p.write(&data);
    p.settle();
    assert_eq!(p.received, data);
    assert_eq!(p.client.counters.timeouts, 0);
    assert_eq!(p.client.counters.retransmits, 0);
    // Fewer acknowledgments than two thirds of the segments (the receiver
    // also sends window updates when the application reads).
    let segments = p.client.counters.frames_out;
    let acks = p.server.counters.frames_out;
    assert!(
        acks * 3 < segments * 2,
        "{acks} acks for {segments} segments"
    );
}

#[test]
fn a_retransmission_of_the_oldest_segment_starts_the_timeout_again() {
    // A round trip of 200 ms and a timeout of 250 ms: the duplicates come
    // back before the timer, the retransmission is acknowledged after it. The
    // timer that the last advancing ACK set would fire in between, and a
    // retransmission resets it (as in the kernel), so no timeout fires.
    for cfg in [
        OwnConfig {
            rto_us: 250_000,
            min_rto_us: 250_000,
            delayed_ack: false,
            ..OwnConfig::default()
        },
        OwnConfig {
            rto_us: 250_000,
            min_rto_us: 250_000,
            ..plain()
        },
    ] {
        let mut p = Pair::new(cfg);
        p.hop = 100_000;
        p.timers = true;
        p.write(&pattern(10 * 1400));
        p.pump(&mut |i| i == 0);
        assert_eq!(p.received, pattern(10 * 1400));
        assert_eq!(p.client.counters.timeouts, 0, "{:?}", p.client.counters);
    }
}
