// SPDX-License-Identifier: GPL-3.0-only
//! The SMB2 wire format of MS-SMB2: the transport header, the 64 octet message
//! header, and the bodies of the messages that the module writes and reads.
//!
//! Every writer appends one complete message to a buffer, behind the 4 octet
//! direct TCP transport header (port 445, MS-SMB2 section 2.1). Offsets inside
//! a body count from the first octet of the SMB2 header, as the specification
//! says, and each writer sets them from the fixed layout of its body. Every
//! reader works on bytes that the peer sent, so it checks each length and
//! offset before it indexes and returns an error and never panics.

use std::fmt;

/// The four octets that start every SMB2 message.
pub const PROTOCOL_ID: [u8; 4] = [0xFE, b'S', b'M', b'B'];

/// The length of the SMB2 header.
pub const HEADER_LEN: usize = 64;

/// The largest message that the 24 bit length of the transport header allows.
pub const MAX_MESSAGE: usize = 0xFF_FFFF;

/// The `ProcessId` that Windows clients put in the reserved field of the header.
const PROCESS_ID: u32 = 0xFEFF;

/// Why a message could not be built or read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireError {
    /// The input ends before the structure does.
    Short {
        /// What was being read.
        what: &'static str,
        /// Octets that the structure needs.
        need: usize,
        /// Octets that the input holds.
        have: usize,
    },
    /// A field has a value that the structure forbids.
    Invalid(String),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::Short { what, need, have } => {
                write!(f, "{what} needs {need} octets, the input has {have}")
            }
            WireError::Invalid(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for WireError {}

pub(crate) fn invalid(message: impl Into<String>) -> WireError {
    WireError::Invalid(message.into())
}

/// SMB2 command codes (MS-SMB2 section 2.2.1.2).
pub mod cmd {
    /// NEGOTIATE.
    pub const NEGOTIATE: u16 = 0x0000;
    /// SESSION_SETUP.
    pub const SESSION_SETUP: u16 = 0x0001;
    /// LOGOFF.
    pub const LOGOFF: u16 = 0x0002;
    /// TREE_CONNECT.
    pub const TREE_CONNECT: u16 = 0x0003;
    /// TREE_DISCONNECT.
    pub const TREE_DISCONNECT: u16 = 0x0004;
    /// CREATE.
    pub const CREATE: u16 = 0x0005;
    /// CLOSE.
    pub const CLOSE: u16 = 0x0006;
    /// READ.
    pub const READ: u16 = 0x0008;
    /// WRITE.
    pub const WRITE: u16 = 0x0009;
    /// QUERY_DIRECTORY.
    pub const QUERY_DIRECTORY: u16 = 0x000E;
}

/// Header flags (MS-SMB2 section 2.2.1.2).
pub mod flag {
    /// The message is a response.
    pub const SERVER_TO_REDIR: u32 = 0x1;
    /// The header is an asynchronous header.
    pub const ASYNC_COMMAND: u32 = 0x2;
    /// The message follows a related message of a compound.
    pub const RELATED: u32 = 0x4;
    /// The message carries a signature.
    pub const SIGNED: u32 = 0x8;
}

/// The command names of the catalog, in the order of the codes.
const COMMANDS: &[(u16, &str)] = &[
    (cmd::NEGOTIATE, "negotiate"),
    (cmd::SESSION_SETUP, "session_setup"),
    (cmd::LOGOFF, "logoff"),
    (cmd::TREE_CONNECT, "tree_connect"),
    (cmd::TREE_DISCONNECT, "tree_disconnect"),
    (cmd::CREATE, "create"),
    (cmd::CLOSE, "close"),
    (0x0007, "flush"),
    (cmd::READ, "read"),
    (cmd::WRITE, "write"),
    (0x000A, "lock"),
    (0x000B, "ioctl"),
    (0x000C, "cancel"),
    (0x000D, "echo"),
    (cmd::QUERY_DIRECTORY, "query_directory"),
    (0x000F, "change_notify"),
    (0x0010, "query_info"),
    (0x0011, "set_info"),
    (0x0012, "oplock_break"),
];

/// The name of a command code, or `unknown` for a code that SMB2 does not define.
pub fn command_name(code: u16) -> &'static str {
    COMMANDS
        .iter()
        .find(|(c, _)| *c == code)
        .map_or("unknown", |(_, n)| n)
}

/// The code of a command name.
pub fn command_code(name: &str) -> Option<u16> {
    COMMANDS.iter().find(|(_, n)| *n == name).map(|(c, _)| *c)
}

