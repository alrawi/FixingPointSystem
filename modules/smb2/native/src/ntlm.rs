// SPDX-License-Identifier: GPL-3.0-only
//! Synthetic NTLM exchanges: the three NTLMSSP messages of MS-NLMP, the NTLMv2
//! responses, and the SPNEGO tokens of RFC 4178 that carry them in the security
//! buffers of SESSION_SETUP.
//!
//! The exchange is synthetic because both sides of a scenario come from this
//! module and share a user name and a password that the scenario names. The
//! messages are the ones a Windows peer writes, and the NTLMv2 response is
//! computed in full, so a capture analyzer that knows the password verifies it.
//! Three things stay out. The authenticate message carries no `MIC`, key
//! exchange is off so the exported session key equals the session base key, and
//! NTLMv1 does not exist.

use hmac::{Hmac, Mac};
use md4::{Digest, Md4};
use md5::Md5;

use crate::wire::{from_utf16le, invalid, slice, u16_at, u32_at, u64_at, utf16le, WireError};

/// The signature that starts every NTLMSSP message.
pub const SIGNATURE: &[u8; 8] = b"NTLMSSP\0";

/// `NTLMSSP_NEGOTIATE_UNICODE`.
pub const NEG_UNICODE: u32 = 0x0000_0001;
/// `NTLMSSP_NEGOTIATE_KEY_EXCH`.
pub const NEG_KEY_EXCH: u32 = 0x4000_0000;

/// What a Windows client offers in its negotiate message.
pub const DEFAULT_NEGOTIATE_FLAGS: u32 = 0xE208_8297;
/// What a Windows server answers in its challenge message.
pub const DEFAULT_CHALLENGE_FLAGS: u32 = 0xE289_8215;

/// The object identifier of NTLMSSP, 1.3.6.1.4.1.311.2.2.10.
const OID_NTLMSSP: [u8; 10] = [0x2B, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x02, 0x02, 0x0A];
/// The object identifier of SPNEGO, 1.3.6.1.5.5.2.
const OID_SPNEGO: [u8; 6] = [0x2B, 0x06, 0x01, 0x05, 0x05, 0x02];

/// The `Version` field that every message carries, which says Windows 10.
const VERSION: [u8; 8] = [10, 0, 0x61, 0x4A, 0, 0, 0, 0x0F];

type HmacMd5 = Hmac<Md5>;

fn hmac_md5(key: &[u8], parts: &[&[u8]]) -> [u8; 16] {
    // HMAC accepts a key of any length, so the construction cannot fail.
    let mut mac = match <HmacMd5 as Mac>::new_from_slice(key) {
        Ok(m) => m,
        Err(_) => return [0; 16],
    };
    for p in parts {
        mac.update(p);
    }
    mac.finalize().into_bytes().into()
}

/// The NT hash of a password, MD4 over its UTF-16LE form.
pub fn nt_hash(password: &str) -> [u8; 16] {
    Md4::digest(utf16le(password)).into()
}

/// `NTOWFv2` of MS-NLMP section 3.3.2: HMAC-MD5 over the upper case user name
/// and the domain, keyed with the NT hash.
pub fn ntowf_v2(password: &str, user: &str, domain: &str) -> [u8; 16] {
    let identity = utf16le(&format!("{}{}", user.to_uppercase(), domain));
    hmac_md5(&nt_hash(password), &[&identity])
}

// ---------------------------------------------------------------------------
// DER and SPNEGO
// ---------------------------------------------------------------------------

fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    match content.len() {
        n if n < 0x80 => out.push(n as u8),
        n if n <= 0xFF => out.extend_from_slice(&[0x81, n as u8]),
        n if n <= 0xFFFF => out.extend_from_slice(&[0x82, (n >> 8) as u8, n as u8]),
        n => out.extend_from_slice(&[0x83, (n >> 16) as u8, (n >> 8) as u8, n as u8]),
    }
    out.extend_from_slice(content);
    out
}

/// The `negState` values of RFC 4178 that the module writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NegState {
    /// The exchange is complete.
    AcceptCompleted = 0,
    /// The exchange needs another token.
    AcceptIncomplete = 1,
    /// The acceptor refuses the exchange.
    Reject = 2,
}

