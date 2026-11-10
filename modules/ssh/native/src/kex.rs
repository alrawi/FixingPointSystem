// SPDX-License-Identifier: GPL-3.0-only
//! Key exchange: the `SSH_MSG_KEXINIT` message, the `curve25519-sha256`
//! exchange hash of RFC 8731, and the key derivation of RFC 4253 section 7.2.
//!
//! The module implements one algorithm set, which is the modern OpenSSH
//! default: `curve25519-sha256` for the key exchange, `ssh-ed25519` for the
//! host key, and `chacha20-poly1305@openssh.com` for the cipher in both
//! directions. The cipher carries its own integrity check, so the MAC list is
//! present but unused, as in a real OpenSSH exchange.

use fps_flow::FnError;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::transport::Keys;
use crate::wire::{protocol, put_bool, put_mpint, put_string, put_u32, Reader};

/// `SSH_MSG_KEXINIT`.
pub(crate) const MSG_KEXINIT: u8 = 20;

/// The key exchange algorithm that the module runs, and its alias.
pub(crate) const KEX: &str = "curve25519-sha256";
const KEX_ALIAS: &str = "curve25519-sha256@libssh.org";
/// The host key algorithm.
pub(crate) const HOST_KEY: &str = "ssh-ed25519";
/// The cipher of both directions.
pub(crate) const CIPHER: &str = "chacha20-poly1305@openssh.com";
const MAC: &str = "hmac-sha2-256-etm@openssh.com";

/// Renders the `SSH_MSG_KEXINIT` payload with a random cookie.
pub(crate) fn kexinit(rng: &mut dyn RngCore) -> Vec<u8> {
    let mut out = vec![MSG_KEXINIT];
    let mut cookie = [0u8; 16];
    rng.fill_bytes(&mut cookie);
    out.extend_from_slice(&cookie);
    let kex = format!("{KEX},{KEX_ALIAS}");
    for list in [
        kex.as_str(),
        HOST_KEY,
        CIPHER,
        CIPHER,
        MAC,
        MAC,
        "none",
        "none",
        "",
        "",
    ] {
        put_string(&mut out, list.as_bytes());
    }
    put_bool(&mut out, false);
    put_u32(&mut out, 0);
    out
}

/// Checks that the peer's `SSH_MSG_KEXINIT` offers what the module runs.
pub(crate) fn check_kexinit(payload: &[u8]) -> Result<(), FnError> {
    let mut r = Reader::new(payload);
    if r.u8("the message type")? != MSG_KEXINIT {
        return Err(protocol("the message is not SSH_MSG_KEXINIT"));
    }
    r.skip(16, "the cookie")?;
    let kex = r.names("the key exchange list")?;
    let host = r.names("the host key list")?;
    let enc_c2s = r.names("the client to server cipher list")?;
    let enc_s2c = r.names("the server to client cipher list")?;
    for _ in 0..6 {
        r.names("a list of macs, compressions, or languages")?;
    }
    let first_follows = r.boolean("first_kex_packet_follows")?;
    let _reserved = r.u32("the reserved field")?;
    if first_follows {
        return Err(protocol(
            "the peer announces a guessed key exchange packet, which the module does not support",
        ));
    }
    let need = |what: &str, list: &[&str], names: &[&str]| -> Result<(), FnError> {
        if names.iter().any(|n| list.contains(n)) {
            Ok(())
        } else {
            Err(protocol(format!(
                "the peer offers no {what} that the module runs, wanted one of {names:?}"
            )))
        }
    };
    need("key exchange", &kex, &[KEX, KEX_ALIAS])?;
    need("host key algorithm", &host, &[HOST_KEY])?;
    need("client to server cipher", &enc_c2s, &[CIPHER])?;
    need("server to client cipher", &enc_s2c, &[CIPHER])
}

/// The values that the exchange hash covers, in the order of RFC 8731.
pub(crate) struct Exchange<'a> {
    /// The client banner, without the line break.
    pub(crate) v_c: &'a [u8],
    /// The server banner, without the line break.
    pub(crate) v_s: &'a [u8],
    /// The payload of the client `SSH_MSG_KEXINIT`.
    pub(crate) i_c: &'a [u8],
    /// The payload of the server `SSH_MSG_KEXINIT`.
    pub(crate) i_s: &'a [u8],
    /// The host key blob.
    pub(crate) k_s: &'a [u8],
    /// The client ephemeral public key.
    pub(crate) q_c: &'a [u8; 32],
    /// The server ephemeral public key.
    pub(crate) q_s: &'a [u8; 32],
    /// The shared secret.
    pub(crate) k: &'a [u8; 32],
}

