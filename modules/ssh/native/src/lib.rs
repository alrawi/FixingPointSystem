// SPDX-License-Identifier: GPL-3.0-only
//! Native template functions of the `ssh` module.
//!
//! Decision D10 lets first party modules implement their template functions in
//! Rust. This crate implements the `ssh` functions behind the [`TemplateFn`]
//! trait of `fps-flow`, and it runs a real SSH 2.0 transport: the two sides of
//! a scenario exchange identification lines and `SSH_MSG_KEXINIT`, run an
//! X25519 key exchange (`curve25519-sha256`, RFC 8731) in which the server
//! signs the exchange hash with an `ssh-ed25519` host key and the client
//! verifies the signature, derive session keys, and from `SSH_MSG_NEWKEYS` on
//! protect every packet with `chacha20-poly1305@openssh.com`. A side that
//! receives a packet with a wrong tag fails its session, so a session that
//! completes proves that both sides derived the same keys.
//!
//! The scenario language has no hook for received bytes, so a scenario drives
//! the state machine with three actions for every packet that it receives:
//!
//! ```yaml
//! - expect: { bytes: 4, timeout_ms: 10000 }
//! - expect: { bytes: "{{ ssh.size(last) }}", timeout_ms: 10000 }
//! - set: { got: "{{ ssh.recv(last, 'kexinit') }}" }
//! ```
//!
//! The first `expect` reads the head of the packet, which holds the length in
//! the clear or encrypted. `ssh.size` decrypts the length and returns the
//! number of bytes that follow. The second `expect` reads them, and
//! `ssh.recv` checks the tag, decrypts, requires the named message, and
//! applies it to the session. A packet is sent with
//! `send: "{{ ssh.kexinit() }}"` and the other message functions.
//!
//! One [`Ssh`] value serves one session, because it holds the keys and the
//! packet sequence numbers of that session. The SSH library of the module is
//! the RustCrypto `ssh-key` crate for the host key, its signatures, and the
//! public key blob. The packet protocol, the key exchange, and the key
//! derivation are written here, because every Rust SSH implementation that we
//! found owns its socket and cannot be driven one packet at a time from a flow
//! program.
//!
//! [`FUNCTIONS`] is the catalog that `modules/ssh/manifest.yaml` lists under
//! `functions`. The catalog test compares the two, so the manifest stays the
//! single catalog that the validator and the controller read.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod args;
pub mod catalog;
mod kex;
mod msg;
mod names;
mod rng;
mod transport;
mod wire;

use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use rand::RngCore;
use sha2::{Digest, Sha256};
use ssh_encoding::Encode;
use ssh_key::private::Ed25519Keypair;
use ssh_key::public::KeyData;
use ssh_key::{PublicKey, Signature};
use signature::{Signer, Verifier};
use x25519_dalek::{PublicKey as XPublic, StaticSecret};

pub use catalog::{DefaultValue, FunctionSpec, ParamSpec, FUNCTIONS, MAX_CHUNK};

use crate::kex::{Exchange, SessionKeys};
use crate::transport::Half;
use crate::wire::{protocol, put_string, Reader};

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "ssh";

/// The identification prefix of protocol version 2.0.
const BANNER_PREFIX: &str = "SSH-2.0-";

/// Which end of the connection a session is. The role decides which key
/// protects which direction, and it follows from the first key exchange
/// message that a side sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Client,
    Server,
}

/// The template functions of the `ssh` module for one session.
///
/// The value holds the state of the SSH transport of the session: the
/// identification lines, the key exchange messages, the keys, and the packet
/// sequence number of each direction. Use one value per session and side.
#[derive(Default)]
pub struct Ssh {
    own_banner: Option<Vec<u8>>,
    peer_banner: Option<Vec<u8>>,
    own_kexinit: Option<Vec<u8>>,
    peer_kexinit: Option<Vec<u8>>,
    role: Option<Role>,
    /// The ephemeral secret of the client, until the reply arrives.
    secret: Option<StaticSecret>,
    /// The client public key: the own one on a client, the peer's on a server.
    q_c: Option<[u8; 32]>,
    keys: Option<SessionKeys>,
    out: Half,
    inn: Half,
    /// The head of the packet that `size` read and `recv` has not opened yet.
    head: Option<[u8; 4]>,
}

fn invalid(message: String) -> FnError {
    FnError::new(FnErrorKind::InvalidArgument, message)
}

