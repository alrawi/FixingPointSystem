// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of the own stack through its `SessionIo` interface, over
//! the in memory frame link and over a link that loses, corrupts, and
//! reorders frames.

use std::io;
use std::io::ErrorKind;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use fps_backend_afxdp::memory::{frame_pair, MemoryFrames};
use fps_backend_afxdp::{FrameIo, QueueStats};
use fps_io::SessionIo;
use fps_tcp::own::wire::{self, flags};
use fps_tcp::{ManualClock, OwnConfig, OwnStack, TcpStack};
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
    let (client, server) = frame_pair(config.link_depth, config.mtu);
    let (client_log, server_log) = (Log::default(), Log::default());
    let clock = Arc::new(ManualClock::new());
    let stack = OwnStack::with_links(
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
        clock.clone(),
    );
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

async fn pair(stack: &OwnStack) -> (fps_tcp::OwnSession, fps_tcp::OwnSession) {
    let (client, server) = tokio::join!(stack.connect(PORT), stack.accept(PORT));
    (client.expect("connect"), server.expect("accept"))
}

fn parsed(log: &Log) -> Vec<wire::Header> {
    log.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter_map(|f| wire::parse(f).ok().map(|p| p.header))
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
        let h = wire::parse(frame).expect("frame").header;
        if h.flags == flags::SYN && !seen_syn {
            seen_syn = true;
            return Fate::Drop;
        }
        Fate::Deliver
    });
    let server_rule: Rule = Box::new(move |frame| {
        let h = wire::parse(frame).expect("frame").header;
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
        let parsed = wire::parse(frame).expect("frame");
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
        let h = wire::parse(frame).expect("frame").header;
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
        let parsed = wire::parse(frame).expect("frame");
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
        let parsed = wire::parse(frame).expect("frame");
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
            .all(|f| f.len() <= 576));
    })
    .await;
    driver.abort();
}

mod workloads {
    use super::*;
    use fps_tcp::workload::{
        session_rate, socket_scaling, throughput, Direction, RateConfig, ScaleConfig,
        ThroughputConfig,
    };

    #[tokio::test(flavor = "current_thread")]
    async fn the_three_workloads_run_on_the_own_stack() {
        let stack = OwnStack::default();
        let rate = session_rate(
            &stack,
            &RateConfig {
                duration: Duration::from_millis(300),
                concurrency: 8,
                request_bytes: 100,
                response_bytes: 3000,
                ..RateConfig::default()
            },
        )
        .await
        .expect("rate");
        assert_eq!(rate.errors, 0);
        assert!(rate.sessions >= 8, "only {} sessions", rate.sessions);
        // The listener stays, and the closing side of every session holds its
        // tuple in TIME-WAIT for the hold of one second.
        assert!(rate.stats.sockets_open >= 1);
        assert!(rate.stats.sockets_open <= 1 + rate.sessions + 8);
        assert!(rate.stats.frames > rate.sessions * 6);

        for (port, direction) in [(7200, Direction::Upload), (7201, Direction::Download)] {
            let stack = OwnStack::default();
            let run = throughput(
                &stack,
                &ThroughputConfig {
                    sessions: 3,
                    bytes_per_session: 1_000_003,
                    chunk: 4096,
                    direction,
                    port,
                },
            )
            .await
            .expect("throughput");
            assert_eq!(run.bytes, 3_000_009, "{direction:?}");
            assert_eq!(stack.counters().client.retransmits, 0);
            assert_eq!(stack.counters().server.retransmits, 0);
        }

        let shards = [OwnStack::default(), OwnStack::default()];
        let report = socket_scaling(
            &shards,
            &ScaleConfig {
                steps: vec![400, 1600],
                probe_sessions: 20,
                ..ScaleConfig::default()
            },
        )
        .await
        .expect("scale");
        assert!(report.stopped.is_none(), "{:?}", report.stopped);
        assert!(report.steps.iter().all(|s| s.reached));
        assert!(report.max_sockets() >= 1600);
        assert!(report.steps[1].probe_sessions_per_s > 0.0);
    }
}