/// The initial context token of RFC 4178: a GSS-API wrapper around a
/// `NegTokenInit` that offers NTLMSSP and carries `token` as the optimistic
/// mechanism token, or no mechanism token when `token` is `None`, which is what
/// a server sends in its negotiate response.
pub fn spnego_init(token: Option<&[u8]>) -> Vec<u8> {
    let mechs = der(0xA0, &der(0x30, &der(0x06, &OID_NTLMSSP)));
    let mut seq = mechs;
    if let Some(t) = token {
        seq.extend_from_slice(&der(0xA2, &der(0x04, t)));
    }
    let init = der(0xA0, &der(0x30, &seq));
    let mut body = der(0x06, &OID_SPNEGO);
    body.extend_from_slice(&init);
    der(0x60, &body)
}

/// A `NegTokenResp` with the given state, the NTLMSSP mechanism when `mech` is
/// set, and the response token.
pub fn spnego_resp(state: Option<NegState>, mech: bool, token: Option<&[u8]>) -> Vec<u8> {
    let mut seq = Vec::new();
    if let Some(s) = state {
        seq.extend_from_slice(&der(0xA0, &der(0x0A, &[s as u8])));
    }
    if mech {
        seq.extend_from_slice(&der(0xA1, &der(0x06, &OID_NTLMSSP)));
    }
    if let Some(t) = token {
        seq.extend_from_slice(&der(0xA2, &der(0x04, t)));
    }
    der(0xA1, &der(0x30, &seq))
}

/// Finds the NTLMSSP message inside a security buffer, which is a SPNEGO token
/// or a bare NTLMSSP message.
pub fn find_message(security: &[u8]) -> Result<&[u8], WireError> {
    security
        .windows(SIGNATURE.len())
        .position(|w| w == SIGNATURE)
        .map(|p| &security[p..])
        .ok_or_else(|| invalid("the security buffer holds no NTLMSSP message"))
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// A security buffer field of an NTLMSSP message, which is a length, a maximum
/// length, and an offset from the start of the message.
struct Field {
    len: u16,
    offset: u32,
}

impl Field {
    fn write(out: &mut Vec<u8>, len: usize, offset: usize) {
        out.extend_from_slice(&(len as u16).to_le_bytes());
        out.extend_from_slice(&(len as u16).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
    }

    fn read(msg: &[u8], at: usize) -> Result<Field, WireError> {
        Ok(Field {
            len: u16_at(msg, at, "an NTLM field length")?,
            offset: u32_at(msg, at + 4, "an NTLM field offset")?,
        })
    }

    fn bytes<'a>(&self, msg: &'a [u8], what: &'static str) -> Result<&'a [u8], WireError> {
        slice(msg, self.offset as usize, usize::from(self.len), what)
    }
}

fn check_text(what: &str, s: &str) -> Result<(), WireError> {
    if s.len() > 255 {
        return Err(invalid(format!("{what} has more than 255 characters")));
    }
    Ok(())
}

/// The NEGOTIATE_MESSAGE of a client.
pub fn negotiate_message(flags: u32, domain: &str, workstation: &str) -> Result<Vec<u8>, WireError> {
    check_text("the domain", domain)?;
    check_text("the workstation", workstation)?;
    let mut flags = flags;
    // The domain and workstation fields are valid only with their flags.
    if !domain.is_empty() {
        flags |= 0x1000;
    }
    if !workstation.is_empty() {
        flags |= 0x2000;
    }
    let mut m = Vec::new();
    m.extend_from_slice(SIGNATURE);
    m.extend_from_slice(&1u32.to_le_bytes());
    m.extend_from_slice(&flags.to_le_bytes());
    Field::write(&mut m, domain.len(), 40);
    Field::write(&mut m, workstation.len(), 40 + domain.len());
    m.extend_from_slice(&VERSION);
    m.extend_from_slice(domain.as_bytes());
    m.extend_from_slice(workstation.as_bytes());
    Ok(m)
}

/// The names that a server puts in the target information of its challenge.
#[derive(Clone, Copy, Debug, Default)]
pub struct TargetNames<'a> {
    /// The NetBIOS domain name.
    pub domain: &'a str,
    /// The NetBIOS computer name.
    pub computer: &'a str,
    /// The DNS domain name, or empty.
    pub dns_domain: &'a str,
    /// The DNS computer name, or empty.
    pub dns_computer: &'a str,
}

fn av_pair(out: &mut Vec<u8>, id: u16, value: &[u8]) {
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&(value.len() as u16).to_le_bytes());
    out.extend_from_slice(value);
}