fn internal(message: String) -> FnError {
    FnError::new(FnErrorKind::Internal, message)
}

/// Mixes the function name into the seed, so two functions called with one
/// seed do not share a random sequence.
fn derive(function: &str, seed: i64, rng: &mut dyn RngCore) -> rng::Rng {
    let base = if seed >= 0 {
        seed as u64
    } else {
        rng.next_u64()
    };
    let salt = function.bytes().fold(0xCBF2_9CE4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3)
    });
    rng::Rng::new(base ^ salt)
}

fn push(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
}

/// An unsigned 32 bit window or count that a call passed as `i64`.
fn u32_arg(function: &str, name: &str, v: i64) -> Result<u32, FnError> {
    u32::try_from(v).map_err(|_| {
        invalid(format!(
            "'{name}' of '{function}' must lie between 0 and {}, not {v}",
            u32::MAX
        ))
    })
}

/// A `bytes` argument that must be text, such as the result of a nested call.
fn utf8<'a>(name: &str, function: &str, bytes: &'a [u8]) -> Result<&'a str, FnError> {
    std::str::from_utf8(bytes)
        .map_err(|_| invalid(format!("'{name}' of '{function}' must be UTF-8 text")))
}

/// The ed25519 host key of `seed`. A server of one run uses one seed, so every
/// session shows one host key, as a real server does.
fn host_key(seed: i64) -> Ed25519Keypair {
    let mut d = Sha256::new();
    d.update(b"fps-ssh-host-key");
    d.update(seed.to_be_bytes());
    let seed: [u8; 32] = d.finalize().into();
    Ed25519Keypair::from_seed(&seed)
}

/// Checks a banner argument: printable ASCII, a software version without
/// spaces and hyphens, and a line that fits the 255 bytes of RFC 4253.
fn check_software(software: &str) -> Result<(), FnError> {
    let version = software.split(' ').next().unwrap_or("");
    if version.is_empty()
        || version.contains('-')
        || !software.bytes().all(|b| (0x20..0x7f).contains(&b))
        || BANNER_PREFIX.len() + software.len() + 2 > 255
    {
        return Err(invalid(format!(
            "'software' of 'banner' must be printable ASCII, start with a version without '-' or spaces, and fit 255 bytes, not '{software}'"
        )));
    }
    Ok(())
}

impl Ssh {
    fn need<T: Clone>(what: &str, v: &Option<T>) -> Result<T, FnError> {
        v.clone().ok_or_else(|| {
            protocol(format!(
                "the session has no {what} yet, the scenario calls the key exchange in the wrong order"
            ))
        })
    }

    /// Seals `payload` into the next packet of the outgoing direction.
    fn send(&mut self, payload: &[u8], rng: &mut dyn RngCore, out: &mut Vec<u8>) {
        self.out.seal(payload, rng, out);
    }

    fn ephemeral(rng: &mut dyn RngCore) -> (StaticSecret, [u8; 32]) {
        let mut raw = [0u8; 32];
        rng.fill_bytes(&mut raw);
        let secret = StaticSecret::from(raw);
        let public = XPublic::from(&secret).to_bytes();
        (secret, public)
    }

    fn ecdh_init(&mut self, rng: &mut dyn RngCore, out: &mut Vec<u8>) {
        let (secret, public) = Self::ephemeral(rng);
        self.role = Some(Role::Client);
        self.secret = Some(secret);
        self.q_c = Some(public);
        let mut p = vec![msg::ECDH_INIT];
        put_string(&mut p, &public);
        self.send(&p, rng, out);
    }