/// The status codes that the module names (MS-ERREF section 2.3.1).
pub const STATUS_NAMES: &[(u32, &str)] = &[
    (0x0000_0000, "success"),
    (0x0000_0103, "pending"),
    (0x8000_0005, "buffer_overflow"),
    (0x8000_0006, "no_more_files"),
    (0xC000_0001, "unsuccessful"),
    (0xC000_000D, "invalid_parameter"),
    (0xC000_0011, "end_of_file"),
    (0xC000_0016, "more_processing_required"),
    (0xC000_0022, "access_denied"),
    (0xC000_0034, "object_name_not_found"),
    (0xC000_0035, "object_name_collision"),
    (0xC000_003A, "object_path_not_found"),
    (0xC000_0043, "sharing_violation"),
    (0xC000_006D, "logon_failure"),
    (0xC000_00BB, "not_supported"),
    (0xC000_00C9, "network_name_deleted"),
    (0xC000_00CC, "bad_network_name"),
    (0xC000_0128, "file_closed"),
    (0xC000_0203, "user_session_deleted"),
];

/// The code of a status name or of a number such as `0xC0000022` or `3221225506`.
pub fn status_code(text: &str) -> Option<u32> {
    if let Some((c, _)) = STATUS_NAMES.iter().find(|(_, n)| *n == text) {
        return Some(*c);
    }
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => text.parse::<u32>().ok(),
    }
}

/// The name of a status code, or `0x` and eight hexadecimal digits.
pub fn status_name(code: u32) -> String {
    STATUS_NAMES
        .iter()
        .find(|(c, _)| *c == code)
        .map_or_else(|| format!("0x{code:08x}"), |(_, n)| (*n).to_owned())
}

/// A dialect revision that a client offers or a server selects.
pub const DIALECTS: &[(u16, &str)] = &[
    (0x0202, "2.0.2"),
    (0x0210, "2.1"),
    (0x0300, "3.0"),
    (0x0302, "3.0.2"),
    (0x0311, "3.1.1"),
];

/// The revision of a dialect name.
pub fn dialect_code(name: &str) -> Option<u16> {
    DIALECTS.iter().find(|(_, n)| *n == name).map(|(c, _)| *c)
}

/// The name of a dialect revision, or `0x` and four hexadecimal digits. The
/// wildcard revision 0x02FF that a multi protocol answer uses has the name `2.???`.
pub fn dialect_name(code: u16) -> String {
    if code == 0x02FF {
        return "2.???".to_owned();
    }
    DIALECTS
        .iter()
        .find(|(c, _)| *c == code)
        .map_or_else(|| format!("0x{code:04x}"), |(_, n)| (*n).to_owned())
}

// ---------------------------------------------------------------------------
// Reading helpers
// ---------------------------------------------------------------------------

/// Returns `len` octets at `off`, or the error that names `what`.
pub(crate) fn slice<'a>(
    bytes: &'a [u8],
    off: usize,
    len: usize,
    what: &'static str,
) -> Result<&'a [u8], WireError> {
    off.checked_add(len)
        .and_then(|end| bytes.get(off..end))
        .ok_or(WireError::Short {
            what,
            need: off.saturating_add(len),
            have: bytes.len(),
        })
}

pub(crate) fn u8_at(bytes: &[u8], off: usize, what: &'static str) -> Result<u8, WireError> {
    Ok(slice(bytes, off, 1, what)?[0])
}