/// Builds the `AV_PAIR` list of a challenge message.
pub fn target_info(names: &TargetNames<'_>, time: u64) -> Vec<u8> {
    let mut out = Vec::new();
    for (id, text) in [
        (2u16, names.domain),
        (1, names.computer),
        (4, names.dns_domain),
        (3, names.dns_computer),
    ] {
        if !text.is_empty() {
            av_pair(&mut out, id, &utf16le(text));
        }
    }
    av_pair(&mut out, 7, &time.to_le_bytes());
    av_pair(&mut out, 0, &[]);
    out
}

/// The CHALLENGE_MESSAGE of a server.
pub fn challenge_message(
    flags: u32,
    target: &str,
    server_challenge: [u8; 8],
    target_info: &[u8],
) -> Result<Vec<u8>, WireError> {
    check_text("the target name", target)?;
    if target_info.len() > usize::from(u16::MAX) {
        return Err(invalid("the target information is too long"));
    }
    let name = utf16le(target);
    let mut m = Vec::new();
    m.extend_from_slice(SIGNATURE);
    m.extend_from_slice(&2u32.to_le_bytes());
    Field::write(&mut m, name.len(), 56);
    m.extend_from_slice(&flags.to_le_bytes());
    m.extend_from_slice(&server_challenge);
    m.extend_from_slice(&[0u8; 8]);
    Field::write(&mut m, target_info.len(), 56 + name.len());
    m.extend_from_slice(&VERSION);
    m.extend_from_slice(&name);
    m.extend_from_slice(target_info);
    Ok(m)
}

/// The parts of a challenge message that the response needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Challenge {
    /// The flags of the server.
    pub flags: u32,
    /// The 8 octet server challenge.
    pub server_challenge: [u8; 8],
    /// The `AV_PAIR` list, with its end marker.
    pub target_info: Vec<u8>,
    /// The target name.
    pub target: String,
}

/// Reads a CHALLENGE_MESSAGE.
pub fn parse_challenge(msg: &[u8]) -> Result<Challenge, WireError> {
    check_header(msg, 2)?;
    let target = Field::read(msg, 12)?;
    let flags = u32_at(msg, 20, "the challenge flags")?;
    let mut sc = [0u8; 8];
    sc.copy_from_slice(slice(msg, 24, 8, "the server challenge")?);
    let info = Field::read(msg, 40)?;
    let name = target.bytes(msg, "the target name")?;
    Ok(Challenge {
        flags,
        server_challenge: sc,
        target_info: info.bytes(msg, "the target information")?.to_vec(),
        target: decode_text(name, flags)?,
    })
}

fn check_header(msg: &[u8], ty: u32) -> Result<(), WireError> {
    if slice(msg, 0, 8, "the NTLMSSP signature")? != SIGNATURE {
        return Err(invalid("the message does not start with the NTLMSSP signature"));
    }
    let got = u32_at(msg, 8, "the NTLM message type")?;
    if got != ty {
        return Err(invalid(format!(
            "the NTLM message has the type {got}, and the function needs the type {ty}"
        )));
    }
    Ok(())
}

fn decode_text(bytes: &[u8], flags: u32) -> Result<String, WireError> {
    if flags & NEG_UNICODE != 0 {
        from_utf16le(bytes, "an NTLM name")
    } else {
        Ok(bytes.iter().map(|b| char::from(*b)).collect())
    }
}

/// Looks up one `AV_PAIR` value.
pub fn av_value(list: &[u8], id: u16) -> Option<&[u8]> {
    let mut rest = list;
    while rest.len() >= 4 {
        let this = u16::from_le_bytes([rest[0], rest[1]]);
        let len = usize::from(u16::from_le_bytes([rest[2], rest[3]]));
        let value = rest.get(4..4 + len)?;
        if this == id {
            return Some(value);
        }
        if this == 0 {
            return None;
        }
        rest = &rest[4 + len..];
    }
    None
}

/// The credentials of the client.
#[derive(Clone, Copy, Debug)]
pub struct Credentials<'a> {
    /// The user name.
    pub user: &'a str,
    /// The password.
    pub password: &'a str,
    /// The domain of the user.
    pub domain: &'a str,
    /// The name of the client computer.
    pub workstation: &'a str,
}

