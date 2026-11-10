// SPDX-License-Identifier: GPL-3.0-only
//! The binary packet protocol of RFC 4253 section 6.
//!
//! Each direction of a session is a [`Half`]. A half starts in the clear and
//! switches to `chacha20-poly1305@openssh.com` when `SSH_MSG_NEWKEYS` passes.
//! The cipher is the one that OpenSSH defines in PROTOCOL.chacha20poly1305:
//! the 4 byte length is encrypted with its own key, the rest of the packet
//! with a second key, and a Poly1305 tag over both closes the packet. The
//! nonce of both ciphers is the 64 bit packet sequence number.
//!
//! The first 4 bytes of a packet are always readable, either as the length
//! or as the encrypted length, so a receiver reads 4 bytes, asks
//! [`Half::body_len`] for the rest, and reads that many more bytes.

use chacha20::cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};
use chacha20::ChaCha20;
use fps_flow::FnError;
use poly1305::universal_hash::KeyInit;
use poly1305::Poly1305;
use rand::RngCore;

use crate::wire::protocol;

/// The largest `packet_length` that a receiver accepts. RFC 4253 asks
/// implementations to handle 35000 bytes, and OpenSSH refuses more.
pub(crate) const MAX_PACKET: usize = 35_000;

/// Size of the Poly1305 tag.
const TAG: usize = 16;

/// The 64 byte key of `chacha20-poly1305@openssh.com`: `main` is K_2, which
/// protects the payload and keys the tag, and `header` is K_1, which
/// encrypts the length.
#[derive(Clone)]
pub(crate) struct Keys {
    pub(crate) main: [u8; 32],
    pub(crate) header: [u8; 32],
}

impl Keys {
    /// Splits 64 bytes of key material the way PROTOCOL.chacha20poly1305
    /// does: the first 32 bytes are K_2 and the last 32 bytes are K_1.
    pub(crate) fn from_material(material: &[u8]) -> Option<Keys> {
        let main: [u8; 32] = material.get(..32)?.try_into().ok()?;
        let header: [u8; 32] = material.get(32..64)?.try_into().ok()?;
        Some(Keys { main, header })
    }
}

/// One direction of the packet protocol.
pub(crate) struct Half {
    keys: Option<Keys>,
    seq: u32,
}

/// The nonce of packet `seq`: the 64 bit big endian sequence number in the
/// last 8 bytes of a 12 byte IETF ChaCha20 nonce. The counter of the IETF
/// layout is 32 bits wide, and no packet needs a 64 bit one.
fn nonce(seq: u32) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[8..].copy_from_slice(&seq.to_be_bytes());
    n
}