pub(crate) fn u16_at(bytes: &[u8], off: usize, what: &'static str) -> Result<u16, WireError> {
    let b = slice(bytes, off, 2, what)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

pub(crate) fn u32_at(bytes: &[u8], off: usize, what: &'static str) -> Result<u32, WireError> {
    let b = slice(bytes, off, 4, what)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub(crate) fn u64_at(bytes: &[u8], off: usize, what: &'static str) -> Result<u64, WireError> {
    let b = slice(bytes, off, 8, what)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(b);
    Ok(u64::from_le_bytes(a))
}

/// Encodes text as UTF-16LE.
pub fn utf16le(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// Decodes UTF-16LE text. An odd octet count or an unpaired surrogate is an error.
pub fn from_utf16le(bytes: &[u8], what: &'static str) -> Result<String, WireError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(invalid(format!("{what} has an odd number of octets")));
    }
    let units: Vec<u16> = bytes
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&units).map_err(|_| invalid(format!("{what} is not valid UTF-16")))
}

/// Renders octets as lowercase hexadecimal digits.
pub fn to_hex(bytes: &[u8]) -> String {
    use fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Parses hexadecimal digits, with optional spaces and colons between octets.
pub fn from_hex(text: &str) -> Result<Vec<u8>, WireError> {
    let digits: Vec<u8> = text
        .bytes()
        .filter(|b| !matches!(b, b' ' | b':' | b'\n' | b'\t'))
        .collect();
    if !digits.len().is_multiple_of(2) {
        return Err(invalid("hexadecimal text has an odd number of digits"));
    }
    digits
        .chunks(2)
        .map(|p| {
            let hi = (p[0] as char).to_digit(16);
            let lo = (p[1] as char).to_digit(16);
            match (hi, lo) {
                (Some(h), Some(l)) => Ok((h * 16 + l) as u8),
                _ => Err(invalid(
                    "hexadecimal text has a character that is not a digit",
                )),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The header
// ---------------------------------------------------------------------------

/// The fields of an SMB2 header that a caller chooses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// `CreditCharge`.
    pub credit_charge: u16,
    /// `Status`, 0 in a request.
    pub status: u32,
    /// `Command`.
    pub command: u16,
    /// `CreditRequest` of a request or `CreditResponse` of a response.
    pub credits: u16,
    /// `Flags`.
    pub flags: u32,
    /// `NextCommand`, the offset of the next message of a compound, or 0.
    pub next_command: u32,
    /// `MessageId`.
    pub message_id: u64,
    /// `TreeId`, 0 in an asynchronous header.
    pub tree_id: u32,
    /// `AsyncId` of an asynchronous header.
    pub async_id: u64,
    /// `SessionId`.
    pub session_id: u64,
    /// `Signature`.
    pub signature: [u8; 16],
}

impl Header {
    /// A synchronous header with the given command, message, session, and tree.
    pub fn new(command: u16, message_id: u64, session_id: u64, tree_id: u32) -> Header {
        Header {
            credit_charge: 0,
            status: 0,
            command,
            credits: 1,
            flags: 0,
            next_command: 0,
            message_id,
            tree_id,
            async_id: 0,
            session_id,
            signature: [0; 16],
        }
    }

    /// Whether the message is a response.
    pub fn is_response(&self) -> bool {
        self.flags & flag::SERVER_TO_REDIR != 0
    }

    /// Whether the message carries a signature.
    pub fn is_signed(&self) -> bool {
        self.flags & flag::SIGNED != 0
    }

    /// Appends the 64 octets of the header.
    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&PROTOCOL_ID);
        out.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&self.credit_charge.to_le_bytes());
        out.extend_from_slice(&self.status.to_le_bytes());
        out.extend_from_slice(&self.command.to_le_bytes());
        out.extend_from_slice(&self.credits.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(&self.next_command.to_le_bytes());
        out.extend_from_slice(&self.message_id.to_le_bytes());
        if self.flags & flag::ASYNC_COMMAND != 0 {
            out.extend_from_slice(&self.async_id.to_le_bytes());
        } else {
            out.extend_from_slice(&PROCESS_ID.to_le_bytes());
            out.extend_from_slice(&self.tree_id.to_le_bytes());
        }
        out.extend_from_slice(&self.session_id.to_le_bytes());
        out.extend_from_slice(&self.signature);
    }
}

/// One parsed message: the header and the octets of the message.
#[derive(Clone, Copy, Debug)]
pub struct Message<'a> {
    /// The header.
    pub header: Header,
    /// The octets of the message from the first octet of the header. A
    /// compound message ends at `NextCommand`.
    pub raw: &'a [u8],
}

/// Removes the 4 octet transport header when `bytes` starts with one.
pub fn strip_transport(bytes: &[u8]) -> &[u8] {
    if bytes.len() >= 4 + HEADER_LEN && bytes[0] == 0 && bytes[4..8] == PROTOCOL_ID {
        &bytes[4..]
    } else {
        bytes
    }
}

/// Reads the length from the 4 octet transport header.
pub fn transport_length(header: &[u8]) -> Result<usize, WireError> {
    let h = slice(header, 0, 4, "a transport header")?;
    if h[0] != 0 {
        return Err(invalid(format!(
            "a transport header starts with the octet {:#04x}, and direct TCP uses 0x00",
            h[0]
        )));
    }
    Ok(usize::from(h[1]) << 16 | usize::from(h[2]) << 8 | usize::from(h[3]))
}

impl<'a> Message<'a> {
    /// Parses the message at the start of `bytes`, with or without the
    /// transport header.
    pub fn parse(bytes: &'a [u8]) -> Result<Message<'a>, WireError> {
        let b = strip_transport(bytes);
        let id = slice(b, 0, 4, "the SMB2 protocol id")?;
        if id != PROTOCOL_ID {
            if id == [0xFF, b'S', b'M', b'B'] {
                return Err(invalid("the message is SMB1, and the module reads SMB2"));
            }
            return Err(invalid(
                "the message does not start with the SMB2 protocol id",
            ));
        }
        let size = u16_at(b, 4, "the structure size")?;
        if usize::from(size) != HEADER_LEN {
            return Err(invalid(format!(
                "the SMB2 header has the structure size {size}, not {HEADER_LEN}"
            )));
        }
        slice(b, 0, HEADER_LEN, "an SMB2 header")?;
        let flags = u32_at(b, 16, "flags")?;
        let next_command = u32_at(b, 20, "next command")?;
        let (tree_id, async_id) = if flags & flag::ASYNC_COMMAND != 0 {
            (0, u64_at(b, 32, "the async id")?)
        } else {
            (u32_at(b, 36, "the tree id")?, 0)
        };
        let mut signature = [0u8; 16];
        signature.copy_from_slice(slice(b, 48, 16, "the signature")?);
        let header = Header {
            credit_charge: u16_at(b, 6, "the credit charge")?,
            status: u32_at(b, 8, "the status")?,
            command: u16_at(b, 12, "the command")?,
            credits: u16_at(b, 14, "the credits")?,
            flags,
            next_command,
            message_id: u64_at(b, 24, "the message id")?,
            tree_id,
            async_id,
            session_id: u64_at(b, 40, "the session id")?,
            signature,
        };
        let end = match next_command as usize {
            0 => b.len(),
            n if (HEADER_LEN..=b.len()).contains(&n) => n,
            n => {
                return Err(invalid(format!(
                    "the next command offset {n} lies outside the message of {} octets",
                    b.len()
                )))
            }
        };
        Ok(Message {
            header,
            raw: &b[..end],
        })
    }

    /// The octets after the header.
    pub fn body(&self) -> &'a [u8] {
        self.raw.get(HEADER_LEN..).unwrap_or(&[])
    }

    /// Returns the buffer at `offset` from the header with `len` octets.
    pub fn buffer(
        &self,
        offset: usize,
        len: usize,
        what: &'static str,
    ) -> Result<&'a [u8], WireError> {
        if len == 0 {
            return Ok(&[]);
        }
        if offset < HEADER_LEN {
            return Err(invalid(format!(
                "the offset of {what} is {offset} and lies inside the header"
            )));
        }
        slice(self.raw, offset, len, what)
    }

    /// Checks the structure size of the body against `expected`.
    pub fn check_structure(&self, expected: u16, what: &'static str) -> Result<(), WireError> {
        let got = u16_at(self.raw, HEADER_LEN, what)?;
        if got != expected {
            return Err(invalid(format!(
                "the {what} has the structure size {got}, not {expected}"
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// What a writer needs to build the header of one message.
#[derive(Clone, Copy, Debug)]
pub struct Envelope {
    /// The command.
    pub command: u16,
    /// Whether the message is a response.
    pub response: bool,
    /// The status of a response.
    pub status: u32,
    /// `CreditCharge`.
    pub credit_charge: u16,
    /// Credits requested or granted.
    pub credits: u16,
    /// `MessageId`.
    pub message_id: u64,
    /// `SessionId`.
    pub session_id: u64,
    /// `TreeId`.
    pub tree_id: u32,
}

/// Appends a complete message: the transport header, the SMB2 header, and `body`.
pub fn message(out: &mut Vec<u8>, e: &Envelope, body: &[u8]) -> Result<(), WireError> {
    let len = HEADER_LEN + body.len();
    if len > MAX_MESSAGE {
        return Err(invalid(format!(
            "the message has {len} octets, and the transport header allows {MAX_MESSAGE}"
        )));
    }
    out.push(0);
    out.extend_from_slice(&(len as u32).to_be_bytes()[1..]);
    let mut h = Header::new(e.command, e.message_id, e.session_id, e.tree_id);
    h.credit_charge = e.credit_charge;
    h.status = e.status;
    h.credits = e.credits;
    if e.response {
        h.flags |= flag::SERVER_TO_REDIR;
    }
    h.write(out);
    out.extend_from_slice(body);
    Ok(())
}

fn put16(b: &mut Vec<u8>, v: u16) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn put32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn put64(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_le_bytes());
}

/// Pads `b` with zeros until the message offset `HEADER_LEN + b.len()` is a multiple of 8.
fn align8(b: &mut Vec<u8>) {
    while !(HEADER_LEN + b.len()).is_multiple_of(8) {
        b.push(0);
    }
}

/// The offset from the header of the octet that follows a body of `fixed` octets.
fn after(fixed: usize) -> u16 {
    (HEADER_LEN + fixed) as u16
}

/// A 16 octet file identifier.
pub type FileId = [u8; 16];

/// Parses a file identifier of 32 hexadecimal digits.
pub fn parse_file_id(text: &str) -> Result<FileId, WireError> {
    let bytes = from_hex(text)?;
    FileId::try_from(bytes.as_slice()).map_err(|_| {
        invalid(format!(
            "a file id has 32 hexadecimal digits, and the text has {}",
            bytes.len() * 2
        ))
    })
}

/// The SecurityMode of a negotiate or session setup message.
pub fn security_mode(name: &str) -> Result<u8, WireError> {
    match name {
        "disabled" => Ok(0),
        "enabled" => Ok(1),
        "required" => Ok(3),
        other => Err(invalid(format!(
            "signing must be disabled, enabled, or required, not '{other}'"
        ))),
    }
}

/// The NEGOTIATE request body of a client that offers `dialects`.
pub struct NegotiateRequest<'a> {
    /// The dialect revisions in the order that the client lists them.
    pub dialects: &'a [u16],
    /// `SecurityMode`.
    pub security_mode: u8,
    /// `Capabilities`.
    pub capabilities: u32,
    /// `ClientGuid`.
    pub client_guid: [u8; 16],
    /// The salt of the preauthentication integrity context of SMB 3.1.1.
    pub salt: [u8; 32],
}

/// Context types of SMB 3.1.1 (MS-SMB2 section 2.2.3.1).
const CTX_PREAUTH: u16 = 1;
const CTX_ENCRYPTION: u16 = 2;

fn preauth_context(salt: &[u8]) -> Vec<u8> {
    let mut d = Vec::new();
    put16(&mut d, 1); // one hash algorithm
    put16(&mut d, salt.len() as u16);
    put16(&mut d, 1); // SHA-512
    d.extend_from_slice(salt);
    d
}

/// The body of a NEGOTIATE request.
pub fn negotiate_request_body(r: &NegotiateRequest<'_>) -> Result<Vec<u8>, WireError> {
    if r.dialects.is_empty() || r.dialects.len() > 64 {
        return Err(invalid(
            "a negotiate request lists between 1 and 64 dialects",
        ));
    }
    let with_contexts = r.dialects.contains(&0x0311);
    let mut b = Vec::new();
    put16(&mut b, 36);
    put16(&mut b, r.dialects.len() as u16);
    put16(&mut b, u16::from(r.security_mode));
    put16(&mut b, 0);
    put32(&mut b, r.capabilities);
    b.extend_from_slice(&r.client_guid);
    // ClientStartTime, or the context offset, count, and a reserved field.
    let start_time_at = b.len();
    put64(&mut b, 0);
    for d in r.dialects {
        put16(&mut b, *d);
    }
    if with_contexts {
        align8(&mut b);
        let offset = HEADER_LEN + b.len();
        let mut contexts = Vec::new();
        // `b` ends at an offset divisible by 8, so alignment inside `contexts` is right.
        context_after(&mut contexts, CTX_PREAUTH, &preauth_context(&r.salt));
        let mut enc = Vec::new();
        put16(&mut enc, 2);
        put16(&mut enc, 0x0002); // AES-128-GCM
        put16(&mut enc, 0x0001); // AES-128-CCM
        context_after(&mut contexts, CTX_ENCRYPTION, &enc);
        b[start_time_at..start_time_at + 4].copy_from_slice(&(offset as u32).to_le_bytes());
        b[start_time_at + 4..start_time_at + 6].copy_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&contexts);
    }
    Ok(b)
}

/// Appends a context to a buffer that starts at an offset divisible by 8.
fn context_after(b: &mut Vec<u8>, ty: u16, data: &[u8]) {
    while !b.len().is_multiple_of(8) {
        b.push(0);
    }
    put16(b, ty);
    put16(b, data.len() as u16);
    put32(b, 0);
    b.extend_from_slice(data);
}

/// The fields of a NEGOTIATE response.
pub struct NegotiateResponse<'a> {
    /// The selected dialect.
    pub dialect: u16,
    /// `SecurityMode`.
    pub security_mode: u8,
    /// `ServerGuid`.
    pub server_guid: [u8; 16],
    /// `Capabilities`.
    pub capabilities: u32,
    /// `MaxTransactSize`, `MaxReadSize`, and `MaxWriteSize`.
    pub max_size: u32,
    /// `SystemTime`.
    pub system_time: u64,
    /// `ServerStartTime`.
    pub start_time: u64,
    /// The security buffer, a GSS token.
    pub security: &'a [u8],
    /// The salt of the preauthentication integrity context of SMB 3.1.1.
    pub salt: [u8; 32],
}

/// The body of a NEGOTIATE response.
pub fn negotiate_response_body(r: &NegotiateResponse<'_>) -> Result<Vec<u8>, WireError> {
    if r.security.len() > usize::from(u16::MAX) {
        return Err(invalid(
            "the security buffer of a negotiate response is too long",
        ));
    }
    let with_contexts = r.dialect == 0x0311;
    let mut b = Vec::new();
    put16(&mut b, 65);
    put16(&mut b, u16::from(r.security_mode));
    put16(&mut b, r.dialect);
    put16(&mut b, if with_contexts { 1 } else { 0 });
    b.extend_from_slice(&r.server_guid);
    put32(&mut b, r.capabilities);
    put32(&mut b, r.max_size);
    put32(&mut b, r.max_size);
    put32(&mut b, r.max_size);
    put64(&mut b, r.system_time);
    put64(&mut b, r.start_time);
    put16(&mut b, after(64));
    put16(&mut b, r.security.len() as u16);
    let context_offset_at = b.len();
    put32(&mut b, 0);
    b.extend_from_slice(r.security);
    if with_contexts {
        align8(&mut b);
        let offset = HEADER_LEN + b.len();
        b[context_offset_at..context_offset_at + 4].copy_from_slice(&(offset as u32).to_le_bytes());
        context_after(&mut b, CTX_PREAUTH, &preauth_context(&r.salt));
    } else if r.security.is_empty() {
        // The structure size counts one octet of the buffer.
        b.push(0);
    }
    Ok(b)
}

/// The body of a SESSION_SETUP request.
pub fn session_setup_request_body(
    security: &[u8],
    security_mode: u8,
    binding: bool,
    previous_session_id: u64,
) -> Result<Vec<u8>, WireError> {
    if security.len() > usize::from(u16::MAX) {
        return Err(invalid(
            "the security buffer of a session setup is too long",
        ));
    }
    let mut b = Vec::new();
    put16(&mut b, 25);
    b.push(u8::from(binding));
    b.push(security_mode);
    put32(&mut b, 0); // Capabilities
    put32(&mut b, 0); // Channel
    put16(&mut b, after(24));
    put16(&mut b, security.len() as u16);
    put64(&mut b, previous_session_id);
    b.extend_from_slice(security);
    if security.is_empty() {
        b.push(0);
    }
    Ok(b)
}

/// The body of a SESSION_SETUP response.
pub fn session_setup_response_body(
    security: &[u8],
    session_flags: u16,
) -> Result<Vec<u8>, WireError> {
    if security.len() > usize::from(u16::MAX) {
        return Err(invalid(
            "the security buffer of a session setup is too long",
        ));
    }
    let mut b = Vec::new();
    put16(&mut b, 9);
    put16(&mut b, session_flags);
    put16(&mut b, after(8));
    put16(&mut b, security.len() as u16);
    b.extend_from_slice(security);
    if security.is_empty() {
        b.push(0);
    }
    Ok(b)
}

/// The body of a TREE_CONNECT request.
pub fn tree_connect_request_body(path: &str) -> Result<Vec<u8>, WireError> {
    let name = utf16le(path);
    if name.is_empty() || name.len() > usize::from(u16::MAX) {
        return Err(invalid(
            "a tree connect path has between 1 and 32767 characters",
        ));
    }
    let mut b = Vec::new();
    put16(&mut b, 9);
    put16(&mut b, 0); // Flags
    put16(&mut b, after(8));
    put16(&mut b, name.len() as u16);
    b.extend_from_slice(&name);
    Ok(b)
}

/// The share type of a TREE_CONNECT response.
pub fn share_type(name: &str) -> Result<u8, WireError> {
    match name {
        "disk" => Ok(1),
        "pipe" => Ok(2),
        "print" => Ok(3),
        other => Err(invalid(format!(
            "share_type must be disk, pipe, or print, not '{other}'"
        ))),
    }
}

/// The name of a share type code.
pub fn share_type_name(code: u8) -> &'static str {
    match code {
        1 => "disk",
        2 => "pipe",
        3 => "print",
        _ => "unknown",
    }
}

/// The body of a TREE_CONNECT response.
pub fn tree_connect_response_body(
    share_type: u8,
    share_flags: u32,
    capabilities: u32,
    maximal_access: u32,
) -> Vec<u8> {
    let mut b = Vec::new();
    put16(&mut b, 16);
    b.push(share_type);
    b.push(0);
    put32(&mut b, share_flags);
    put32(&mut b, capabilities);
    put32(&mut b, maximal_access);
    b
}

/// The body of the requests and responses that carry only a structure size of
/// 4 and a reserved field, which are TREE_DISCONNECT and LOGOFF.
pub fn empty_body() -> Vec<u8> {
    let mut b = Vec::new();
    put16(&mut b, 4);
    put16(&mut b, 0);
    b
}

/// The fields of a CREATE request.
pub struct CreateRequest<'a> {
    /// The path below the share, with backslashes, or empty for the root.
    pub name: &'a str,
    /// `RequestedOplockLevel`.
    pub oplock: u8,
    /// `DesiredAccess`.
    pub desired_access: u32,
    /// `FileAttributes`.
    pub file_attributes: u32,
    /// `ShareAccess`.
    pub share_access: u32,
    /// `CreateDisposition`.
    pub disposition: u32,
    /// `CreateOptions`.
    pub options: u32,
}

/// The body of a CREATE request.
pub fn create_request_body(r: &CreateRequest<'_>) -> Result<Vec<u8>, WireError> {
    let name = utf16le(r.name);
    if name.len() > usize::from(u16::MAX) {
        return Err(invalid("a create name has more than 32767 characters"));
    }
    let mut b = Vec::new();
    put16(&mut b, 57);
    b.push(0); // SecurityFlags
    b.push(r.oplock);
    put32(&mut b, 2); // Impersonation
    put64(&mut b, 0); // SmbCreateFlags
    put64(&mut b, 0);
    put32(&mut b, r.desired_access);
    put32(&mut b, r.file_attributes);
    put32(&mut b, r.share_access);
    put32(&mut b, r.disposition);
    put32(&mut b, r.options);
    put16(&mut b, after(56));
    put16(&mut b, name.len() as u16);
    put32(&mut b, 0); // CreateContextsOffset
    put32(&mut b, 0); // CreateContextsLength
    b.extend_from_slice(&name);
    if name.is_empty() {
        b.push(0);
    }
    Ok(b)
}

/// The fields of a CREATE or CLOSE response that describe a file.
#[derive(Clone, Copy, Debug)]
pub struct FileInfo {
    /// `CreationTime`, `LastAccessTime`, `LastWriteTime`, and `ChangeTime`.
    pub time: u64,
    /// `AllocationSize`.
    pub allocation_size: u64,
    /// `EndofFile`.
    pub end_of_file: u64,
    /// `FileAttributes`.
    pub attributes: u32,
}

/// The body of a CREATE response.
pub fn create_response_body(oplock: u8, action: u32, info: &FileInfo, id: &FileId) -> Vec<u8> {
    let mut b = Vec::new();
    put16(&mut b, 89);
    b.push(oplock);
    b.push(0); // Flags
    put32(&mut b, action);
    for _ in 0..4 {
        put64(&mut b, info.time);
    }
    put64(&mut b, info.allocation_size);
    put64(&mut b, info.end_of_file);
    put32(&mut b, info.attributes);
    put32(&mut b, 0);
    b.extend_from_slice(id);
    put32(&mut b, 0); // CreateContextsOffset
    put32(&mut b, 0); // CreateContextsLength
    b
}

/// The body of a CLOSE request.
pub fn close_request_body(flags: u16, id: &FileId) -> Vec<u8> {
    let mut b = Vec::new();
    put16(&mut b, 24);
    put16(&mut b, flags);
    put32(&mut b, 0);
    b.extend_from_slice(id);
    b
}

/// The body of a CLOSE response. The attributes are zero unless `flags` asks for them.
pub fn close_response_body(flags: u16, info: &FileInfo) -> Vec<u8> {
    let mut b = Vec::new();
    put16(&mut b, 60);
    put16(&mut b, flags);
    put32(&mut b, 0);
    let post = flags & 1 != 0;
    let t = if post { info.time } else { 0 };
    for _ in 0..4 {
        put64(&mut b, t);
    }
    put64(&mut b, if post { info.allocation_size } else { 0 });
    put64(&mut b, if post { info.end_of_file } else { 0 });
    put32(&mut b, if post { info.attributes } else { 0 });
    b
}

/// The body of a READ request.
pub fn read_request_body(
    id: &FileId,
    length: u32,
    offset: u64,
    minimum: u32,
    remaining: u32,
) -> Vec<u8> {
    let mut b = Vec::new();
    put16(&mut b, 49);
    b.push(0x50); // Padding, the offset of the data in the response
    b.push(0); // Flags
    put32(&mut b, length);
    put64(&mut b, offset);
    b.extend_from_slice(id);
    put32(&mut b, minimum);
    put32(&mut b, 0); // Channel
    put32(&mut b, remaining);
    put16(&mut b, 0);
    put16(&mut b, 0);
    b.push(0); // the one octet of the buffer
    b
}

/// The body of a READ response.
pub fn read_response_body(data: &[u8], remaining: u32) -> Result<Vec<u8>, WireError> {
    let len = u32::try_from(data.len()).map_err(|_| invalid("the read data is too long"))?;
    let mut b = Vec::with_capacity(16 + data.len());
    put16(&mut b, 17);
    b.push((HEADER_LEN + 16) as u8);
    b.push(0);
    put32(&mut b, len);
    put32(&mut b, remaining);
    put32(&mut b, 0);
    b.extend_from_slice(data);
    Ok(b)
}

/// The body of a WRITE request.
pub fn write_request_body(
    id: &FileId,
    data: &[u8],
    offset: u64,
    remaining: u32,
    flags: u32,
) -> Result<Vec<u8>, WireError> {
    let len = u32::try_from(data.len()).map_err(|_| invalid("the write data is too long"))?;
    let mut b = Vec::with_capacity(48 + data.len());
    put16(&mut b, 49);
    put16(&mut b, after(48));
    put32(&mut b, len);
    put64(&mut b, offset);
    b.extend_from_slice(id);
    put32(&mut b, 0); // Channel
    put32(&mut b, remaining);
    put16(&mut b, 0);
    put16(&mut b, 0);
    put32(&mut b, flags);
    b.extend_from_slice(data);
    if data.is_empty() {
        b.push(0);
    }
    Ok(b)
}

/// The body of a WRITE response.
pub fn write_response_body(count: u32, remaining: u32) -> Vec<u8> {
    let mut b = Vec::new();
    put16(&mut b, 17);
    put16(&mut b, 0);
    put32(&mut b, count);
    put32(&mut b, remaining);
    put16(&mut b, 0);
    put16(&mut b, 0);
    b
}

/// The FileInformationClass of the directory listing that the module writes.
pub const FILE_ID_BOTH_DIRECTORY_INFORMATION: u8 = 0x25;

/// The body of a QUERY_DIRECTORY request.
pub fn query_directory_request_body(
    id: &FileId,
    pattern: &str,
    flags: u8,
    output_length: u32,
) -> Result<Vec<u8>, WireError> {
    let name = utf16le(pattern);
    if name.len() > usize::from(u16::MAX) {
        return Err(invalid("a search pattern has more than 32767 characters"));
    }
    let mut b = Vec::new();
    put16(&mut b, 33);
    b.push(FILE_ID_BOTH_DIRECTORY_INFORMATION);
    b.push(flags);
    put32(&mut b, 0); // FileIndex
    b.extend_from_slice(id);
    put16(&mut b, after(32));
    put16(&mut b, name.len() as u16);
    put32(&mut b, output_length);
    b.extend_from_slice(&name);
    if name.is_empty() {
        b.push(0);
    }
    Ok(b)
}

/// One entry of a directory listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    /// The file name.
    pub name: String,
    /// The size in octets.
    pub size: u64,
    /// Whether the entry is a directory.
    pub directory: bool,
}