/// The NTLMv2 response of MS-NLMP section 3.3.2: the proof and the client
/// blob. Returns the 16 octet proof followed by the blob, and the session base key.
pub fn ntlmv2_response(
    cred: &Credentials<'_>,
    server_challenge: &[u8; 8],
    client_challenge: &[u8; 8],
    time: u64,
    server_name: &[u8],
) -> (Vec<u8>, [u8; 16]) {
    let key = ntowf_v2(cred.password, cred.user, cred.domain);
    let mut temp = vec![1, 1, 0, 0, 0, 0, 0, 0];
    temp.extend_from_slice(&time.to_le_bytes());
    temp.extend_from_slice(client_challenge);
    temp.extend_from_slice(&[0; 4]);
    temp.extend_from_slice(server_name);
    temp.extend_from_slice(&[0; 4]);
    let proof = hmac_md5(&key, &[server_challenge, &temp]);
    let session_base_key = hmac_md5(&key, &[&proof]);
    let mut response = proof.to_vec();
    response.extend_from_slice(&temp);
    (response, session_base_key)
}

/// The AUTHENTICATE_MESSAGE of a client that answers `challenge`.
pub fn authenticate_message(
    challenge: &Challenge,
    cred: &Credentials<'_>,
    client_challenge: [u8; 8],
    client_time: u64,
) -> Result<Vec<u8>, WireError> {
    check_text("the user", cred.user)?;
    check_text("the domain", cred.domain)?;
    check_text("the workstation", cred.workstation)?;
    // The server time in the target information wins over the clock of the
    // client, as MS-NLMP section 3.1.5.1.2 says, and then the LM response is
    // 24 zero octets.
    let (time, lm) = match av_value(&challenge.target_info, 7) {
        Some(t) if t.len() == 8 => (
            u64::from_le_bytes([t[0], t[1], t[2], t[3], t[4], t[5], t[6], t[7]]),
            vec![0u8; 24],
        ),
        _ => {
            let key = ntowf_v2(cred.password, cred.user, cred.domain);
            let mut lm = hmac_md5(&key, &[&challenge.server_challenge, &client_challenge]).to_vec();
            lm.extend_from_slice(&client_challenge);
            (client_time, lm)
        }
    };
    let server_name: &[u8] = if challenge.target_info.is_empty() {
        &[0; 4]
    } else {
        &challenge.target_info
    };
    let (nt, _) = ntlmv2_response(cred, &challenge.server_challenge, &client_challenge, time, server_name);
    let domain = utf16le(cred.domain);
    let user = utf16le(cred.user);
    let workstation = utf16le(cred.workstation);
    let flags = (challenge.flags & !NEG_KEY_EXCH) | NEG_UNICODE;
    let mut m = Vec::new();
    m.extend_from_slice(SIGNATURE);
    m.extend_from_slice(&3u32.to_le_bytes());
    let mut at = 72;
    let mut place = |m: &mut Vec<u8>, len: usize| {
        Field::write(m, len, at);
        at += len;
    };
    place(&mut m, lm.len());
    place(&mut m, nt.len());
    place(&mut m, domain.len());
    place(&mut m, user.len());
    place(&mut m, workstation.len());
    place(&mut m, 0);
    m.extend_from_slice(&flags.to_le_bytes());
    m.extend_from_slice(&VERSION);
    for part in [&lm, &nt, &domain, &user, &workstation] {
        m.extend_from_slice(part);
    }
    Ok(m)
}

/// The fields of an AUTHENTICATE_MESSAGE.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authenticate {
    /// The flags of the client.
    pub flags: u32,
    /// The domain.
    pub domain: String,
    /// The user name.
    pub user: String,
    /// The workstation.
    pub workstation: String,
    /// The NT response, the proof and the client blob.
    pub nt_response: Vec<u8>,
    /// Whether the message carries an encrypted random session key.
    pub key_exchange: bool,
}

