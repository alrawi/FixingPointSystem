// SPDX-License-Identifier: GPL-3.0-only
//! Component and property tests of the in memory duplex through the public
//! `SessionIo` interface.

use fps_io::memory::duplex;
use fps_io::SessionIo;
use proptest::prelude::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime")
}

/// Writes `chunks` from one end and reads everything at the other end.
async fn transfer<S: SessionIo>(mut tx: S, mut rx: S, chunks: Vec<Vec<u8>>) -> Vec<u8> {
    let writer = async move {
        for c in &chunks {
            tx.write_all(c).await.expect("write");
        }
        tx.shutdown().await.expect("shutdown");
    };
    let reader = async move {
        let mut out = Vec::new();
        rx.read_to_end(&mut out).await.expect("read");
        out
    };
    let ((), out) = tokio::join!(writer, reader);
    out
}

proptest! {
    #[test]
    fn bytes_arrive_intact_and_in_order(
        capacity in 1usize..256,
        chunks in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..512), 0..16),
    ) {
        let expected: Vec<u8> = chunks.concat();
        let (a, b) = duplex(capacity);
        let out = runtime().block_on(transfer(a, b, chunks));
        prop_assert_eq!(out, expected);
    }
}

#[tokio::test]
async fn boxed_trait_objects_forward_abort() {
    let (a, b) = duplex(16);
    let mut a: Box<dyn SessionIo> = Box::new(a);
    let mut b: Box<dyn SessionIo> = Box::new(b);
    a.write_all(b"hello").await.expect("write");
    let mut buf = [0u8; 5];
    b.read_exact(&mut buf).await.expect("read");
    assert_eq!(&buf, b"hello");
    a.abort();
    let err = b.read(&mut buf).await.expect_err("reset");
    assert_eq!(err.kind(), std::io::ErrorKind::ConnectionReset);
}

#[tokio::test]
async fn abort_while_the_writer_waits_on_a_full_pipe() {
    let (mut a, mut b) = duplex(4);
    let writer = tokio::spawn(async move { a.write_all(&[7u8; 64]).await });
    tokio::task::yield_now().await;
    b.abort();
    let err = writer.await.expect("join").expect_err("reset");
    assert_eq!(err.kind(), std::io::ErrorKind::ConnectionReset);
}