    fn ecdh_reply(
        &mut self,
        host_seed: i64,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        let q_c = Self::need("client key from SSH_MSG_KEX_ECDH_INIT", &self.q_c)?;
        let v_c = Self::need("client banner", &self.peer_banner)?;
        let v_s = Self::need("server banner", &self.own_banner)?;
        let i_c = Self::need("client SSH_MSG_KEXINIT", &self.peer_kexinit)?;
        let i_s = Self::need("server SSH_MSG_KEXINIT", &self.own_kexinit)?;
        let (secret, q_s) = Self::ephemeral(rng);
        let shared = secret.diffie_hellman(&XPublic::from(q_c));
        if !shared.was_contributory() {
            return Err(protocol("the client key is a point of small order"));
        }
        let k = shared.to_bytes();
        let pair = host_key(host_seed);
        let mut k_s = Vec::new();
        KeyData::Ed25519(pair.public)
            .encode(&mut k_s)
            .map_err(|e| internal(format!("encoding the host key failed: {e}")))?;
        let h = Exchange {
            v_c: &v_c,
            v_s: &v_s,
            i_c: &i_c,
            i_s: &i_s,
            k_s: &k_s,
            q_c: &q_c,
            q_s: &q_s,
            k: &k,
        }
        .hash();
        let signature: Signature = pair
            .try_sign(&h)
            .map_err(|e| internal(format!("signing the exchange hash failed: {e}")))?;
        let blob = Vec::<u8>::try_from(signature)
            .map_err(|e| internal(format!("encoding the signature failed: {e}")))?;
        self.keys = Some(kex::session_keys(&k, &h, &h)?);
        self.role = Some(Role::Server);
        let mut p = vec![msg::ECDH_REPLY];
        put_string(&mut p, &k_s);
        put_string(&mut p, &q_s);
        put_string(&mut p, &blob);
        self.send(&p, rng, out);
        Ok(())
    }

    fn newkeys(&mut self, rng: &mut dyn RngCore, out: &mut Vec<u8>) -> Result<(), FnError> {
        let role = Self::need("key exchange result", &self.role)?;
        let keys = self
            .keys
            .as_ref()
            .ok_or_else(|| protocol("newkeys was called before the key exchange finished"))?;
        let mine = match role {
            Role::Client => keys.c2s.clone(),
            Role::Server => keys.s2c.clone(),
        };
        self.send(&[msg::NEWKEYS], rng, out);
        self.out.switch(mine);
        Ok(())
    }

    /// Handles `SSH_MSG_KEX_ECDH_REPLY` on the client: verifies the host key
    /// signature over the exchange hash and derives the session keys.
    fn client_finish(&mut self, r: &mut Reader<'_>) -> Result<(), FnError> {
        if self.role != Some(Role::Client) {
            return Err(protocol(
                "SSH_MSG_KEX_ECDH_REPLY arrived on a side that sent no SSH_MSG_KEX_ECDH_INIT",
            ));
        }
        let k_s = r.string("the host key")?;
        let q_s: [u8; 32] = r
            .string("the server key")?
            .try_into()
            .map_err(|_| protocol("the server key is not 32 bytes"))?;
        let sig_blob = r.string("the signature")?;
        let secret = self
            .secret
            .take()
            .ok_or_else(|| protocol("the key exchange reply arrived twice"))?;
        let q_c = Self::need("client key", &self.q_c)?;
        let v_c = Self::need("client banner", &self.own_banner)?;
        let v_s = Self::need("server banner", &self.peer_banner)?;
        let i_c = Self::need("client SSH_MSG_KEXINIT", &self.own_kexinit)?;
        let i_s = Self::need("server SSH_MSG_KEXINIT", &self.peer_kexinit)?;
        let shared = secret.diffie_hellman(&XPublic::from(q_s));
        if !shared.was_contributory() {
            return Err(protocol("the server key is a point of small order"));
        }
        let k = shared.to_bytes();
        let h = Exchange {
            v_c: &v_c,
            v_s: &v_s,
            i_c: &i_c,
            i_s: &i_s,
            k_s,
            q_c: &q_c,
            q_s: &q_s,
            k: &k,
        }
        .hash();
        let public = PublicKey::from_bytes(k_s)
            .map_err(|e| protocol(format!("the host key blob does not parse: {e}")))?;
        let ed = public
            .key_data()
            .ed25519()
            .ok_or_else(|| protocol("the host key is not an ssh-ed25519 key"))?;
        let signature = Signature::try_from(sig_blob)
            .map_err(|e| protocol(format!("the signature blob does not parse: {e}")))?;
        ed.verify(&h, &signature)
            .map_err(|_| protocol("the signature of the exchange hash does not verify"))?;
        self.keys = Some(kex::session_keys(&k, &h, &h)?);
        Ok(())
    }

