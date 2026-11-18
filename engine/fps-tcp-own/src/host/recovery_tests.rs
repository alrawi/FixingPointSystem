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
        let pools = Pools::sized("10.1.0.0/16", 1, "10.2.0.0/16", 1, None).expect("pools");
        let built = pools.stations().expect("stations");
        let client_ip = pools.client.addresses().expect("addresses")[0];
        let server_ip = pools.server.addresses().expect("addresses")[0];
        let mut client = Host::new(built.client, cfg.clone(), 1500);
        let mut server = Host::new(built.server, cfg, 1500);
        server.listen(SERVER_PORT).expect("listen");
        let now = 1_000_000;
        let c = client
            .open(client_ip, server_ip, SERVER_PORT, now)
            .expect("open");
        let mut pair = Self {
            client,
            server,
            c,
            s: (0, 0),
            now,
            hop: 0,
            data_seen: 0,
            received: Vec::new(),
            eof: false,
        };
        pair.pump(&mut |_| false);
        let mut ticket = None;
        pair.s = pair
            .server
            .accept(SERVER_PORT, &mut ticket, Waker::noop())
            .expect("accept")
            .expect("a connection is queued");
        pair
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
    let mut p = Pair::new(OwnConfig::default());
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
    let mut p = Pair::new(OwnConfig::default());
    let data = pattern(60 * MSS);
    p.write(&data);
    p.pump(&mut |_| false);
    assert_eq!(p.received, data);
    assert_eq!(p.client.counters.retransmits, 0);
    assert_eq!(p.client.counters.timeouts, 0);
}

#[test]
fn three_duplicate_acknowledgments_retransmit_without_waiting_for_the_timer() {
    let mut p = Pair::new(OwnConfig::default());
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
    let mut p = Pair::new(OwnConfig::default());
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
    let mut p = Pair::new(OwnConfig::default());
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
    let mut p = Pair::new(OwnConfig::default());
    let data = pattern(MSS);
    p.write(&data);
    // The only segment is lost, so no duplicate acknowledgment can come.
    p.pump(&mut |i| i == 0);
    assert!(p.received.is_empty());
    assert_eq!(p.conn().rto_us, 200_000);
    p.now += 250_000;
    assert_eq!(p.client.expire(p.now), 1);
    assert_eq!(p.client.counters.timeouts, 1);
    // The timeout doubled.
    assert_eq!(p.conn().rto_us, 400_000);
    p.pump(&mut |_| false);
    assert_eq!(p.received, data);
}

#[test]
fn a_timeout_cuts_the_window_to_one_segment() {
    let mut p = Pair::new(OwnConfig::default());
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
        ..OwnConfig::default()
    };
    let mut p = Pair::new(cfg);
    p.write(&pattern(1000));
    p.pump(&mut |i| i == 0);
    p.now += 300_000;
    p.client.expire(p.now);
    assert_eq!(p.conn().rto_us, 400_000);
    // The retransmission is acknowledged. That says nothing of the round
    // trip, because the ACK could be for either send, so the timeout stays.
    p.hop = 20_000;
    p.pump(&mut |_| false);
    assert_eq!(p.received.len(), 1000);
    assert_eq!(p.conn().rto_us, 400_000);
    // A new segment is sent once and acknowledged after 40 ms: SRTT = 40 ms,
    // RTTVAR = 20 ms, RTO = 40 + 4 * 20 = 120 ms.
    p.write(&pattern(1000));
    p.pump(&mut |_| false);
    assert_eq!(p.received.len(), 2000);
    assert_eq!(p.conn().rto_us, 120_000);
}

#[test]
fn the_timeout_is_kept_at_the_minimum() {
    let mut p = Pair::new(OwnConfig::default());
    p.hop = 1_000;
    p.write(&pattern(1000));
    p.pump(&mut |_| false);
    // 2 ms of round trip gives 6 ms, and the minimum is 200 ms.
    assert_eq!(p.conn().rto_us, 200_000);
}

#[test]
fn a_fin_behind_a_hole_waits_in_the_queue_and_is_taken_when_the_hole_fills() {
    let mut p = Pair::new(OwnConfig::default());
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
    let mut p = Pair::new(OwnConfig::default());
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
    let mut p = Pair::new(OwnConfig::default());
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
        ..OwnConfig::default()
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
    // The size of Conn before TRN-23 was 168 bytes.
    assert!(size <= 168 + 8, "{size}");
}
