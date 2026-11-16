// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of the smoltcp adapter through its `SessionIo` interface.

use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;

use fps_tcp::{ManualClock, MonotonicClock, SmolConfig, SmolStack, TcpStack, TimeWait};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;

const PORT: u16 = 8080;

fn stack(config: SmolConfig) -> SmolStack {
    SmolStack::new(config, Arc::new(MonotonicClock::new()))
}

fn start_driver(stack: &SmolStack) -> JoinHandle<()> {
    let driver = stack.clone();
    tokio::spawn(async move { driver.drive().await })
}

/// Runs a test body with a driver and a timeout, so a hang fails the test.
async fn within<F: std::future::Future>(body: F) -> F::Output {
    match tokio::time::timeout(Duration::from_secs(30), body).await {
        Ok(output) => output,
        Err(_) => panic!("the test did not finish within 30 seconds"),
    }
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[tokio::test(flavor = "current_thread")]
async fn request_and_response_round_trip() {
    let stack = stack(SmolConfig::default());
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    within(async {
        let (client, server) = tokio::join!(stack.connect(PORT), stack.accept(PORT));
        let (mut client, mut server) = (client.expect("connect"), server.expect("accept"));
        client.write_all(b"ping").await.expect("write");
        let mut request = [0u8; 4];
        server.read_exact(&mut request).await.expect("read");
        assert_eq!(&request, b"ping");
        server.write_all(b"pong").await.expect("write");
        let mut response = [0u8; 4];
        client.read_exact(&mut response).await.expect("read");
        assert_eq!(&response, b"pong");
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn bulk_transfer_through_small_buffers_keeps_every_byte() {
    // 4 KiB buffers make the writer wait for window updates many times.
    let config = SmolConfig {
        rx_buffer: 4096,
        tx_buffer: 4096,
        ..SmolConfig::default()
    };
    let stack = stack(config);
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    let data = pattern(1_000_003);
    let received = within(async {
        let (client, server) = tokio::join!(stack.connect(PORT), stack.accept(PORT));
        let (mut client, mut server) = (client.expect("connect"), server.expect("accept"));
        let sent = data.clone();
        let writer = tokio::spawn(async move {
            client.write_all(&sent).await.expect("write");
            client.shutdown().await.expect("shutdown");
            client
        });
        let mut received = Vec::new();
        server.read_to_end(&mut received).await.expect("read");
        drop(writer.await.expect("writer"));
        received
    })
    .await;
    driver.abort();
    assert_eq!(received.len(), data.len());
    assert!(
        received == data,
        "the received bytes differ from the sent bytes"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_gives_the_peer_end_of_stream_after_the_data() {
    let stack = stack(SmolConfig::default());
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    within(async {
        let (client, server) = tokio::join!(stack.connect(PORT), stack.accept(PORT));
        let (mut client, mut server) = (client.expect("connect"), server.expect("accept"));
        client.write_all(b"last words").await.expect("write");
        client.shutdown().await.expect("shutdown");
        let mut all = Vec::new();
        server.read_to_end(&mut all).await.expect("read");
        assert_eq!(all, b"last words");
        // The server can still answer on the half closed session.
        server.write_all(b"ok").await.expect("write");
        let mut answer = [0u8; 2];
        client.read_exact(&mut answer).await.expect("read");
        assert_eq!(&answer, b"ok");
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn abort_resets_the_peer_and_ends_later_operations() {
    use fps_io::SessionIo;
    let stack = stack(SmolConfig::default());
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    within(async {
        let (client, server) = tokio::join!(stack.connect(PORT), stack.accept(PORT));
        let (mut client, mut server) = (client.expect("connect"), server.expect("accept"));
        client.write_all(b"unread").await.expect("write");
        client.abort();
        client.abort();
        let error = client.write_all(b"x").await.expect_err("aborted write");
        assert_eq!(error.kind(), ErrorKind::NotConnected);
        let mut buf = [0u8; 16];
        let error = server.read(&mut buf).await.expect_err("reset read");
        assert_eq!(error.kind(), ErrorKind::ConnectionReset);
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn connecting_to_a_port_without_listener_is_refused() {
    let stack = stack(SmolConfig::default());
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    let error = within(stack.connect(PORT + 1))
        .await
        .err()
        .expect("refused connect");
    assert_eq!(error.kind(), ErrorKind::ConnectionRefused);
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn listening_twice_on_one_port_fails() {
    let stack = stack(SmolConfig::default());
    stack.listen(PORT).expect("listen");
    let error = stack.listen(PORT).expect_err("second listen");
    assert_eq!(error.kind(), ErrorKind::AddrInUse);
}

#[tokio::test(flavor = "current_thread")]
async fn a_burst_larger_than_the_backlog_all_connects() {
    let config = SmolConfig {
        backlog: 2,
        ..SmolConfig::default()
    };
    let stack = stack(config);
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    let accepted = within(async {
        let mut clients = Vec::new();
        for _ in 0..100 {
            let stack = stack.clone();
            clients.push(tokio::spawn(async move { stack.connect(PORT).await }));
        }
        let mut sessions = Vec::new();
        for _ in 0..100 {
            sessions.push(stack.accept(PORT).await.expect("accept"));
        }
        for client in clients {
            sessions.push(client.await.expect("task").expect("connect"));
        }
        sessions.len()
    })
    .await;
    driver.abort();
    assert_eq!(accepted, 200);
}

#[tokio::test(flavor = "current_thread")]
async fn ended_sessions_leave_no_sockets_behind() {
    let stack = stack(SmolConfig::default());
    stack.listen(PORT).expect("listen");
    let baseline = stack.stats().sockets_open;
    let driver = start_driver(&stack);
    within(async {
        for _ in 0..50 {
            let (client, server) = tokio::join!(stack.connect(PORT), stack.accept(PORT));
            let (mut client, mut server) = (client.expect("connect"), server.expect("accept"));
            client.write_all(b"a").await.expect("write");
            let mut byte = [0u8; 1];
            server.read_exact(&mut byte).await.expect("read");
            client.shutdown().await.expect("shutdown");
            let mut rest = Vec::new();
            server.read_to_end(&mut rest).await.expect("read");
            server.shutdown().await.expect("shutdown");
            drop(client);
            drop(server);
        }
        // The driver reaps closed sockets on its next passes.
        for _ in 0..1000 {
            if stack.stats().sockets_open == baseline {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    driver.abort();
    assert_eq!(stack.stats().sockets_open, baseline);
}

#[tokio::test(flavor = "current_thread")]
async fn time_wait_sockets_stay_under_the_keep_policy() {
    let config = SmolConfig {
        time_wait: TimeWait::Keep,
        ..SmolConfig::default()
    };
    let clock = Arc::new(ManualClock::new());
    let stack = SmolStack::new(config, clock.clone());
    stack.listen(PORT).expect("listen");
    let baseline = stack.stats().sockets_open;
    let driver = start_driver(&stack);
    within(async {
        let (client, server) = tokio::join!(stack.connect(PORT), stack.accept(PORT));
        let (mut client, mut server) = (client.expect("connect"), server.expect("accept"));
        client.shutdown().await.expect("shutdown");
        let mut rest = Vec::new();
        server.read_to_end(&mut rest).await.expect("read");
        server.shutdown().await.expect("shutdown");
        let mut rest = Vec::new();
        client.read_to_end(&mut rest).await.expect("read");
        drop(client);
        drop(server);
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }
        // The client side waits in TIME-WAIT while the manual clock stands still.
        assert_eq!(stack.stats().sockets_open, baseline + 1);
        clock.advance_micros(11_000_000);
        for _ in 0..1000 {
            if stack.stats().sockets_open == baseline {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(stack.stats().sockets_open, baseline);
    })
    .await;
    driver.abort();
}