    /// Opens the packet after its head and applies the message.
    fn recv(&mut self, packet: &[u8], expect: &str) -> Result<usize, FnError> {
        let want = msg::expected(expect)?;
        let head = self
            .head
            .take()
            .ok_or_else(|| protocol("recv was called without a size call before it"))?;
        let payload = self.inn.open(&head, packet)?;
        let code = *payload
            .first()
            .ok_or_else(|| protocol("the packet holds no message"))?;
        if let Some(w) = want {
            if w != code {
                return Err(protocol(format!(
                    "expected {}, received {}",
                    msg::name_of(w),
                    msg::name_of(code)
                )));
            }
        }
        let mut r = Reader::new(&payload[1..]);
        match code {
            msg::KEXINIT => {
                kex::check_kexinit(&payload)?;
                self.peer_kexinit = Some(payload);
                Ok(0)
            }
            msg::ECDH_INIT => {
                let q: [u8; 32] = r
                    .string("the client key")?
                    .try_into()
                    .map_err(|_| protocol("the client key is not 32 bytes"))?;
                self.q_c = Some(q);
                Ok(0)
            }
            msg::ECDH_REPLY => {
                self.client_finish(&mut r)?;
                Ok(0)
            }
            msg::NEWKEYS => {
                let role = Self::need("key exchange result", &self.role)?;
                let keys = self
                    .keys
                    .as_ref()
                    .ok_or_else(|| protocol("SSH_MSG_NEWKEYS arrived before the key exchange"))?;
                let theirs = match role {
                    Role::Client => keys.s2c.clone(),
                    Role::Server => keys.c2s.clone(),
                };
                self.inn.switch(theirs);
                Ok(0)
            }
            msg::USERAUTH_REQUEST => {
                for what in ["the user name", "the service", "the method"] {
                    r.string(what)?;
                }
                Ok(0)
            }
            msg::CHANNEL_CONFIRM
            | msg::WINDOW_ADJUST
            | msg::CHANNEL_DATA
            | msg::CHANNEL_EOF
            | msg::CHANNEL_CLOSE
            | msg::CHANNEL_REQUEST
            | msg::CHANNEL_SUCCESS => {
                let channel = r.u32("the channel number")?;
                if channel != 0 {
                    return Err(protocol(format!(
                        "the packet addresses channel {channel}, the module opens only channel 0"
                    )));
                }
                if code == msg::CHANNEL_DATA {
                    Ok(r.string("the data")?.len())
                } else {
                    Ok(0)
                }
            }
            _ => Ok(0),
        }
    }
}

/// Parses the identification line of the peer and returns what follows
/// `SSH-2.0-`.
fn parse_banner(line: &[u8]) -> Result<(Vec<u8>, String), FnError> {
    let body = line
        .strip_suffix(b"\r\n")
        .or_else(|| line.strip_suffix(b"\n"))
        .ok_or_else(|| protocol("the identification line does not end in a line break"))?;
    if line.len() > 255 {
        return Err(protocol("the identification line is longer than 255 bytes"));
    }
    let text = std::str::from_utf8(body)
        .map_err(|_| protocol("the identification line is not text"))?;
    let software = text
        .strip_prefix(BANNER_PREFIX)
        .or_else(|| text.strip_prefix("SSH-1.99-"))
        .ok_or_else(|| {
            protocol("the identification line does not start with SSH-2.0-, SSH is not spoken")
        })?;
    Ok((body.to_vec(), software.to_string()))
}