/// Reads an AUTHENTICATE_MESSAGE.
pub fn parse_authenticate(msg: &[u8]) -> Result<Authenticate, WireError> {
    check_header(msg, 3)?;
    let flags = u32_at(msg, 60, "the authenticate flags")?;
    let nt = Field::read(msg, 20)?;
    let domain = Field::read(msg, 28)?;
    let user = Field::read(msg, 36)?;
    let workstation = Field::read(msg, 44)?;
    let key = Field::read(msg, 52)?;
    Ok(Authenticate {
        flags,
        domain: decode_text(domain.bytes(msg, "the domain")?, flags)?,
        user: decode_text(user.bytes(msg, "the user name")?, flags)?,
        workstation: decode_text(workstation.bytes(msg, "the workstation")?, flags)?,
        nt_response: nt.bytes(msg, "the NT response")?.to_vec(),
        key_exchange: key.len > 0,
    })
}

/// The session base key that both sides derive from an AUTHENTICATE_MESSAGE
/// and the password. It is the exported session key because the module sends no
/// key exchange, and SMB 2.x signs with it.
pub fn session_key(authenticate: &Authenticate, password: &str) -> Result<[u8; 16], WireError> {
    if authenticate.key_exchange {
        return Err(invalid(
            "the authenticate message carries an encrypted session key, and the module supports no key exchange",
        ));
    }
    let proof = authenticate
        .nt_response
        .get(..16)
        .ok_or_else(|| invalid("the NT response is shorter than its 16 octet proof"))?;
    let key = ntowf_v2(password, &authenticate.user, &authenticate.domain);
    Ok(hmac_md5(&key, &[proof]))
}

/// The message type of an NTLMSSP message.
pub fn message_type(msg: &[u8]) -> Result<u32, WireError> {
    if slice(msg, 0, 8, "the NTLMSSP signature")? != SIGNATURE {
        return Err(invalid("the message does not start with the NTLMSSP signature"));
    }
    u32_at(msg, 8, "the NTLM message type")
}

/// Reads a field of any NTLMSSP message as text.
pub fn field(msg: &[u8], name: &str) -> Result<String, WireError> {
    let ty = message_type(msg)?;
    let missing = |ty: u32| {
        invalid(format!(
            "the NTLM field '{name}' does not exist in a message of the type {ty}"
        ))
    };
    match (name, ty) {
        ("type", _) => Ok(ty.to_string()),
        ("flags", 1) => Ok(u32_at(msg, 12, "the flags")?.to_string()),
        ("domain", 1) | ("workstation", 1) => {
            let at = if name == "domain" { 16 } else { 24 };
            let f = Field::read(msg, at)?;
            let bytes = f.bytes(msg, "a negotiate name")?;
            Ok(bytes.iter().map(|b| char::from(*b)).collect())
        }
        ("flags", 2) => Ok(parse_challenge(msg)?.flags.to_string()),
        ("target", 2) => Ok(parse_challenge(msg)?.target),
        ("challenge", 2) => Ok(crate::wire::to_hex(&parse_challenge(msg)?.server_challenge)),
        ("computer" | "dns_domain" | "dns_computer" | "domain", 2) => {
            let id = match name {
                "domain" => 2,
                "computer" => 1,
                "dns_domain" => 4,
                _ => 3,
            };
            let info = parse_challenge(msg)?.target_info;
            match av_value(&info, id) {
                Some(v) => from_utf16le(v, "a target name"),
                None => Ok(String::new()),
            }
        }
        ("time", 2) => {
            let info = parse_challenge(msg)?.target_info;
            match av_value(&info, 7) {
                Some(v) if v.len() == 8 => Ok(u64_at(v, 0, "the target time")?.to_string()),
                _ => Ok(String::new()),
            }
        }
        ("flags", 3) => Ok(parse_authenticate(msg)?.flags.to_string()),
        ("user", 3) => Ok(parse_authenticate(msg)?.user),
        ("domain", 3) => Ok(parse_authenticate(msg)?.domain),
        ("workstation", 3) => Ok(parse_authenticate(msg)?.workstation),
        ("nt_response", 3) => Ok(crate::wire::to_hex(&parse_authenticate(msg)?.nt_response)),
        ("flags" | "domain" | "workstation" | "target" | "challenge" | "computer"
        | "dns_domain" | "dns_computer" | "time" | "user" | "nt_response", other) => {
            Err(missing(other))
        }
        _ => Err(invalid(format!(
            "the NTLM field '{name}' is not one of type, flags, domain, workstation, target, challenge, computer, dns_domain, dns_computer, time, user, or nt_response"
        ))),
    }
}
