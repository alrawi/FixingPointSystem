// SPDX-License-Identifier: GPL-3.0-only
//! Component tests of the kernel reference stack through its `SessionIo` interface.

use std::io::ErrorKind;
use std::time::Duration;

use fps_tcp::{KernelConfig, KernelStack, TcpStack};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;

const PORT: u16 = 8080;

fn stack(config: KernelConfig) -> KernelStack {
    KernelStack::new(config)
}

fn start_driver(stack: &KernelStack) -> JoinHandle<()> {
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
    let stack = stack(KernelConfig::default());
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
async fn a_congestion_algorithm_is_set_on_a_socket_and_an_unknown_one_is_refused() {
    // `reno` is always built into Linux.
    let socket = tokio::net::TcpSocket::new_v4().expect("socket");
    fps_tcp::kernel::set_congestion(&socket, "reno").expect("reno");
    let name = socket2::SockRef::from(&socket)
        .tcp_congestion()
        .expect("read back");
    assert!(name.starts_with(b"reno"));
    assert!(fps_tcp::kernel::set_congestion(&socket, "no-such-algorithm").is_err());
    let config = KernelConfig {
        congestion: Some("reno".into()),
        ..KernelConfig::default()
    };
    let stack = stack(config);
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    within(async {
        let (client, server) = tokio::join!(stack.connect(PORT), stack.accept(PORT));
        client.expect("connect");
        server.expect("accept");
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn bulk_transfer_through_small_buffers_keeps_every_byte() {
    // Small buffers make the writer wait for window updates many times.
    let config = KernelConfig {
        buffer_bytes: Some(4096),
        ..KernelConfig::default()
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
    let stack = stack(KernelConfig::default());
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
    let stack = stack(KernelConfig::default());
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
        // The kernel may hand over the bytes that arrived before the reset
        // first, and then the reset shows.
        let mut buf = [0u8; 16];
        let error = loop {
            match server.read(&mut buf).await {
                Ok(0) => panic!("end of stream instead of a reset"),
                Ok(_) => {}
                Err(error) => break error,
            }
        };
        assert_eq!(error.kind(), ErrorKind::ConnectionReset);
    })
    .await;
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn connecting_to_a_port_without_listener_is_refused() {
    let stack = stack(KernelConfig::default());
    stack.listen(PORT).expect("listen");
    let driver = start_driver(&stack);
    let error = within(stack.connect(PORT + 1))
        .await
        .expect_err("refused connect");
    assert_eq!(error.kind(), ErrorKind::ConnectionRefused);
    driver.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn listening_twice_on_one_port_fails() {
    let stack = stack(KernelConfig::default());
    stack.listen(PORT).expect("listen");
    let error = stack.listen(PORT).expect_err("second listen");
    assert_eq!(error.kind(), ErrorKind::AddrInUse);
}

#[tokio::test(flavor = "current_thread")]
async fn a_burst_of_connects_all_arrive() {
    let stack = stack(KernelConfig::default());
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
    let stack = stack(KernelConfig::default());
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
    })
    .await;
    driver.abort();
    assert_eq!(stack.stats().sockets_open, baseline);
}