fn tags_equal(a: &[u8; TAG], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Default for Half {
    fn default() -> Self {
        Self::new()
    }
}

impl Half {
    /// A half that has sent or received no packet.
    pub(crate) fn new() -> Self {
        Half { keys: None, seq: 0 }
    }

    /// Starts to protect the packets after this one with `keys`.
    pub(crate) fn switch(&mut self, keys: Keys) {
        self.keys = Some(keys);
    }

    /// The number of packets that this half has handled.
    #[cfg(test)]
    pub(crate) fn sequence(&self) -> u32 {
        self.seq
    }

    /// Builds the packet for `payload` and appends it to `out`.
    pub(crate) fn seal(&mut self, payload: &[u8], rng: &mut dyn RngCore, out: &mut Vec<u8>) {
        // The padding makes the part that the length covers a multiple of 8
        // bytes, and at least 4 bytes of padding are mandatory. In the clear
        // the length field itself counts, so the total is a multiple of 8.
        let head = if self.keys.is_some() { 1 } else { 5 };
        let mut pad = 8 - (head + payload.len()) % 8;
        if pad < 4 {
            pad += 8;
        }
        let length = 1 + payload.len() + pad;
        let start = out.len();
        out.extend_from_slice(&u32::try_from(length).unwrap_or(u32::MAX).to_be_bytes());
        out.push(u8::try_from(pad).unwrap_or(4));
        out.extend_from_slice(payload);
        let pad_at = out.len();
        out.resize(pad_at + pad, 0);
        rng.fill_bytes(&mut out[pad_at..]);
        if let Some(keys) = &self.keys {
            let n = nonce(self.seq);
            let packet = &mut out[start..];
            let mut header = ChaCha20::new(&keys.header.into(), &n.into());
            header.apply_keystream(&mut packet[..4]);
            let mut main = ChaCha20::new(&keys.main.into(), &n.into());
            let mut poly_key = [0u8; 32];
            main.apply_keystream(&mut poly_key);
            main.seek(64u32);
            main.apply_keystream(&mut packet[4..]);
            let tag: [u8; TAG] = Poly1305::new(&poly_key.into())
                .compute_unpadded(packet)
                .into();
            out.extend_from_slice(&tag);
        }
        self.seq = self.seq.wrapping_add(1);
    }

    /// Reads the length field of a packet from its first 4 bytes and returns
    /// how many bytes follow them, tag included.
    pub(crate) fn body_len(&self, head: &[u8; 4]) -> Result<usize, FnError> {
        let mut raw = *head;
        if let Some(keys) = &self.keys {
            let mut header = ChaCha20::new(&keys.header.into(), &nonce(self.seq).into());
            header.apply_keystream(&mut raw);
        }
        let length = u32::from_be_bytes(raw) as usize;
        if !(8..=MAX_PACKET).contains(&length) {
            return Err(protocol(format!(
                "packet length {length} lies outside 8 to {MAX_PACKET}"
            )));
        }
        if self.keys.is_some() {
            if length % 8 != 0 {
                return Err(protocol(format!(
                    "packet length {length} is not a multiple of 8"
                )));
            }
            Ok(length + TAG)
        } else {
            if (length + 4) % 8 != 0 {
                return Err(protocol(format!(
                    "unencrypted packet of {} bytes is not a multiple of 8",
                    length + 4
                )));
            }
            Ok(length)
        }
    }

    /// Checks and decrypts the packet whose first 4 bytes are `head` and
    /// whose remaining bytes are `body`, and returns its payload.
    pub(crate) fn open(&mut self, head: &[u8; 4], body: &[u8]) -> Result<Vec<u8>, FnError> {
        let expected = self.body_len(head)?;
        if body.len() != expected {
            return Err(protocol(format!(
                "packet body has {} bytes, its length field announced {expected}",
                body.len()
            )));
        }
        let mut data = if let Some(keys) = &self.keys {
            let n = nonce(self.seq);
            let (enc, tag) = body.split_at(body.len() - TAG);
            let mut main = ChaCha20::new(&keys.main.into(), &n.into());
            let mut poly_key = [0u8; 32];
            main.apply_keystream(&mut poly_key);
            let mut signed = Vec::with_capacity(4 + enc.len());
            signed.extend_from_slice(head);
            signed.extend_from_slice(enc);
            let expected_tag: [u8; TAG] = Poly1305::new(&poly_key.into())
                .compute_unpadded(&signed)
                .into();
            if !tags_equal(&expected_tag, tag) {
                return Err(protocol(format!(
                    "the authentication tag of packet {} does not match",
                    self.seq
                )));
            }
            main.seek(64u32);
            let mut plain = enc.to_vec();
            main.apply_keystream(&mut plain);
            plain
        } else {
            body.to_vec()
        };
        self.seq = self.seq.wrapping_add(1);
        let pad = usize::from(*data.first().ok_or_else(|| protocol("empty packet"))?);
        if pad < 4 || pad + 1 > data.len() {
            return Err(protocol(format!(
                "padding length {pad} does not fit a packet of {} bytes",
                data.len()
            )));
        }
        data.truncate(data.len() - pad);
        data.remove(0);
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn rng() -> rand_pcg::Pcg64Mcg {
        rand_pcg::Pcg64Mcg::seed_from_u64(1)
    }

    fn keys(fill: u8) -> Keys {
        Keys {
            main: [fill; 32],
            header: [fill.wrapping_add(1); 32],
        }
    }

    fn open(half: &mut Half, packet: &[u8]) -> Result<Vec<u8>, FnError> {
        let head: [u8; 4] = packet[..4].try_into().unwrap();
        half.open(&head, &packet[4..])
    }

    #[test]
    fn clear_packets_round_trip_and_align() {
        let (mut tx, mut rx) = (Half::new(), Half::new());
        for len in 0..40 {
            let payload = vec![0x42; len];
            let mut out = Vec::new();
            tx.seal(&payload, &mut rng(), &mut out);
            assert_eq!(out.len() % 8, 0, "payload {len}");
            assert!(out.len() >= 16);
            assert_eq!(open(&mut rx, &out).unwrap(), payload);
        }
        assert_eq!(tx.sequence(), 40);
        assert_eq!(rx.sequence(), 40);
    }

    #[test]
    fn encrypted_packets_round_trip_and_align() {
        let (mut tx, mut rx) = (Half::new(), Half::new());
        tx.switch(keys(7));
        rx.switch(keys(7));
        for len in 0..80 {
            let payload: Vec<u8> = (0..len as u8).collect();
            let mut out = Vec::new();
            tx.seal(&payload, &mut rng(), &mut out);
            assert_eq!((out.len() - 4 - TAG) % 8, 0, "payload {len}");
            assert_eq!(open(&mut rx, &out).unwrap(), payload);
        }
    }

    #[test]
    fn a_keystroke_packet_has_the_size_that_openssh_sends() {
        // SSH_MSG_CHANNEL_DATA with one byte: type, channel, length, byte.
        let mut tx = Half::new();
        tx.switch(keys(3));
        let mut out = Vec::new();
        tx.seal(&[0u8; 10], &mut rng(), &mut out);
        assert_eq!(out.len(), 36);
    }

    #[test]
    fn the_sequence_number_is_part_of_the_nonce() {
        let (mut tx, mut rx) = (Half::new(), Half::new());
        tx.switch(keys(1));
        rx.switch(keys(1));
        let (mut a, mut b) = (Vec::new(), Vec::new());
        tx.seal(b"same", &mut rng(), &mut a);
        tx.seal(b"same", &mut rng(), &mut b);
        assert_ne!(a, b);
        // Reading the second packet first uses the wrong nonce.
        assert!(open(&mut rx, &b).is_err());
    }

    #[test]
    fn a_flipped_bit_fails_the_tag() {
        let (mut tx, mut rx) = (Half::new(), Half::new());
        tx.switch(keys(9));
        rx.switch(keys(9));
        let mut out = Vec::new();
        tx.seal(b"payload", &mut rng(), &mut out);
        for at in [5, 12, out.len() - 1] {
            let mut bad = out.clone();
            bad[at] ^= 1;
            let mut fresh = Half::new();
            fresh.switch(keys(9));
            assert!(open(&mut fresh, &bad).is_err(), "byte {at}");
        }
        assert!(open(&mut rx, &out).is_ok());
    }

    #[test]
    fn hostile_lengths_fail_without_panicking() {
        let h = Half::new();
        for head in [[0, 0, 0, 0], [0xff, 0xff, 0xff, 0xff], [0, 0, 0x90, 0], [0, 0, 0, 13]] {
            assert!(h.body_len(&head).is_err(), "{head:?}");
        }
        let mut e = Half::new();
        e.switch(keys(5));
        for head in [[0, 0, 0, 0], [0xff; 4], [1, 2, 3, 4]] {
            // A random header decrypts to some length, which the checks
            // accept or refuse, and neither case panics.
            let _ = e.body_len(&head);
        }
        let mut h = Half::new();
        assert!(h.open(&[0, 0, 0, 12], &[0; 3]).is_err());
        // A padding length that exceeds the packet.
        let mut body = vec![0u8; 12];
        body[0] = 200;
        assert!(h.open(&[0, 0, 0, 12], &body).is_err());
    }
}
