// SPDX-License-Identifier: GPL-3.0-only
//! Component and property tests of the in memory datagram pair through the
//! public `DatagramIo` interface.

use fps_io::memory::datagram_pair;
use fps_io::{DatagramIo, DatagramIoExt};
use proptest::prelude::*;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime")
}

/// Sends `datagrams` from `tx` and receives as many at `rx` into a buffer of
/// `buf_len` bytes, returning the full length and the kept bytes of each.
async fn exchange<D: DatagramIo>(
    mut tx: D,
    mut rx: D,
    datagrams: &[Vec<u8>],
    buf_len: usize,
) -> Vec<(usize, Vec<u8>)> {
    for d in datagrams {
        tx.send_datagram(d).await.expect("send");
    }
    let mut buf = vec![0u8; buf_len];
    let mut out = Vec::new();
    for _ in datagrams {
        let n = rx.recv_datagram(&mut buf).await.expect("recv");
        out.push((n, buf[..n.min(buf_len)].to_vec()));
    }
    out
}

proptest! {
    /// Every datagram arrives once, whole or truncated to the buffer, in
    /// order, and with its full length, so boundaries never move.
    #[test]
    fn datagrams_keep_boundaries_order_and_length(
        buf_len in 1usize..96,
        datagrams in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..128), 0..16),
    ) {
        let (a, b) = datagram_pair(datagrams.len());
        let got = runtime().block_on(exchange(a, b, &datagrams, buf_len));
        prop_assert_eq!(got.len(), datagrams.len());
        for ((n, kept), sent) in got.iter().zip(&datagrams) {
            prop_assert_eq!(*n, sent.len());
            prop_assert_eq!(kept.as_slice(), &sent[..sent.len().min(buf_len)]);
        }
    }

    /// A queue of `q` datagrams keeps the first `q` and drops the rest.
    #[test]
    fn a_full_queue_keeps_the_oldest_datagrams(q in 0usize..8, sent in 0usize..16) {
        let (mut a, b) = datagram_pair(q);
        runtime().block_on(async {
            for i in 0..sent {
                a.send_datagram(&[i as u8]).await.expect("send");
            }
        });
        prop_assert_eq!(b.queued(), sent.min(q));
        prop_assert_eq!(a.dropped(), sent.saturating_sub(q) as u64);
    }
}

#[test]
fn a_boxed_end_is_a_datagram_io() {
    let (a, b) = datagram_pair(4);
    let a: Box<dyn DatagramIo> = Box::new(a);
    let b: Box<dyn DatagramIo> = Box::new(b);
    let got = runtime().block_on(exchange(a, b, &[b"boxed".to_vec()], 16));
    assert_eq!(got, [(5, b"boxed".to_vec())]);
}