impl Exchange<'_> {
    /// The exchange hash H.
    pub(crate) fn hash(&self) -> [u8; 32] {
        let mut buf = Vec::new();
        for s in [
            self.v_c, self.v_s, self.i_c, self.i_s, self.k_s, self.q_c, self.q_s,
        ] {
            put_string(&mut buf, s);
        }
        put_mpint(&mut buf, self.k);
        Sha256::digest(&buf).into()
    }
}

/// Derives one key of `len` bytes: `K1 = HASH(K || H || letter || id)`, and
/// every further block is `HASH(K || H || K1 || ...)`.
fn derive(k: &[u8; 32], h: &[u8; 32], letter: u8, session_id: &[u8; 32], len: usize) -> Vec<u8> {
    let mut k_mpint = Vec::new();
    put_mpint(&mut k_mpint, k);
    let mut out = {
        let mut d = Sha256::new();
        d.update(&k_mpint);
        d.update(h);
        d.update([letter]);
        d.update(session_id);
        d.finalize().to_vec()
    };
    while out.len() < len {
        let mut d = Sha256::new();
        d.update(&k_mpint);
        d.update(h);
        d.update(&out);
        let more = d.finalize();
        out.extend_from_slice(&more);
    }
    out.truncate(len);
    out
}

/// The cipher keys of the two directions.
pub(crate) struct SessionKeys {
    /// Client to server, the letter `C`.
    pub(crate) c2s: Keys,
    /// Server to client, the letter `D`.
    pub(crate) s2c: Keys,
}

/// Derives the cipher keys of a session. `session_id` is the exchange hash of
/// the first key exchange, which is also `h` in the module because it runs
/// one exchange per session.
pub(crate) fn session_keys(
    k: &[u8; 32],
    h: &[u8; 32],
    session_id: &[u8; 32],
) -> Result<SessionKeys, FnError> {
    let make = |letter| {
        Keys::from_material(&derive(k, h, letter, session_id, 64))
            .ok_or_else(|| protocol("key derivation returned too few bytes"))
    };
    Ok(SessionKeys {
        c2s: make(b'C')?,
        s2c: make(b'D')?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn rng() -> rand_pcg::Pcg64Mcg {
        rand_pcg::Pcg64Mcg::seed_from_u64(5)
    }

    #[test]
    fn kexinit_parses_with_the_checker() {
        let k = kexinit(&mut rng());
        check_kexinit(&k).unwrap();
        // 1 type, 16 cookie, 10 lists, 1 flag, 4 reserved.
        let mut r = Reader::new(&k);
        r.skip(17, "head").unwrap();
        for _ in 0..10 {
            r.string("list").unwrap();
        }
        assert_eq!(r.rest().len(), 5);
    }

    #[test]
    fn the_checker_refuses_other_algorithms() {
        let mut out = vec![MSG_KEXINIT];
        out.extend_from_slice(&[0; 16]);
        for list in [
            "diffie-hellman-group14-sha256",
            HOST_KEY,
            CIPHER,
            CIPHER,
            "",
            "",
            "none",
            "none",
            "",
            "",
        ] {
            put_string(&mut out, list.as_bytes());
        }
        put_bool(&mut out, false);
        put_u32(&mut out, 0);
        let err = check_kexinit(&out).unwrap_err();
        assert!(err.message.contains("key exchange"), "{err}");
        assert!(check_kexinit(&out[..30]).is_err());
        assert!(check_kexinit(&[]).is_err());
    }

    #[test]
    fn keys_are_64_bytes_and_depend_on_the_letter() {
        let (k, h, id) = ([1u8; 32], [2u8; 32], [3u8; 32]);
        let keys = session_keys(&k, &h, &id).unwrap();
        assert_ne!(keys.c2s.main, keys.s2c.main);
        assert_ne!(keys.c2s.main, keys.c2s.header);
        let again = session_keys(&k, &h, &id).unwrap();
        assert_eq!(keys.c2s.main, again.c2s.main);
        let other = session_keys(&[9u8; 32], &h, &id).unwrap();
        assert_ne!(keys.c2s.main, other.c2s.main);
    }

    #[test]
    fn the_exchange_hash_covers_every_field() {
        let base = Exchange {
            v_c: b"SSH-2.0-a",
            v_s: b"SSH-2.0-b",
            i_c: b"ic",
            i_s: b"is",
            k_s: b"ks",
            q_c: &[1; 32],
            q_s: &[2; 32],
            k: &[3; 32],
        };
        let h = base.hash();
        let q = [9u8; 32];
        for changed in [
            Exchange {
                v_c: b"SSH-2.0-x",
                ..base
            },
            Exchange { i_s: b"ix", ..base },
            Exchange { q_c: &q, ..base },
            Exchange { k: &q, ..base },
        ] {
            assert_ne!(changed.hash(), h);
        }
    }
}