/// The fixed part of a FileIdBothDirectoryInformation entry.
const DIR_ENTRY_FIXED: usize = 104;

/// Encodes the entries as FileIdBothDirectoryInformation records.
pub fn directory_information(entries: &[DirEntry], time: u64) -> Result<Vec<u8>, WireError> {
    let mut out: Vec<u8> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let name = utf16le(&e.name);
        let start = out.len();
        let mut b = Vec::with_capacity(DIR_ENTRY_FIXED + name.len());
        put32(&mut b, 0); // NextEntryOffset, patched below
        put32(&mut b, 0); // FileIndex
        for _ in 0..4 {
            put64(&mut b, time);
        }
        put64(&mut b, if e.directory { 0 } else { e.size });
        put64(
            &mut b,
            (if e.directory { 0 } else { e.size }).div_ceil(4096) * 4096,
        );
        put32(&mut b, if e.directory { 0x10 } else { 0x20 });
        put32(&mut b, name.len() as u32);
        put32(&mut b, 0); // EaSize
        b.push(0); // ShortNameLength
        b.push(0);
        b.extend_from_slice(&[0u8; 24]);
        put16(&mut b, 0);
        put64(&mut b, 0x1000 + i as u64); // FileId
        b.extend_from_slice(&name);
        while !b.len().is_multiple_of(8) {
            b.push(0);
        }
        out.extend_from_slice(&b);
        if i + 1 < entries.len() {
            let next = (out.len() - start) as u32;
            out[start..start + 4].copy_from_slice(&next.to_le_bytes());
        }
    }
    Ok(out)
}