impl TemplateFn for Ssh {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        if call.module != MODULE_NAME {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("module '{}' is not ssh", call.module),
            ));
        }
        use catalog as c;
        match call.function {
            "credential" => {
                let a = args::bind(&c::CREDENTIAL, &call.args)?;
                let mut r = derive("credential", a.int("seed")?, rng);
                match a.text("field")? {
                    "user" => push(out, &names::user(&mut r)),
                    "password" => push(out, &names::password(&mut r)),
                    other => {
                        return Err(invalid(format!(
                            "field '{other}' of 'credential' is not supported, use user or password"
                        )))
                    }
                }
            }
            "file_name" => {
                let a = args::bind(&c::FILE_NAME, &call.args)?;
                let ext = a.text("ext")?;
                if ext.is_empty()
                    || ext.len() > 8
                    || !ext
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.')
                {
                    return Err(invalid(format!(
                        "'ext' of 'file_name' must be 1 to 8 lower case letters, digits, or dots, not '{ext}'"
                    )));
                }
                let seed = a.int("seed")?;
                if seed < 0 {
                    return Err(invalid(format!(
                        "'seed' of 'file_name' must not be negative, not {seed}"
                    )));
                }
                let mut r = derive("file_name", seed, rng);
                push(out, &names::file_name(&mut r, ext));
            }
            "command" => {
                let a = args::bind(&c::COMMAND, &call.args)?;
                let mut r = derive("command", a.int("seed")?, rng);
                push(out, names::command(&mut r));
            }
            "banner" => {
                let a = args::bind(&c::BANNER, &call.args)?;
                let software = a.text("software")?;
                check_software(software)?;
                let line = format!("{BANNER_PREFIX}{software}");
                out.extend_from_slice(line.as_bytes());
                out.extend_from_slice(b"\r\n");
                self.own_banner = Some(line.into_bytes());
            }
            "peer_banner" => {
                let a = args::bind(&c::PEER_BANNER, &call.args)?;
                let (line, software) = parse_banner(a.bytes("line")?)?;
                self.peer_banner = Some(line);
                push(out, &software);
            }
            "kexinit" => {
                args::bind(&c::KEXINIT, &call.args)?;
                let payload = kex::kexinit(rng);
                self.send(&payload, rng, out);
                self.own_kexinit = Some(payload);
            }
            "ecdh_init" => {
                args::bind(&c::ECDH_INIT, &call.args)?;
                self.ecdh_init(rng, out);
            }
            "ecdh_reply" => {
                let a = args::bind(&c::ECDH_REPLY, &call.args)?;
                self.ecdh_reply(a.int("host_key_seed")?, rng, out)?;
            }
            "newkeys" => {
                args::bind(&c::NEWKEYS, &call.args)?;
                self.newkeys(rng, out)?;
            }
            "size" => {
                let a = args::bind(&c::SIZE, &call.args)?;
                let head: [u8; 4] = a.bytes("head")?.try_into().map_err(|_| {
                    protocol(format!(
                        "'head' of 'size' must be the first 4 bytes of a packet, not {} bytes",
                        a.bytes("head").map_or(0, <[u8]>::len)
                    ))
                })?;
                let n = self.inn.body_len(&head)?;
                self.head = Some(head);
                push(out, &n.to_string());
            }
            "recv" => {
                let a = args::bind(&c::RECV, &call.args)?;
                let n = self.recv(a.bytes("packet")?, a.text("expect")?)?;
                push(out, &n.to_string());
            }
            "service_request" => {
                let a = args::bind(&c::SERVICE_REQUEST, &call.args)?;
                let p = msg::service(msg::SERVICE_REQUEST, a.text("name")?);
                self.send(&p, rng, out);
            }
            "service_accept" => {
                let a = args::bind(&c::SERVICE_ACCEPT, &call.args)?;
                let p = msg::service(msg::SERVICE_ACCEPT, a.text("name")?);
                self.send(&p, rng, out);
            }
            "auth_none" => {
                let a = args::bind(&c::AUTH_NONE, &call.args)?;
                let p = msg::userauth_none(a.text("user")?);
                self.send(&p, rng, out);
            }
            "auth_password" => {
                let a = args::bind(&c::AUTH_PASSWORD, &call.args)?;
                let password = utf8("password", "auth_password", a.bytes("password")?)?;
                let p = msg::userauth_password(a.text("user")?, password);
                self.send(&p, rng, out);
            }
            "auth_failure" => {
                let a = args::bind(&c::AUTH_FAILURE, &call.args)?;
                let p = msg::userauth_failure(a.text("methods")?);
                self.send(&p, rng, out);
            }
            "auth_success" => {
                args::bind(&c::AUTH_SUCCESS, &call.args)?;
                self.send(&[msg::USERAUTH_SUCCESS], rng, out);
            }
            "channel_open" | "channel_confirm" => {
                let spec = if call.function == "channel_open" {
                    &c::CHANNEL_OPEN
                } else {
                    &c::CHANNEL_CONFIRM
                };
                let a = args::bind(spec, &call.args)?;
                let window = u32_arg(call.function, "window", a.int("window")?)?;
                let max = u32_arg(call.function, "max_packet", a.int("max_packet")?)?;
                if max == 0 || i64::from(max) > MAX_CHUNK {
                    return Err(invalid(format!(
                        "'max_packet' of '{}' must lie between 1 and {MAX_CHUNK}, not {max}",
                        call.function
                    )));
                }
                let p = if call.function == "channel_open" {
                    msg::channel_open(window, max)
                } else {
                    msg::channel_confirm(window, max)
                };
                self.send(&p, rng, out);
            }
            "channel_request" => {
                let a = args::bind(&c::CHANNEL_REQUEST, &call.args)?;
                let command = utf8("command", "channel_request", a.bytes("command")?)?;
                let p = msg::channel_request(a.text("kind")?, command, a.boolean("reply")?)?;
                self.send(&p, rng, out);
            }
            "channel_success" => {
                args::bind(&c::CHANNEL_SUCCESS, &call.args)?;
                self.send(&msg::channel_only(msg::CHANNEL_SUCCESS), rng, out);
            }
            "exit_status" => {
                let a = args::bind(&c::EXIT_STATUS, &call.args)?;
                let code = u32_arg("exit_status", "code", a.int("code")?)?;
                self.send(&msg::exit_status(code), rng, out);
            }
            "channel_data" => {
                let a = args::bind(&c::CHANNEL_DATA, &call.args)?;
                let data = a.bytes("data")?;
                if data.len() as i64 > MAX_CHUNK {
                    return Err(invalid(format!(
                        "'data' of 'channel_data' holds {} bytes, more than the {MAX_CHUNK} of one packet",
                        data.len()
                    )));
                }
                self.send(&msg::channel_data(data), rng, out);
            }
            "window_adjust" => {
                let a = args::bind(&c::WINDOW_ADJUST, &call.args)?;
                let n = u32_arg("window_adjust", "count", a.int("count")?)?;
                self.send(&msg::window_adjust(n), rng, out);
            }
            "channel_eof" => {
                args::bind(&c::CHANNEL_EOF, &call.args)?;
                self.send(&msg::channel_only(msg::CHANNEL_EOF), rng, out);
            }
            "channel_close" => {
                args::bind(&c::CHANNEL_CLOSE, &call.args)?;
                self.send(&msg::channel_only(msg::CHANNEL_CLOSE), rng, out);
            }
            "disconnect" => {
                let a = args::bind(&c::DISCONNECT, &call.args)?;
                self.send(&msg::disconnect(a.text("reason")?), rng, out);
            }
            "chunk" => {
                let a = args::bind(&c::CHUNK, &call.args)?;
                let size = a.int("size")?;
                if !(0..=MAX_CHUNK).contains(&size) {
                    return Err(invalid(format!(
                        "'size' of 'chunk' must lie between 0 and {MAX_CHUNK}, not {size}"
                    )));
                }
                let size = size as usize;
                match a.text("kind")? {
                    "text" => out.extend_from_slice(&names::text(&mut derive("chunk", -1, rng), size)),
                    "keys" => out.extend_from_slice(&names::keys(&mut derive("chunk", -1, rng), size)),
                    "binary" => {
                        let at = out.len();
                        out.resize(at + size, 0);
                        rng.fill_bytes(&mut out[at..]);
                    }
                    other => {
                        return Err(invalid(format!(
                            "kind '{other}' of 'chunk' is not supported, use text, keys, or binary"
                        )))
                    }
                }
            }
            "scp_header" => {
                let a = args::bind(&c::SCP_HEADER, &call.args)?;
                let (size, name, mode) = (a.int("size")?, a.text("name")?, a.text("mode")?);
                if size < 0 {
                    return Err(invalid(format!(
                        "'size' of 'scp_header' must not be negative, not {size}"
                    )));
                }
                if mode.len() != 4 || !mode.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
                    return Err(invalid(format!(
                        "'mode' of 'scp_header' must be four octal digits, not '{mode}'"
                    )));
                }
                if name.is_empty()
                    || name.contains('/')
                    || !name.bytes().all(|b| b.is_ascii_graphic())
                {
                    return Err(invalid(format!(
                        "'name' of 'scp_header' must be printable ASCII without spaces or slashes, not '{name}'"
                    )));
                }
                push(out, &format!("C{mode} {size} {name}\n"));
            }
            "div" | "rem" => {
                let spec = if call.function == "div" { &c::DIV } else { &c::REM };
                let a = args::bind(spec, &call.args)?;
                let (x, y) = (a.int("a")?, a.int("b")?);
                if x < 0 || y <= 0 {
                    return Err(invalid(format!(
                        "'{}' needs a dividend of 0 or more and a divisor above 0, not {x} and {y}",
                        call.function
                    )));
                }
                let n = if call.function == "div" { x / y } else { x % y };
                push(out, &n.to_string());
            }
            "flag" => {
                let a = args::bind(&c::FLAG, &call.args)?;
                push(out, if a.int("n")? > 0 { "1" } else { "0" });
            }
            other => {
                return Err(FnError::new(
                    FnErrorKind::Unsupported,
                    format!("ssh has no function '{other}'"),
                ))
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
