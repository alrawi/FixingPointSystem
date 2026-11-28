// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of the own stack through its `SessionIo` interface, over
//! the in memory frame link and over a link that loses, corrupts, and
//! reorders frames.

use std::io;
use std::io::ErrorKind;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use fps_backend_afxdp::memory::{frame_pair, MemoryFrames};
use fps_backend_afxdp::{FrameIo, QueueStats};
use fps_io::SessionIo;
use fps_tcp_own::clock::{Clock, ManualClock};
use fps_tcp_own::net::decode_header;
use fps_tcp_own::wire::{self, flags};
use fps_tcp_own::{OsProfile, OwnConfig, OwnSession, OwnStack, Pools};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;

const PORT: u16 = 8080;

/// What a link does with one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fate {
    Deliver,
    Drop,
    Corrupt,
    /// Hold the frame back and deliver it after the next one.
    Delay,
}

type Rule = Box<dyn FnMut(&[u8]) -> Fate + Send>;

/// One end of a link that records and judges every frame that it sends.
struct FaultyLink {
    inner: MemoryFrames,
    rule: Rule,
    held: Option<Vec<u8>>,
    log: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl FaultyLink {
    fn new(inner: MemoryFrames, rule: Rule, log: Arc<Mutex<Vec<Vec<u8>>>>) -> Self {
        Self {
            inner,
            rule,
            held: None,
            log,
        }
    }
}

impl FrameIo for FaultyLink {
    fn queue_id(&self) -> u32 {
        self.inner.queue_id()
    }
    fn max_frame_len(&self) -> usize {
        self.inner.max_frame_len()
    }
    fn recv_batch(&mut self, max: usize, frame: &mut dyn FnMut(&[u8])) -> io::Result<usize> {
        self.inner.recv_batch(max, frame)
    }
    fn send_batch_with(
        &mut self,
        count: usize,
        fill: &mut dyn FnMut(usize, &mut [u8]) -> usize,
    ) -> io::Result<usize> {
        let mut buffer = vec![0u8; self.inner.max_frame_len()];
        let mut queued = 0;
        for index in 0..count {
            let len = fill(index, &mut buffer);
            if len == 0 || len > buffer.len() {
                break;
            }
            let frame = &mut buffer[..len];
            self.log
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(frame.to_vec());
            let fate = (self.rule)(frame);
            match fate {
                Fate::Drop => {}
                Fate::Delay => {
                    if self.held.is_none() {
                        self.held = Some(frame.to_vec());
                    } else {
                        self.inner.send_batch(&[&*frame])?;
                    }
                }
                Fate::Corrupt | Fate::Deliver => {
                    if fate == Fate::Corrupt {
                        let last = frame.len() - 1;
                        frame[last] ^= 0x01;
                    }
                    self.inner.send_batch(&[&*frame])?;
                    if let Some(held) = self.held.take() {
                        self.inner.send_batch(&[&held])?;
                    }
                }
            }
            queued += 1;
        }
        Ok(queued)
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

fn deliver_all() -> Rule {
    Box::new(|_| Fate::Deliver)
}

type Log = Arc<Mutex<Vec<Vec<u8>>>>;

/// A stack over a faulty link, with a clock that a test drives, and the logs
/// of the frames that the client and the server sent.
struct Lab {
    stack: OwnStack,
    clock: Arc<ManualClock>,
    client_log: Log,
    server_log: Log,
}

fn lab(config: OwnConfig, client_rule: Rule, server_rule: Rule) -> Lab {
    lab_pools(config, Pools::pair(), client_rule, server_rule)
}

fn lab_pools(config: OwnConfig, pools: Pools, client_rule: Rule, server_rule: Rule) -> Lab {
    let (client, server) = frame_pair(config.link_depth, config.mtu + 18);
    let (client_log, server_log) = (Log::default(), Log::default());
    let clock = Arc::new(ManualClock::new());
    let stack = OwnStack::with_links_and_pools(
        Box::new(FaultyLink::new(
            client,
            client_rule,
            Arc::clone(&client_log),
        )),
        Box::new(FaultyLink::new(
            server,
            server_rule,
            Arc::clone(&server_log),
        )),
        config,
        pools,
        clock.clone(),
    )
    .expect("pools");
    Lab {
        stack,
        clock,
        client_log,
        server_log,
    }
}

impl Lab {
    /// Pumps the hosts and moves the clock by 10 ms per round, so timers
    /// fire in virtual time while the test body awaits.
    fn start_clock_driver(&self) -> JoinHandle<()> {
        let stack = self.stack.clone();
        let clock = Arc::clone(&self.clock);
        tokio::spawn(async move {
            loop {
                stack.pump();
                clock.advance_micros(10_000);
                tokio::task::yield_now().await;
            }
        })
    }
}

fn start_driver(stack: &OwnStack) -> JoinHandle<()> {
    let driver = stack.clone();
    tokio::spawn(async move { driver.drive().await })
}

async fn within<F: std::future::Future>(body: F) -> F::Output {
    match tokio::time::timeout(Duration::from_secs(30), body).await {
        Ok(output) => output,
        Err(_) => panic!("the test did not finish within 30 seconds"),
    }
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

async fn pair(stack: &OwnStack) -> (OwnSession, OwnSession) {
    let (client, server) = tokio::join!(stack.connect(PORT), stack.accept(PORT));
    (client.expect("connect"), server.expect("accept"))
}

fn parsed(log: &Log) -> Vec<wire::Header> {
    log.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter_map(|f| wire::parse_frame(f).ok().map(|p| p.header))
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn the_handshake_is_syn_syn_ack_ack_with_matching_numbers() {
    let lab = lab(OwnConfig::default(), deliver_all(), deliver_all());
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        let (client, server) = pair(&lab.stack).await;
        let client_frames = parsed(&lab.client_log);
        let server_frames = parsed(&lab.server_log);
        let syn = client_frames[0];
        let syn_ack = server_frames[0];
        let ack = client_frames[1];
        assert_eq!(syn.flags, flags::SYN);
        assert_eq!(syn.mss, Some(1460));
        assert_eq!(syn_ack.flags, flags::SYN | flags::ACK);
        assert_eq!(syn_ack.ack, syn.seq.wrapping_add(1));
        assert_eq!(syn_ack.mss, Some(1460));
        assert_eq!(ack.flags, flags::ACK);
        assert_eq!(ack.seq, syn.seq.wrapping_add(1));
        assert_eq!(ack.ack, syn_ack.seq.wrapping_add(1));
        drop((client, server));
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn each_connection_gets_its_own_initial_sequence_number() {
    let lab = lab(OwnConfig::default(), deliver_all(), deliver_all());
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        let mut held = Vec::new();
        for _ in 0..8 {
            held.push(pair(&lab.stack).await);
        }
        let client_isns: Vec<u32> = parsed(&lab.client_log)
            .iter()
            .filter(|h| h.flags == flags::SYN)
            .map(|h| h.seq)
            .collect();
        let server_isns: Vec<u32> = parsed(&lab.server_log)
            .iter()
            .filter(|h| h.flags == flags::SYN | flags::ACK)
            .map(|h| h.seq)
            .collect();
        for isns in [client_isns, server_isns] {
            let mut sorted = isns.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(isns.len(), 8);
            assert_eq!(sorted.len(), 8, "repeated initial sequence numbers");
        }
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn a_request_and_a_response_round_trip() {
    let stack = OwnStack::default();
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    within(async {
        let (mut client, mut server) = pair(&stack).await;
        client.write_all(b"hello server").await.expect("write");
        let mut buf = [0u8; 12];
        server.read_exact(&mut buf).await.expect("read");
        assert_eq!(&buf, b"hello server");
        server.write_all(b"hello client").await.expect("write");
        client.read_exact(&mut buf).await.expect("read");
        assert_eq!(&buf, b"hello client");
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn bulk_data_arrives_intact_in_both_directions_across_many_segments() {
    let stack = OwnStack::default();
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    within(async {
        let (mut client, mut server) = pair(&stack).await;
        let up = pattern(1_000_003);
        let down = pattern(400_009);
        let (up_copy, down_copy) = (up.clone(), down.clone());
        let writer = tokio::spawn(async move {
            client.write_all(&up_copy).await.expect("write");
            let mut got = vec![0u8; down_copy.len()];
            client.read_exact(&mut got).await.expect("read");
            assert_eq!(got, down_copy);
            client
        });
        let mut got = vec![0u8; up.len()];
        server.read_exact(&mut got).await.expect("read");
        assert_eq!(got, up);
        server.write_all(&down).await.expect("write");
        let _client = writer.await.expect("task");
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn a_small_receive_window_slows_the_sender_without_losing_data() {
    let config = OwnConfig {
        rcv_buffer: 3000,
        ..OwnConfig::default()
    };
    let stack = OwnStack::new(config);
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    within(async {
        let (mut client, mut server) = pair(&stack).await;
        let data = pattern(200_000);
        let copy = data.clone();
        let writer = tokio::spawn(async move {
            client.write_all(&copy).await.expect("write");
            client.shutdown().await.expect("shutdown");
            client
        });
        // A slow reader: 500 bytes per call with a pause, so the window closes.
        let mut got = Vec::new();
        let mut buf = [0u8; 500];
        loop {
            let n = server.read(&mut buf).await.expect("read");
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
            if got.len() % 20_000 < 500 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
        assert_eq!(got, data);
        let _client = writer.await.expect("task");
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn a_full_window_is_reopened_by_a_window_update_and_not_by_a_timeout() {
    let config = OwnConfig {
        rcv_buffer: 2000,
        ..OwnConfig::default()
    };
    let lab = lab(config, deliver_all(), deliver_all());
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        let (mut client, mut server) = pair(&lab.stack).await;
        client.write_all(&pattern(10_000)).await.expect("write");
        // Let the window fill, then read everything.
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        let mut got = vec![0u8; 10_000];
        server.read_exact(&mut got).await.expect("read");
        assert_eq!(got, pattern(10_000));
        assert_eq!(lab.stack.counters().client.retransmits, 0);
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_sends_a_fin_after_the_data_and_the_peer_reads_to_the_end() {
    let stack = OwnStack::default();
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    within(async {
        let (mut client, mut server) = pair(&stack).await;
        client.write_all(&pattern(5000)).await.expect("write");
        client.shutdown().await.expect("shutdown");
        let mut got = Vec::new();
        server.read_to_end(&mut got).await.expect("read");
        assert_eq!(got, pattern(5000));
        // Half close: the server still sends, and the client still reads.
        server.write_all(b"after your FIN").await.expect("write");
        server.shutdown().await.expect("shutdown");
        let mut reply = Vec::new();
        client.read_to_end(&mut reply).await.expect("read");
        assert_eq!(reply, b"after your FIN");
        // Writing after a shutdown fails.
        let error = client.write_all(b"x").await.expect_err("closed");
        assert_eq!(error.kind(), ErrorKind::BrokenPipe);
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn both_sides_close_and_the_tuple_leaves_time_wait_after_the_hold() {
    let config = OwnConfig {
        time_wait_us: 1_000_000,
        ..OwnConfig::default()
    };
    let lab = lab(config, deliver_all(), deliver_all());
    lab.stack.listen(PORT).expect("listen");
    let driver = start_driver(&lab.stack);
    within(async {
        let (mut client, mut server) = pair(&lab.stack).await;
        let pump = |rounds: u32| {
            for _ in 0..rounds {
                lab.stack.pump();
            }
        };
        client.shutdown().await.expect("shutdown");
        pump(2);
        let mut rest = Vec::new();
        server.read_to_end(&mut rest).await.expect("read");
        server.shutdown().await.expect("shutdown");
        pump(2);
        let mut rest = Vec::new();
        client.read_to_end(&mut rest).await.expect("read");
        drop((client, server));
        pump(2);
        // The closer holds a TIME-WAIT connection, and the other side is gone.
        assert_eq!(lab.stack.stats().sockets_open, 2, "listener and TIME-WAIT");
        lab.clock.advance_micros(500_000);
        pump(1);
        assert_eq!(lab.stack.stats().sockets_open, 2, "still held");
        lab.clock.advance_micros(600_000);
        pump(1);
        assert_eq!(lab.stack.stats().sockets_open, 1, "only the listener");
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn abort_resets_the_peer_and_ends_later_operations() {
    let stack = OwnStack::default();
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    within(async {
        let (mut client, mut server) = pair(&stack).await;
        client.write_all(b"unsent or unread").await.expect("write");
        client.abort();
        client.abort();
        let error = client.write_all(b"x").await.expect_err("aborted");
        assert_eq!(error.kind(), ErrorKind::NotConnected);
        let mut buf = [0u8; 64];
        let error = server.read(&mut buf).await.expect_err("reset");
        assert_eq!(error.kind(), ErrorKind::ConnectionReset);
        // The error is sticky and the write fails the same way.
        let error = server.write_all(b"x").await.expect_err("reset");
        assert_eq!(error.kind(), ErrorKind::ConnectionReset);
    })
    .await;
    driver.abort();
}

/// A client that connects and resets before the server accepts, as the
/// churned connections of `strike/conn_churn` do (bug #689). A kernel keeps
/// the connection in the accept queue: `accept` returns it and the first read
/// reports the reset. `drop_ack` drops the plain ACK that completes the
/// handshake, so the reset (which carries the ACK field) is the first segment
/// that completes it.
async fn reset_before_accept(drop_ack: bool, connections: usize) {
    let lab = lab(
        OwnConfig::default(),
        Box::new(move |frame| {
            let Ok(parsed) = wire::parse_frame(frame) else {
                return Fate::Deliver;
            };
            let f = parsed.header.flags;
            if drop_ack && f & (flags::SYN | flags::RST | flags::FIN) == 0 && f & flags::ACK != 0 {
                return Fate::Drop;
            }
            Fate::Deliver
        }),
        deliver_all(),
    );
    lab.stack.listen(PORT).expect("listen");
    let driver = start_driver(&lab.stack);
    within(async {
        for _ in 0..connections {
            let mut client = lab.stack.connect(PORT).await.expect("connect");
            client.abort();
        }
        for index in 0..connections {
            let mut server = tokio::time::timeout(Duration::from_secs(5), lab.stack.accept(PORT))
                .await
                .unwrap_or_else(|_| {
                    panic!("accept {index} timed out: the reset connection was dropped")
                })
                .expect("accept");
            let mut buf = [0u8; 8];
            let error = server.read(&mut buf).await.expect_err("reset");
            assert_eq!(error.kind(), ErrorKind::ConnectionReset);
        }
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn a_connection_reset_after_the_handshake_stays_in_the_accept_queue() {
    reset_before_accept(false, 1).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_reset_that_carries_the_last_ack_of_the_handshake_stays_in_the_accept_queue() {
    reset_before_accept(true, 1).await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_hundred_connections_reset_before_accept_are_all_accepted() {
    reset_before_accept(true, 100).await;
}

/// A server that accepts and resets before the connecting task looks at its
/// socket (the churned connections of `strike/conn_churn`, bug #689). The
/// connect of a kernel has succeeded at the SYN-ACK, and the first read fails
/// with the reset, so `connect` must not report the reset.
#[tokio::test(flavor = "current_thread")]
async fn a_connect_that_is_reset_right_after_the_handshake_succeeds_and_the_first_read_fails() {
    let stack = OwnStack::default();
    stack.listen(PORT).expect("listen");
    within(async {
        let connect = tokio::spawn({
            let stack = stack.clone();
            async move { stack.connect(PORT).await }
        });
        // The connect task sends its SYN, and the frames of the handshake
        // move, without the task looking at its socket again.
        tokio::task::yield_now().await;
        stack.pump();
        let mut server = stack.accept(PORT).await.expect("accept");
        server.abort();
        stack.pump();
        let mut client = connect.await.expect("task").expect("connect succeeds");
        let mut buf = [0u8; 8];
        let error = client.read(&mut buf).await.expect_err("reset");
        assert_eq!(error.kind(), ErrorKind::ConnectionReset);
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn a_server_abort_resets_the_client() {
    let stack = OwnStack::default();
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    within(async {
        let (mut client, mut server) = pair(&stack).await;
        server.abort();
        let mut buf = [0u8; 8];
        let error = client.read(&mut buf).await.expect_err("reset");
        assert_eq!(error.kind(), ErrorKind::ConnectionReset);
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn a_connection_to_a_port_without_a_listener_is_refused() {
    let stack = OwnStack::default();
    let driver = start_driver(&stack);
    within(async {
        let error = stack.connect(9999).await.expect_err("refused");
        assert_eq!(error.kind(), ErrorKind::ConnectionRefused);
        assert_eq!(stack.counters().server.resets_sent, 1);
        assert_eq!(stack.stats().sockets_open, 0);
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_a_session_with_unread_data_resets_and_without_closes_cleanly() {
    let stack = OwnStack::default();
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    within(async {
        let (mut client, server) = pair(&stack).await;
        client.write_all(b"never read").await.expect("write");
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        drop(server);
        let mut buf = [0u8; 8];
        let error = client.read(&mut buf).await.expect_err("reset");
        assert_eq!(error.kind(), ErrorKind::ConnectionReset);

        let (mut client, server) = pair(&stack).await;
        drop(server);
        let mut buf = [0u8; 8];
        assert_eq!(client.read(&mut buf).await.expect("eof"), 0);
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn a_port_is_listened_on_once_and_accept_needs_a_listener() {
    let stack = OwnStack::default();
    stack.listen(PORT).expect("listen");
    assert_eq!(
        stack.listen(PORT).expect_err("twice").kind(),
        ErrorKind::AddrInUse
    );
    assert_eq!(
        stack.accept(1).await.expect_err("none").kind(),
        ErrorKind::InvalidInput
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_lost_syn_and_a_lost_syn_ack_are_sent_again() {
    let mut seen_syn = false;
    let mut seen_syn_ack = false;
    let client_rule: Rule = Box::new(move |frame| {
        let h = wire::parse_frame(frame).expect("frame").header;
        if h.flags == flags::SYN && !seen_syn {
            seen_syn = true;
            return Fate::Drop;
        }
        Fate::Deliver
    });
    let server_rule: Rule = Box::new(move |frame| {
        let h = wire::parse_frame(frame).expect("frame").header;
        if h.flags == flags::SYN | flags::ACK && !seen_syn_ack {
            seen_syn_ack = true;
            return Fate::Drop;
        }
        Fate::Deliver
    });
    let lab = lab(OwnConfig::default(), client_rule, server_rule);
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        let (mut client, mut server) = pair(&lab.stack).await;
        client.write_all(b"ping").await.expect("write");
        let mut buf = [0u8; 4];
        server.read_exact(&mut buf).await.expect("read");
        let counters = lab.stack.counters();
        assert!(counters.client.retransmits >= 1, "{counters:?}");
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn lost_data_segments_are_sent_again_and_arrive_in_order() {
    // The client loses every seventh frame that carries data.
    let mut data_frames = 0u32;
    let client_rule: Rule = Box::new(move |frame| {
        let parsed = wire::parse_frame(frame).expect("frame");
        if parsed.payload.is_empty() {
            return Fate::Deliver;
        }
        data_frames += 1;
        if data_frames.is_multiple_of(7) {
            Fate::Drop
        } else {
            Fate::Deliver
        }
    });
    let lab = lab(OwnConfig::default(), client_rule, deliver_all());
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        let (mut client, mut server) = pair(&lab.stack).await;
        let data = pattern(300_001);
        let copy = data.clone();
        let writer = tokio::spawn(async move {
            client.write_all(&copy).await.expect("write");
            client.shutdown().await.expect("shutdown");
            client
        });
        let mut got = Vec::new();
        server.read_to_end(&mut got).await.expect("read");
        assert_eq!(got, data);
        let _client = writer.await.expect("task");
        assert!(lab.stack.counters().client.retransmits > 0);
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn a_lost_fin_is_sent_again() {
    let mut dropped = false;
    let client_rule: Rule = Box::new(move |frame| {
        let h = wire::parse_frame(frame).expect("frame").header;
        if h.flags & flags::FIN != 0 && !dropped {
            dropped = true;
            return Fate::Drop;
        }
        Fate::Deliver
    });
    let lab = lab(OwnConfig::default(), client_rule, deliver_all());
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        let (mut client, mut server) = pair(&lab.stack).await;
        client.write_all(b"last words").await.expect("write");
        client.shutdown().await.expect("shutdown");
        let mut got = Vec::new();
        server.read_to_end(&mut got).await.expect("read");
        assert_eq!(got, b"last words");
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn a_frame_that_arrives_late_is_dropped_and_sent_again() {
    // Every ninth data frame is delayed behind the next one, which the
    // receiver sees as a gap and drops, so only the retransmission delivers.
    let mut data_frames = 0u32;
    let client_rule: Rule = Box::new(move |frame| {
        let parsed = wire::parse_frame(frame).expect("frame");
        if parsed.payload.is_empty() {
            return Fate::Deliver;
        }
        data_frames += 1;
        if data_frames.is_multiple_of(9) {
            Fate::Delay
        } else {
            Fate::Deliver
        }
    });
    let lab = lab(OwnConfig::default(), client_rule, deliver_all());
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        let (mut client, mut server) = pair(&lab.stack).await;
        let data = pattern(120_000);
        let copy = data.clone();
        let writer = tokio::spawn(async move {
            client.write_all(&copy).await.expect("write");
            client.shutdown().await.expect("shutdown");
            client
        });
        let mut got = Vec::new();
        server.read_to_end(&mut got).await.expect("read");
        assert_eq!(got, data);
        let _client = writer.await.expect("task");
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn corrupted_frames_are_counted_and_dropped_and_the_data_still_arrives() {
    let mut data_frames = 0u32;
    let client_rule: Rule = Box::new(move |frame| {
        let parsed = wire::parse_frame(frame).expect("frame");
        if parsed.payload.is_empty() {
            return Fate::Deliver;
        }
        data_frames += 1;
        if data_frames.is_multiple_of(5) {
            Fate::Corrupt
        } else {
            Fate::Deliver
        }
    });
    let lab = lab(OwnConfig::default(), client_rule, deliver_all());
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        let (mut client, mut server) = pair(&lab.stack).await;
        let data = pattern(60_000);
        let copy = data.clone();
        let writer = tokio::spawn(async move {
            client.write_all(&copy).await.expect("write");
            client.shutdown().await.expect("shutdown");
            client
        });
        let mut got = Vec::new();
        server.read_to_end(&mut got).await.expect("read");
        assert_eq!(got, data);
        let _client = writer.await.expect("task");
        assert!(lab.stack.counters().server.bad_frames > 0);
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn a_peer_that_goes_silent_ends_the_connection_with_a_timeout() {
    let config = OwnConfig {
        max_retries: 3,
        rto_us: 50_000,
        max_rto_us: 100_000,
        ..OwnConfig::default()
    };
    // After the handshake the server never answers.
    let mut frames = 0u32;
    let server_rule: Rule = Box::new(move |_| {
        frames += 1;
        if frames == 1 {
            Fate::Deliver
        } else {
            Fate::Drop
        }
    });
    let lab = lab(config, deliver_all(), server_rule);
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        let (mut client, _server) = pair(&lab.stack).await;
        client.write_all(b"anyone there").await.expect("write");
        let mut buf = [0u8; 8];
        let error = client.read(&mut buf).await.expect_err("timeout");
        assert_eq!(error.kind(), ErrorKind::TimedOut);
        assert_eq!(lab.stack.counters().client.retransmits, 3);
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn a_client_port_in_time_wait_on_the_server_can_be_reused() {
    // The server closes first, so the server holds TIME-WAIT for the tuple.
    // The client wraps its ports around and meets the old tuple again, and the
    // server accepts the new SYN because its sequence number is higher.
    let config = OwnConfig {
        time_wait_us: 60_000_000,
        ..OwnConfig::default()
    };
    let lab = lab(config, deliver_all(), deliver_all());
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        for _ in 0..(65_535 - 10_000 + 2) {
            let (mut client, mut server) = pair(&lab.stack).await;
            server.shutdown().await.expect("shutdown");
            let mut rest = Vec::new();
            client.read_to_end(&mut rest).await.expect("read");
            client.shutdown().await.expect("shutdown");
            let mut rest = Vec::new();
            server.read_to_end(&mut rest).await.expect("read");
        }
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn many_sessions_share_one_port_and_accept_in_turn() {
    let stack = OwnStack::default();
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    within(async {
        let mut clients = Vec::new();
        let mut servers = Vec::new();
        for _ in 0..300 {
            let (c, s) = pair(&stack).await;
            clients.push(c);
            servers.push(s);
        }
        assert_eq!(stack.stats().sockets_open, 601);
        for (i, client) in clients.iter_mut().enumerate() {
            client.write_all(&[i as u8; 3]).await.expect("write");
        }
        for (i, server) in servers.iter_mut().enumerate() {
            let mut buf = [0u8; 3];
            server.read_exact(&mut buf).await.expect("read");
            assert_eq!(buf, [i as u8; 3]);
        }
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn a_smaller_mtu_lowers_the_mss_and_still_moves_data() {
    let config = OwnConfig {
        mtu: 576,
        ..OwnConfig::default()
    };
    let lab = lab(config, deliver_all(), deliver_all());
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        let (mut client, mut server) = pair(&lab.stack).await;
        client.write_all(&pattern(20_000)).await.expect("write");
        let mut got = vec![0u8; 20_000];
        server.read_exact(&mut got).await.expect("read");
        assert_eq!(got, pattern(20_000));
        let syn = parsed(&lab.client_log)[0];
        assert_eq!(syn.mss, Some(536));
        assert!(lab
            .client_log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .all(|f| f.len() <= 576 + 14));
    })
    .await;
    driver.abort();
}

fn frames_of(log: &Log) -> Vec<Vec<u8>> {
    log.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

#[tokio::test(flavor = "current_thread")]
async fn pools_talk_over_a_tagged_link_with_one_mac_per_host() {
    let pools = Pools::sized("10.1.0.0/16", 4, "10.2.0.0/16", 3, Some(100)).expect("pools");
    let lab = lab_pools(OwnConfig::default(), pools, deliver_all(), deliver_all());
    lab.stack.listen(PORT).expect("listen");
    assert_eq!(lab.stack.stats().client_hosts, 4);
    assert_eq!(lab.stack.stats().server_hosts, 3);
    let driver = lab.start_clock_driver();
    within(async {
        // 12 connects use each of the 4 x 3 address pairs once, on one port.
        let mut pairs = Vec::new();
        for i in 0..12u8 {
            let (client, server) = tokio::join!(lab.stack.connect(PORT), lab.stack.accept(PORT));
            pairs.push((i, client.expect("connect"), server.expect("accept")));
        }
        for (i, client, server) in &mut pairs {
            client.write_all(&[*i; 700]).await.expect("write");
            let mut got = [0u8; 700];
            server.read_exact(&mut got).await.expect("read");
            assert_eq!(got, [*i; 700]);
        }
    })
    .await;
    driver.abort();
    let mut tuples = std::collections::BTreeSet::new();
    let mut client_macs = std::collections::BTreeSet::new();
    for frame in frames_of(&lab.client_log) {
        let eth = decode_header(&frame).expect("ethernet");
        assert_eq!(eth.vlan, Some(100), "every frame carries the tag");
        assert!(frame.len() >= 60, "frames are padded to the minimum");
        client_macs.insert(eth.src);
        let h = wire::parse_frame(&frame).expect("frame").header;
        if h.flags == flags::SYN {
            tuples.insert((h.src_ip, h.dst_ip));
        }
    }
    assert_eq!(tuples.len(), 12, "each connect used its own address pair");
    assert_eq!(client_macs.len(), 4, "one MAC per client host");
    let mut server_macs = std::collections::BTreeSet::new();
    for frame in frames_of(&lab.server_log) {
        let eth = decode_header(&frame).expect("ethernet");
        assert_eq!(eth.vlan, Some(100));
        assert!(
            client_macs.contains(&eth.dst),
            "replies go to a client host"
        );
        server_macs.insert(eth.src);
    }
    assert_eq!(server_macs.len(), 3, "one MAC per server host");
    let c = lab.stack.counters();
    assert_eq!(c.client.unresolved + c.server.unresolved, 0);
    assert_eq!(c.client.link_dropped + c.server.link_dropped, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn connect_between_chosen_hosts_and_refuse_unknown_ones() {
    let pools = Pools::sized("10.1.0.0/16", 2, "10.2.0.0/16", 2, None).expect("pools");
    let lab = lab_pools(OwnConfig::default(), pools, deliver_all(), deliver_all());
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        let (client, server) =
            tokio::join!(lab.stack.connect_hosts(1, 1, PORT), lab.stack.accept(PORT));
        let _ = (client.expect("connect"), server.expect("accept"));
        let err = lab
            .stack
            .connect_hosts(2, 0, PORT)
            .await
            .expect_err("no host 2");
        assert_eq!(err.kind(), ErrorKind::AddrNotAvailable);
    })
    .await;
    driver.abort();
    let syn = frames_of(&lab.client_log)
        .iter()
        .map(|f| wire::parse_frame(f).expect("frame").header)
        .find(|h| h.flags == flags::SYN)
        .expect("syn");
    assert_eq!(syn.src_ip, wire::v4(0x0A01_0002), "client host 1");
    assert_eq!(syn.dst_ip, wire::v4(0x0A02_0002), "server host 1");
}

// ---------------------------------------------------------------------------
// IPv6 (TRN-30).

fn ip6(text: &str) -> std::net::IpAddr {
    std::net::IpAddr::V6(text.parse().expect("an IPv6 address"))
}

/// Every frame of `log` is an IPv6 frame, and the Ethernet type says so.
fn assert_all_ipv6(log: &Log) {
    let frames = frames_of(log);
    assert!(!frames.is_empty());
    for frame in frames {
        let eth = decode_header(&frame).expect("ethernet");
        assert_eq!(eth.ethertype, fps_tcp_own::net::ETHERTYPE_IPV6);
        let h = wire::parse_frame(&frame).expect("frame").header;
        assert!(h.src_ip.is_ipv6() && h.dst_ip.is_ipv6());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn an_ipv6_pair_connects_moves_data_both_ways_and_closes() {
    let lab = lab_pools(
        OwnConfig::default(),
        Pools::pair_v6(),
        deliver_all(),
        deliver_all(),
    );
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        let (client, server) = pair(&lab.stack).await;
        let up = pattern(200_003);
        let down = pattern(150_001);
        let (up_len, down_len) = (up.len(), down.len());
        let client_task = tokio::spawn(async move {
            let mut got = vec![0u8; down_len];
            let (mut rd, mut wr) = tokio::io::split(client);
            let (written, read) = tokio::join!(wr.write_all(&up), rd.read_exact(&mut got));
            written.expect("write");
            read.expect("read");
            wr.shutdown().await.expect("shutdown");
            (got, rd, wr)
        });
        let server_task = tokio::spawn(async move {
            let mut got = vec![0u8; up_len];
            let (mut rd, mut wr) = tokio::io::split(server);
            let (written, read) = tokio::join!(wr.write_all(&down), rd.read_exact(&mut got));
            written.expect("write");
            read.expect("read");
            wr.shutdown().await.expect("shutdown");
            (got, rd, wr)
        });
        let (client_side, server_side) = (
            client_task.await.expect("task"),
            server_task.await.expect("task"),
        );
        assert_eq!(client_side.0, pattern(150_001));
        assert_eq!(server_side.0, pattern(200_003));
    })
    .await;
    driver.abort();
    assert_all_ipv6(&lab.client_log);
    assert_all_ipv6(&lab.server_log);
    let syn = parsed(&lab.client_log)[0];
    assert_eq!(syn.flags, flags::SYN);
    assert_eq!(syn.src_ip, ip6("fd00:1::1"));
    assert_eq!(syn.dst_ip, ip6("fd00:2::1"));
    // 1500 less 40 bytes of IPv6 and 20 of TCP.
    assert_eq!(syn.mss, Some(1440));
    let c = lab.stack.counters();
    assert_eq!(c.client.bad_frames + c.server.bad_frames, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn ipv6_segments_fit_the_mtu_and_a_small_mtu_lowers_the_mss() {
    let config = OwnConfig {
        mtu: 1280,
        ..OwnConfig::default()
    };
    let lab = lab_pools(config, Pools::pair_v6(), deliver_all(), deliver_all());
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        let (mut client, mut server) = pair(&lab.stack).await;
        client.write_all(&pattern(40_000)).await.expect("write");
        let mut got = vec![0u8; 40_000];
        server.read_exact(&mut got).await.expect("read");
        assert_eq!(got, pattern(40_000));
        assert_eq!(parsed(&lab.client_log)[0].mss, Some(1220));
        let longest = frames_of(&lab.client_log)
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0);
        assert!(longest <= 1280 + 14, "{longest}");
        assert!(longest > 1200, "segments use the MTU: {longest}");
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn ipv6_lost_data_segments_are_sent_again_and_arrive_in_order() {
    let mut data_frames = 0u32;
    let client_rule: Rule = Box::new(move |frame| {
        let parsed = wire::parse_frame(frame).expect("frame");
        if parsed.payload.is_empty() {
            return Fate::Deliver;
        }
        data_frames += 1;
        if data_frames.is_multiple_of(7) {
            Fate::Drop
        } else {
            Fate::Deliver
        }
    });
    let lab = lab_pools(
        OwnConfig::default(),
        Pools::pair_v6(),
        client_rule,
        deliver_all(),
    );
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        let (mut client, mut server) = pair(&lab.stack).await;
        let data = pattern(300_001);
        let copy = data.clone();
        let writer = tokio::spawn(async move {
            client.write_all(&copy).await.expect("write");
            client.shutdown().await.expect("shutdown");
            client
        });
        let mut got = Vec::new();
        server.read_to_end(&mut got).await.expect("read");
        assert_eq!(got, data);
        let _client = writer.await.expect("task");
        assert!(lab.stack.counters().client.retransmits > 0);
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn ipv6_corrupted_frames_are_counted_and_the_data_still_arrives() {
    let mut n = 0u32;
    let client_rule: Rule = Box::new(move |_| {
        n += 1;
        if n.is_multiple_of(11) {
            Fate::Corrupt
        } else {
            Fate::Deliver
        }
    });
    let lab = lab_pools(
        OwnConfig::default(),
        Pools::pair_v6(),
        client_rule,
        deliver_all(),
    );
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        let (mut client, mut server) = pair(&lab.stack).await;
        client.write_all(&pattern(100_000)).await.expect("write");
        let mut got = vec![0u8; 100_000];
        server.read_exact(&mut got).await.expect("read");
        assert_eq!(got, pattern(100_000));
    })
    .await;
    driver.abort();
    assert!(lab.stack.counters().server.bad_frames > 0);
}

fn dual_pools(vlan: Option<u16>) -> Pools {
    let pool = |name, v4, v6, count| {
        fps_hosts::HostPool::parse(&fps_hosts::PoolSpec {
            name,
            ipv4: Some(v4),
            ipv6: Some(v6),
            count,
            ..fps_hosts::PoolSpec::default()
        })
        .expect("pool")
    };
    Pools {
        client: fps_tcp_own::HostSlice::whole(pool("client", "10.1.0.0/24", "fd00:1::/64", 3)),
        server: fps_tcp_own::HostSlice::whole(pool("server", "10.2.0.0/24", "fd00:2::/64", 2)),
        vlan,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn a_dual_stack_pool_serves_both_families_with_one_mac_per_host() {
    let lab = lab_pools(
        OwnConfig::default(),
        dual_pools(Some(30)),
        deliver_all(),
        deliver_all(),
    );
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        // Without a family the stack uses IPv4, and by address it can use
        // either, between any client host and server host.
        let (v4, accepted4) = tokio::join!(lab.stack.connect(PORT), lab.stack.accept(PORT));
        let (mut v4, mut accepted4) = (v4.expect("v4"), accepted4.expect("accept"));
        let (v6, accepted6) = tokio::join!(
            lab.stack
                .connect_addrs(ip6("fd00:1::3"), ip6("fd00:2::2"), PORT),
            lab.stack.accept(PORT)
        );
        let (mut v6, mut accepted6) = (v6.expect("v6"), accepted6.expect("accept"));
        for (client, server, byte) in [(&mut v4, &mut accepted4, 4u8), (&mut v6, &mut accepted6, 6)]
        {
            client.write_all(&[byte; 5000]).await.expect("write");
            let mut got = vec![0u8; 5000];
            server.read_exact(&mut got).await.expect("read");
            assert_eq!(got, vec![byte; 5000]);
        }
        // A pair of mixed families and an address that no host has.
        let err = lab
            .stack
            .connect_addrs(ip6("fd00:1::1"), "10.2.0.1".parse().expect("addr"), PORT)
            .await
            .expect_err("mixed");
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let err = lab
            .stack
            .connect_addrs(ip6("fd00:1::ff"), ip6("fd00:2::1"), PORT)
            .await
            .expect_err("no such host");
        assert_eq!(err.kind(), ErrorKind::AddrNotAvailable);
        let err = lab
            .stack
            .connect_addrs(ip6("fd00:1::1"), ip6("fd00:2::ff"), PORT)
            .await
            .expect_err("no such neighbor");
        assert_eq!(err.kind(), ErrorKind::HostUnreachable);
    })
    .await;
    driver.abort();
    let mut macs = std::collections::BTreeSet::new();
    let mut families = std::collections::BTreeSet::new();
    for frame in frames_of(&lab.client_log) {
        let eth = decode_header(&frame).expect("ethernet");
        assert_eq!(eth.vlan, Some(30));
        let h = wire::parse_frame(&frame).expect("frame").header;
        families.insert(h.src_ip.is_ipv4());
        if h.flags == flags::SYN {
            macs.insert((h.src_ip.is_ipv4(), eth.src));
        }
    }
    assert_eq!(families.len(), 2, "frames of both families");
    // The SYN of the IPv6 connect came from client host 2 (fd00:1::3), and
    // the one of the IPv4 connect from client host 0: two MAC addresses.
    assert_eq!(macs.len(), 2);
    let c = lab.stack.counters();
    assert_eq!(c.client.unresolved + c.server.unresolved, 0);
    assert_eq!(c.client.link_dropped + c.server.link_dropped, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn ipv6_pools_with_many_hosts_connect_by_index_on_a_tagged_link() {
    let pools = Pools::sized_v6("fd00:a::/64", 4, "fd00:b::/64", 3, Some(100)).expect("pools");
    let lab = lab_pools(OwnConfig::default(), pools, deliver_all(), deliver_all());
    lab.stack.listen(PORT).expect("listen");
    let driver = lab.start_clock_driver();
    within(async {
        let mut sessions = Vec::new();
        for i in 0..12u8 {
            let (client, server) = tokio::join!(lab.stack.connect(PORT), lab.stack.accept(PORT));
            sessions.push((i, client.expect("connect"), server.expect("accept")));
        }
        for (i, client, server) in &mut sessions {
            client.write_all(&[*i; 700]).await.expect("write");
            let mut got = [0u8; 700];
            server.read_exact(&mut got).await.expect("read");
            assert_eq!(got, [*i; 700]);
        }
        let (c, s) = tokio::join!(lab.stack.connect_hosts(3, 2, PORT), lab.stack.accept(PORT));
        let _ = (c.expect("connect"), s.expect("accept"));
    })
    .await;
    driver.abort();
    let mut tuples = std::collections::BTreeSet::new();
    for frame in frames_of(&lab.client_log) {
        assert_eq!(decode_header(&frame).expect("eth").vlan, Some(100));
        let h = wire::parse_frame(&frame).expect("frame").header;
        if h.flags == flags::SYN {
            tuples.insert((h.src_ip, h.dst_ip));
        }
    }
    assert_eq!(tuples.len(), 12, "each connect used its own address pair");
    assert!(tuples.contains(&(ip6("fd00:a::4"), ip6("fd00:b::3"))));
}

/// The TTL, window, and option names of the first frame of `log` with the
/// flags `want`, read from the bytes.
fn handshake_fields(log: &Log, want: u8) -> (u8, u16, Vec<&'static str>) {
    let frames = frames_of(log);
    let frame = frames
        .iter()
        .find(|f| wire::parse_frame(f).is_ok_and(|p| p.header.flags == want))
        .expect("a handshake frame");
    let head = decode_header(frame).expect("an Ethernet header");
    let ip = &frame[head.payload_at..];
    let tcp = &ip[wire::IP_LEN..];
    let window = u16::from_be_bytes([tcp[14], tcp[15]]);
    let mut names = Vec::new();
    let mut at = wire::TCP_LEN;
    let end = usize::from(tcp[12] >> 4) * 4;
    while at < end {
        let kind = tcp[at];
        let (name, len) = match kind {
            0 => ("eol", 1),
            1 => ("nop", 1),
            2 => ("mss", 4),
            3 => ("ws", 3),
            4 => ("sok", 2),
            8 => ("ts", 10),
            other => panic!("option kind {other}"),
        };
        names.push(name);
        if kind == 0 {
            break;
        }
        at += len;
    }
    (ip[8], window, names)
}

#[tokio::test(flavor = "current_thread")]
async fn os_profiles_shape_the_handshake_and_the_data_still_flows() {
    // Each profile on both sides, a transfer in both directions, and the
    // table of the Transport page read off the captured frames. A peer with
    // the same profile offers the same options, so the SYN-ACK carries the
    // order of the SYN.
    let table = [
        (
            "windows11",
            128u8,
            64240u16,
            vec!["mss", "nop", "ws", "nop", "nop", "sok"],
        ),
        ("linux6", 64, 64240, vec!["mss", "sok", "ts", "nop", "ws"]),
        (
            "macos14",
            64,
            65535,
            vec!["mss", "nop", "ws", "nop", "nop", "ts", "sok", "eol"],
        ),
    ];
    for (name, ttl, window, options) in table {
        let profile = OsProfile::builtin(name).expect("a built in profile");
        let config = OwnConfig {
            client_os_profile: Some(profile),
            server_os_profile: Some(profile),
            ..OwnConfig::default()
        };
        let lab = lab(config, deliver_all(), deliver_all());
        lab.stack.listen(PORT).expect("listen");
        let driver = lab.start_clock_driver();
        within(async {
            let (mut client, mut server) = pair(&lab.stack).await;
            let up = pattern(600_011);
            let down = pattern(200_003);
            let (up_copy, down_copy) = (up.clone(), down.clone());
            let writer = tokio::spawn(async move {
                client.write_all(&up_copy).await.expect("write");
                let mut got = vec![0u8; down_copy.len()];
                client.read_exact(&mut got).await.expect("read");
                assert_eq!(got, down_copy);
                client
            });
            let mut got = vec![0u8; up.len()];
            server.read_exact(&mut got).await.expect("read");
            assert_eq!(got, up, "{name}");
            server.write_all(&down).await.expect("write");
            let _client = writer.await.expect("task");
        })
        .await;
        driver.abort();
        for (log, bits) in [
            (&lab.client_log, flags::SYN),
            (&lab.server_log, flags::SYN | flags::ACK),
        ] {
            let (got_ttl, got_window, got_options) = handshake_fields(log, bits);
            assert_eq!(got_ttl, ttl, "{name} ttl of {bits:#x}");
            assert_eq!(got_window, window, "{name} window of {bits:#x}");
            assert_eq!(got_options, options, "{name} options of {bits:#x}");
        }
        for log in [&lab.client_log, &lab.server_log] {
            for frame in frames_of(log) {
                let head = decode_header(&frame).expect("an Ethernet header");
                assert_eq!(frame[head.payload_at + 8], ttl, "{name} every frame");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Listener lifetime (TRN-33).

#[tokio::test(flavor = "current_thread")]
async fn unlisten_resets_a_handshake_that_is_not_finished() {
    // The client delivers its SYN and drops everything after it, so the
    // server holds a connection in SYN-RECEIVED that the backlog counts.
    let mut sent = 0;
    let lab = lab(
        OwnConfig::default(),
        Box::new(move |_| {
            sent += 1;
            if sent == 1 {
                Fate::Deliver
            } else {
                Fate::Drop
            }
        }),
        deliver_all(),
    );
    lab.stack.listen(PORT).expect("listen");
    within(async {
        let connect = tokio::spawn({
            let stack = lab.stack.clone();
            async move { stack.connect(PORT).await }
        });
        tokio::task::yield_now().await;
        lab.stack.pump();
        // The client thinks it is established, because the SYN-ACK arrived.
        let mut client = connect.await.expect("task").expect("connect");
        // One connection of each host and the listener.
        assert_eq!(lab.stack.stats().sockets_open, 3);
        assert!(lab.stack.unlisten(PORT));
        assert_eq!(
            lab.stack.stats().sockets_open,
            1,
            "the half open connection and the listener are gone"
        );
        lab.stack.pump();
        let mut byte = [0u8; 1];
        let error = client.read(&mut byte).await.expect_err("reset");
        assert_eq!(error.kind(), ErrorKind::ConnectionReset);
        let sent_rst = frames_of(&lab.server_log)
            .iter()
            .filter_map(|f| wire::parse_frame(f).ok())
            .any(|p| p.header.flags & flags::RST != 0);
        assert!(
            sent_rst,
            "the server sent a reset for the half open connection"
        );
    })
    .await;
}

/// A clock that moves by 100 microseconds at every reading, and counts them.
struct TickingClock {
    micros: AtomicI64,
    reads: AtomicI64,
}

impl Clock for TickingClock {
    fn now_micros(&self) -> i64 {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.micros.fetch_add(100, Ordering::Relaxed)
    }
}

/// Bug #732: one `pump` call can run for milliseconds (an unoptimized build
/// needs 65 microseconds per segment, and the acknowledgments of a burst come
/// back while it is still going out). A time read once at its start gave a
/// segment and its acknowledgment the same instant, so the round trip samples
/// read 1 microsecond, the smallest round trip of HyStart became 1, and the
/// slow start ran on past the queue (63 to 113 segments sent again where the
/// kernel sends 7). The call reads the clock before every step of a pass.
#[test]
fn a_pump_pass_reads_the_clock_before_each_of_its_steps() {
    let clock = Arc::new(TickingClock {
        micros: AtomicI64::new(0),
        reads: AtomicI64::new(0),
    });
    let stack = OwnStack::with_clock(OwnConfig::default(), clock.clone());
    assert!(!stack.pump(), "an idle stack has nothing to move");
    // Receive and transmit of the client and of the server: four steps.
    assert_eq!(clock.reads.load(Ordering::Relaxed), 4);
}