/// Decodes FileIdBothDirectoryInformation records.
pub fn parse_directory_information(mut buf: &[u8]) -> Result<Vec<DirEntry>, WireError> {
    let mut entries = Vec::new();
    loop {
        let next = u32_at(buf, 0, "a directory entry")? as usize;
        let end_of_file = u64_at(buf, 40, "a directory entry")?;
        let attributes = u32_at(buf, 56, "a directory entry")?;
        let name_len = u32_at(buf, 60, "a directory entry")? as usize;
        let name = slice(buf, DIR_ENTRY_FIXED, name_len, "a directory entry name")?;
        entries.push(DirEntry {
            name: from_utf16le(name, "a directory entry name")?,
            size: end_of_file,
            directory: attributes & 0x10 != 0,
        });
        if next == 0 {
            return Ok(entries);
        }
        if next < DIR_ENTRY_FIXED {
            return Err(invalid(
                "a directory entry has a next offset below its size",
            ));
        }
        buf = buf
            .get(next..)
            .ok_or_else(|| invalid("a directory entry has a next offset beyond the buffer"))?;
        if entries.len() > 100_000 {
            return Err(invalid("a directory listing has more than 100000 entries"));
        }
    }
}

/// The body of a QUERY_DIRECTORY response.
pub fn query_directory_response_body(information: &[u8]) -> Result<Vec<u8>, WireError> {
    let len = u32::try_from(information.len()).map_err(|_| invalid("the listing is too long"))?;
    let mut b = Vec::new();
    put16(&mut b, 9);
    put16(&mut b, after(8));
    put32(&mut b, len);
    b.extend_from_slice(information);
    if information.is_empty() {
        b.push(0);
    }
    Ok(b)
}

/// The body of an error response (MS-SMB2 section 2.2.2).
pub fn error_response_body() -> Vec<u8> {
    let mut b = Vec::new();
    put16(&mut b, 9);
    b.push(0); // ErrorContextCount
    b.push(0);
    put32(&mut b, 0); // ByteCount
    b.push(0); // ErrorData
    b
}

/// Signs the message at the start of `bytes`, which holds a message without
/// the transport header, with HMAC-SHA256 as SMB 2.x does. The function sets
/// the signed flag and writes the first 16 octets of the digest into the
/// signature field.
pub fn sign_hmac_sha256(message: &mut [u8], key: &[u8]) -> Result<(), WireError> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    if message.len() < HEADER_LEN {
        return Err(WireError::Short {
            what: "an SMB2 header",
            need: HEADER_LEN,
            have: message.len(),
        });
    }
    let flags =
        u32::from_le_bytes([message[16], message[17], message[18], message[19]]) | flag::SIGNED;
    message[16..20].copy_from_slice(&flags.to_le_bytes());
    message[48..64].fill(0);
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key)
        .map_err(|_| invalid("the signing key has an unusable length"))?;
    mac.update(message);
    let digest = mac.finalize().into_bytes();
    message[48..64].copy_from_slice(&digest[..16]);
    Ok(())
}
