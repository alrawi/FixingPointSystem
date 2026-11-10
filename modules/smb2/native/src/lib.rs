// SPDX-License-Identifier: GPL-3.0-only
//! Native template functions of the `smb2` module.
//!
//! Decision D10 lets first party modules implement their template functions in
//! Rust. This crate implements the `smb2` functions behind the [`TemplateFn`]
//! trait of `fps-flow`, so the interpreter renders
//! `{{ smb2.negotiate(dialects='2.1,3.0.2,3.1.1', message_id=0) }}` without a
//! WebAssembly runtime. The functions write the messages of MS-SMB2 that a file
//! transfer needs, which are NEGOTIATE, SESSION_SETUP, TREE_CONNECT, CREATE,
//! READ, WRITE, CLOSE, QUERY_DIRECTORY, TREE_DISCONNECT, and LOGOFF, with the
//! responses to them, and they read the messages of the peer. The NTLM functions
//! write the three NTLMSSP messages of a synthetic exchange with a full NTLMv2
//! response inside SPNEGO tokens, so both sides of a scenario authenticate with
//! a user and a password that the scenario names.
//!
//! Every message that a function writes starts with the 4 octet transport
//! header of direct TCP (port 445), so a `send` action writes it as it is.
//! Every function is binary safe: arguments and results are octets, and a WRITE
//! request or a READ response carries any bytes that a nested call such as
//! `content.binary` returns.
//!
//! # State
//!
//! The module has no state. The ids that tie messages together are arguments:
//! a flow numbers its messages with `message_id`, takes the session id from the
//! response to its first SESSION_SETUP, and takes the tree id and the file id
//! from the responses that assign them. A server draws a new id from the
//! `session_id`, `tree_id`, or `file_id` argument that the scenario computes.
//!
//! # Reading messages in a flow
//!
//! A flow reads a message in two `expect` actions, the 4 octet transport
//! header, then the message of the length that the header announces. After the
//! second `expect`, `last` holds the message, and `header_field` and
//! `body_field` read its fields as decimal digits or text, which a `set` action
//! binds. A `set` action keeps text, so a value that must keep its octets, the
//! security buffer of a session setup, travels inside one expression or as
//! `smb2.hex(...)` and `smb2.unhex(vars.name)`.
//!
//! ```yaml
//! - expect: { bytes: 4 }                                          # transport header
//! - expect: { bytes: "{{ smb2.message_length(last) }}" }         # the message
//! - set:
//!     command: "{{ smb2.header_field(last, 'command') }}"
//!     session: "{{ smb2.header_field(last, 'session_id') }}"
//! - send: "{{ smb2.session_setup_response(smb2.ntlm_challenge(), 1, 4660, status='more_processing_required') }}"
//! ```
//!
//! # Limits
//!
//! * Session ids use 64 bits and a template integer is signed, so a session id
//!   travels as the two's complement integer. `header_field` writes it so and
//!   the functions that take a session id read it back, so ids round trip.
//! * `sign` signs with HMAC-SHA256, which is the signature of SMB 2.0.2 and
//!   2.1. SMB 3.x signs with AES-CMAC and a derived key, and encrypts messages
//!   with AES-GCM or AES-CCM, and the module does none of this.
//! * The NTLM exchange sends no message integrity code and no key exchange.
//!   An SMB 3.1.1 negotiate lists its preauthentication and encryption
//!   contexts, and the exchange that follows is not hashed.
//! * Compound requests, asynchronous responses, create contexts, leases, and
//!   durable handles are not written, and a reader that meets them returns the
//!   fields of the first message.
//!
//! [`FUNCTIONS`] is the catalog that `modules/smb2/manifest.yaml` lists under
//! `functions`. The catalog test compares the two, so the manifest stays the
//! single catalog that the validator and the controller read.
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod args;
pub mod catalog;
pub mod ntlm;
pub mod read;
pub mod wire;

use fps_flow::{FnCall, FnError, FnErrorKind, TemplateFn};
use rand::RngCore;

pub use catalog::{DefaultValue, FunctionSpec, ParamSpec, DEFAULT_FILETIME, FUNCTIONS};

use args::Args;
use wire::{cmd, Envelope, FileInfo, Message, WireError};

/// Name of the module that this crate implements.
pub const MODULE_NAME: &str = "smb2";

/// The template functions of the `smb2` module. The value has no state, so one
/// value serves every session.
#[derive(Clone, Copy, Debug, Default)]
pub struct Smb2;

fn invalid(message: impl Into<String>) -> FnError {
    FnError::new(FnErrorKind::InvalidArgument, message)
}

fn protocol(message: impl Into<String>) -> FnError {
    FnError::new(FnErrorKind::Protocol, message)
}

/// Maps a wire error that comes from the arguments of a call.
fn bad_arg(function: &str) -> impl Fn(WireError) -> FnError + '_ {
    move |e| invalid(format!("'{function}': {e}"))
}

/// Maps a wire error that comes from bytes of the peer.
fn bad_msg(function: &str) -> impl Fn(WireError) -> FnError + '_ {
    move |e| protocol(format!("'{function}': {e}"))
}

fn push(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
}

/// Reads an integer argument in `lo..=hi`.
fn ranged(a: &Args<'_>, function: &str, name: &str, lo: i64, hi: i64) -> Result<i64, FnError> {
    let v = a.int(name)?;
    if !(lo..=hi).contains(&v) {
        return Err(invalid(format!(
            "'{name}' of '{function}' must be between {lo} and {hi}, not {v}"
        )));
    }
    Ok(v)
}

fn u32_arg(a: &Args<'_>, function: &str, name: &str) -> Result<u32, FnError> {
    Ok(ranged(a, function, name, 0, i64::from(u32::MAX))? as u32)
}

fn u64_arg(a: &Args<'_>, function: &str, name: &str) -> Result<u64, FnError> {
    Ok(ranged(a, function, name, 0, i64::MAX)? as u64)
}

/// Reads a session id, which uses all 64 bits as a two's complement integer.
fn session_id(a: &Args<'_>) -> Result<u64, FnError> {
    Ok(a.int("session_id")? as u64)
}

/// Reads a time, and -1 is the fixed time of the module.
fn time_arg(a: &Args<'_>, function: &str, name: &str) -> Result<u64, FnError> {
    match a.int(name)? {
        -1 => Ok(DEFAULT_FILETIME),
        v if v >= 0 => Ok(v as u64),
        v => Err(invalid(format!(
            "'{name}' of '{function}' must be -1 or a time of 0 or more, not {v}"
        ))),
    }
}

/// The credit charge of a message: the argument, or `auto` for -1.
fn credit_charge(a: &Args<'_>, function: &str, auto: u64) -> Result<u16, FnError> {
    match a.int("credit_charge")? {
        -1 => Ok(auto.min(u64::from(u16::MAX)) as u16),
        v if (0..=i64::from(u16::MAX)).contains(&v) => Ok(v as u16),
        v => Err(invalid(format!(
            "'credit_charge' of '{function}' must be -1 or between 0 and 65535, not {v}"
        ))),
    }
}

/// One credit per 64 KiB of data, and at least one.
fn credits_for(len: usize) -> u64 {
    (len as u64).div_ceil(65_536).max(1)
}

fn random<const N: usize>(rng: &mut dyn RngCore) -> [u8; N] {
    let mut b = [0u8; N];
    rng.fill_bytes(&mut b);
    b
}

/// Reads a 16 octet identifier argument, and draws one when the text is empty.
fn guid(
    a: &Args<'_>,
    function: &str,
    name: &str,
    rng: &mut dyn RngCore,
) -> Result<[u8; 16], FnError> {
    let given = a.bytes(name)?;
    if given.is_empty() {
        return Ok(random(rng));
    }
    <[u8; 16]>::try_from(given).map_err(|_| {
        invalid(format!(
            "'{name}' of '{function}' must be 16 octets, not {}",
            given.len()
        ))
    })
}

fn file_id(a: &Args<'_>, function: &str) -> Result<wire::FileId, FnError> {
    wire::parse_file_id(a.text("file_id")?).map_err(bad_arg(function))
}

/// Parses a comma separated dialect list.
fn dialect_list(text: &str) -> Result<Vec<u16>, FnError> {
    text.split(',')
        .map(|d| {
            let d = d.trim();
            wire::dialect_code(d).ok_or_else(|| {
                invalid(format!(
                    "'dialects' of 'negotiate' lists '{d}', which is not 2.0.2, 2.1, 3.0, 3.0.2, or 3.1.1"
                ))
            })
        })
        .collect()
}

fn status_arg(a: &Args<'_>, function: &str, name: &str) -> Result<u32, FnError> {
    let text = a.text(name)?;
    wire::status_code(text).ok_or_else(|| {
        invalid(format!(
            "'{name}' of '{function}' is not a status name or number: '{text}'"
        ))
    })
}

/// The request envelope with the ids of the call.
fn request(
    a: &Args<'_>,
    function: &str,
    command: u16,
    auto_charge: u64,
    tree: bool,
) -> Result<Envelope, FnError> {
    Ok(Envelope {
        command,
        response: false,
        status: 0,
        credit_charge: credit_charge(a, function, auto_charge)?,
        credits: ranged(a, function, "credits", 0, i64::from(u16::MAX))? as u16,
        message_id: u64_arg(a, function, "message_id")?,
        session_id: session_id(a)?,
        tree_id: if tree {
            u32_arg(a, function, "tree_id")?
        } else {
            0
        },
    })
}

/// The response envelope with the ids of the call.
fn response(
    a: &Args<'_>,
    function: &str,
    command: u16,
    auto_charge: u64,
    tree: bool,
) -> Result<Envelope, FnError> {
    let mut e = request(a, function, command, auto_charge, tree)?;
    e.response = true;
    Ok(e)
}

fn finish(out: &mut Vec<u8>, e: &Envelope, body: &[u8], function: &str) -> Result<(), FnError> {
    wire::message(out, e, body).map_err(bad_arg(function))
}

/// The file description of a CREATE or CLOSE response.
fn file_info(a: &Args<'_>, function: &str) -> Result<FileInfo, FnError> {
    let size = u64_arg(a, function, "size")?;
    let allocation = match a.int("allocation_size")? {
        -1 => size.div_ceil(4096).saturating_mul(4096),
        v if v >= 0 => v as u64,
        v => {
            return Err(invalid(format!(
                "'allocation_size' of '{function}' must be -1 or 0 or more, not {v}"
            )))
        }
    };
    Ok(FileInfo {
        time: time_arg(a, function, "time")?,
        allocation_size: allocation,
        end_of_file: size,
        attributes: u32_arg(a, function, "attributes")?,
    })
}

/// Parses the lines `size kind name` of a directory listing.
fn dir_entries(text: &str) -> Result<Vec<wire::DirEntry>, FnError> {
    let mut entries = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let bad = || {
            invalid(format!(
                "line {} of 'entries' must be 'size kind name' with kind file or directory",
                n + 1
            ))
        };
        let (size, rest) = line.split_once(' ').ok_or_else(bad)?;
        let (kind, name) = rest.split_once(' ').ok_or_else(bad)?;
        let directory = match kind {
            "file" => false,
            "directory" => true,
            _ => return Err(bad()),
        };
        if name.is_empty() || name.contains(['\\', '/', '\0']) {
            return Err(invalid(format!(
                "line {} of 'entries' has a name that is empty or holds a slash or a NUL",
                n + 1
            )));
        }
        entries.push(wire::DirEntry {
            name: name.to_owned(),
            size: size.parse().map_err(|_| bad())?,
            directory,
        });
    }
    Ok(entries)
}

fn wrap_init(spnego: bool, msg: Vec<u8>) -> Vec<u8> {
    if spnego {
        ntlm::spnego_init(Some(&msg))
    } else {
        msg
    }
}

fn wrap_resp(spnego: bool, state: Option<ntlm::NegState>, mech: bool, msg: Vec<u8>) -> Vec<u8> {
    if spnego {
        ntlm::spnego_resp(state, mech, Some(&msg))
    } else {
        msg
    }
}

fn flags_arg(a: &Args<'_>, function: &str, default: u32) -> Result<u32, FnError> {
    match a.int("flags")? {
        -1 => Ok(default),
        v if (0..=i64::from(u32::MAX)).contains(&v) => Ok(v as u32),
        v => Err(invalid(format!(
            "'flags' of '{function}' must be -1 or an unsigned 32 bit number, not {v}"
        ))),
    }
}

impl Smb2 {
    fn requests(
        &self,
        function: &str,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<bool, FnError> {
        match function {
            "negotiate" => {
                let a = args::bind(&catalog::NEGOTIATE, &call.args)?;
                let dialects = dialect_list(a.text("dialects")?)?;
                let modern = dialects.iter().any(|d| *d >= 0x0300);
                let capabilities = match a.int("capabilities")? {
                    -1 => if modern { 0x07 } else { 0 },
                    v if (0..=i64::from(u32::MAX)).contains(&v) => v as u32,
                    v => {
                        return Err(invalid(format!(
                            "'capabilities' of 'negotiate' must be -1 or an unsigned 32 bit number, not {v}"
                        )))
                    }
                };
                let client_guid = guid(&a, "negotiate", "client_guid", rng)?;
                let salt = random(rng);
                let body = wire::negotiate_request_body(&wire::NegotiateRequest {
                    dialects: &dialects,
                    security_mode: wire::security_mode(a.text("signing")?)
                        .map_err(bad_arg("negotiate"))?,
                    capabilities,
                    client_guid,
                    salt,
                })
                .map_err(bad_arg("negotiate"))?;
                let e = Envelope {
                    command: cmd::NEGOTIATE,
                    response: false,
                    status: 0,
                    credit_charge: 0,
                    credits: ranged(&a, "negotiate", "credits", 0, i64::from(u16::MAX))? as u16,
                    message_id: u64_arg(&a, "negotiate", "message_id")?,
                    session_id: 0,
                    tree_id: 0,
                };
                finish(out, &e, &body, "negotiate")?;
            }
            "session_setup" => {
                let a = args::bind(&catalog::SESSION_SETUP, &call.args)?;
                let body = wire::session_setup_request_body(
                    a.bytes("security")?,
                    wire::security_mode(a.text("signing")?).map_err(bad_arg("session_setup"))?,
                    a.boolean("binding")?,
                    a.int("previous_session_id")? as u64,
                )
                .map_err(bad_arg("session_setup"))?;
                let e = request(&a, "session_setup", cmd::SESSION_SETUP, 1, false)?;
                finish(out, &e, &body, "session_setup")?;
            }
            "tree_connect" => {
                let a = args::bind(&catalog::TREE_CONNECT, &call.args)?;
                let body = wire::tree_connect_request_body(a.text("path")?)
                    .map_err(bad_arg("tree_connect"))?;
                let mut e = request(&a, "tree_connect", cmd::TREE_CONNECT, 1, false)?;
                e.tree_id = 0;
                finish(out, &e, &body, "tree_connect")?;
            }
            "create" => {
                let a = args::bind(&catalog::CREATE, &call.args)?;
                let body = wire::create_request_body(&wire::CreateRequest {
                    name: a.text("name")?,
                    oplock: ranged(&a, "create", "oplock", 0, 255)? as u8,
                    desired_access: u32_arg(&a, "create", "desired_access")?,
                    file_attributes: u32_arg(&a, "create", "file_attributes")?,
                    share_access: u32_arg(&a, "create", "share_access")?,
                    disposition: u32_arg(&a, "create", "disposition")?,
                    options: u32_arg(&a, "create", "options")?,
                })
                .map_err(bad_arg("create"))?;
                finish(
                    out,
                    &request(&a, "create", cmd::CREATE, 1, true)?,
                    &body,
                    "create",
                )?;
            }
            "read" => {
                let a = args::bind(&catalog::READ, &call.args)?;
                let length = u32_arg(&a, "read", "length")?;
                let body = wire::read_request_body(
                    &file_id(&a, "read")?,
                    length,
                    u64_arg(&a, "read", "offset")?,
                    u32_arg(&a, "read", "minimum_count")?,
                    u32_arg(&a, "read", "remaining")?,
                );
                let e = request(&a, "read", cmd::READ, credits_for(length as usize), true)?;
                finish(out, &e, &body, "read")?;
            }
            "write" => {
                let a = args::bind(&catalog::WRITE, &call.args)?;
                let data = a.bytes("data")?;
                let body = wire::write_request_body(
                    &file_id(&a, "write")?,
                    data,
                    u64_arg(&a, "write", "offset")?,
                    u32_arg(&a, "write", "remaining")?,
                    u32_arg(&a, "write", "flags")?,
                )
                .map_err(bad_arg("write"))?;
                let e = request(&a, "write", cmd::WRITE, credits_for(data.len()), true)?;
                finish(out, &e, &body, "write")?;
            }
            "close" => {
                let a = args::bind(&catalog::CLOSE, &call.args)?;
                let body = wire::close_request_body(
                    ranged(&a, "close", "flags", 0, 0xFFFF)? as u16,
                    &file_id(&a, "close")?,
                );
                finish(
                    out,
                    &request(&a, "close", cmd::CLOSE, 1, true)?,
                    &body,
                    "close",
                )?;
            }
            "tree_disconnect" => {
                let a = args::bind(&catalog::TREE_DISCONNECT, &call.args)?;
                let e = request(&a, "tree_disconnect", cmd::TREE_DISCONNECT, 1, true)?;
                finish(out, &e, &wire::empty_body(), "tree_disconnect")?;
            }
            "logoff" => {
                let a = args::bind(&catalog::LOGOFF, &call.args)?;
                let e = request(&a, "logoff", cmd::LOGOFF, 1, false)?;
                finish(out, &e, &wire::empty_body(), "logoff")?;
            }
            "query_directory" => {
                let a = args::bind(&catalog::QUERY_DIRECTORY, &call.args)?;
                let flags = u8::from(a.boolean("restart")?) | (2 * u8::from(a.boolean("single")?));
                let body = wire::query_directory_request_body(
                    &file_id(&a, "query_directory")?,
                    a.text("pattern")?,
                    flags,
                    u32_arg(&a, "query_directory", "output_length")?,
                )
                .map_err(bad_arg("query_directory"))?;
                let e = request(&a, "query_directory", cmd::QUERY_DIRECTORY, 1, true)?;
                finish(out, &e, &body, "query_directory")?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn responses(
        &self,
        function: &str,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<bool, FnError> {
        match function {
            "negotiate_response" => {
                let a = args::bind(&catalog::NEGOTIATE_RESPONSE, &call.args)?;
                let name = a.text("dialect")?;
                let dialect = wire::dialect_code(name).ok_or_else(|| {
                    invalid(format!(
                        "'dialect' of 'negotiate_response' must be 2.0.2, 2.1, 3.0, 3.0.2, or 3.1.1, not '{name}'"
                    ))
                })?;
                let capabilities = match a.int("capabilities")? {
                    -1 => if dialect >= 0x0210 { 0x07 } else { 0 },
                    v if (0..=i64::from(u32::MAX)).contains(&v) => v as u32,
                    v => {
                        return Err(invalid(format!(
                            "'capabilities' of 'negotiate_response' must be -1 or an unsigned 32 bit number, not {v}"
                        )))
                    }
                };
                let server_guid = guid(&a, "negotiate_response", "server_guid", rng)?;
                let salt = random(rng);
                let given = a.bytes("security")?;
                let default_token;
                let security = if given.is_empty() {
                    default_token = ntlm::spnego_init(None);
                    &default_token[..]
                } else {
                    given
                };
                let body = wire::negotiate_response_body(&wire::NegotiateResponse {
                    dialect,
                    security_mode: wire::security_mode(a.text("signing")?)
                        .map_err(bad_arg("negotiate_response"))?,
                    server_guid,
                    capabilities,
                    max_size: u32_arg(&a, "negotiate_response", "max_size")?,
                    system_time: time_arg(&a, "negotiate_response", "system_time")?,
                    start_time: time_arg(&a, "negotiate_response", "start_time")?,
                    security,
                    salt,
                })
                .map_err(bad_arg("negotiate_response"))?;
                let e = Envelope {
                    command: cmd::NEGOTIATE,
                    response: true,
                    status: 0,
                    credit_charge: 0,
                    credits: ranged(&a, "negotiate_response", "credits", 0, i64::from(u16::MAX))?
                        as u16,
                    message_id: u64_arg(&a, "negotiate_response", "message_id")?,
                    session_id: 0,
                    tree_id: 0,
                };
                finish(out, &e, &body, "negotiate_response")?;
            }
            "session_setup_response" => {
                let a = args::bind(&catalog::SESSION_SETUP_RESPONSE, &call.args)?;
                let body = wire::session_setup_response_body(
                    a.bytes("security")?,
                    ranged(&a, "session_setup_response", "session_flags", 0, 0xFFFF)? as u16,
                )
                .map_err(bad_arg("session_setup_response"))?;
                let mut e = response(&a, "session_setup_response", cmd::SESSION_SETUP, 1, false)?;
                e.status = status_arg(&a, "session_setup_response", "status")?;
                finish(out, &e, &body, "session_setup_response")?;
            }
            "tree_connect_response" => {
                let a = args::bind(&catalog::TREE_CONNECT_RESPONSE, &call.args)?;
                let body = wire::tree_connect_response_body(
                    wire::share_type(a.text("share_type")?)
                        .map_err(bad_arg("tree_connect_response"))?,
                    u32_arg(&a, "tree_connect_response", "share_flags")?,
                    u32_arg(&a, "tree_connect_response", "capabilities")?,
                    u32_arg(&a, "tree_connect_response", "maximal_access")?,
                );
                let e = response(&a, "tree_connect_response", cmd::TREE_CONNECT, 1, true)?;
                finish(out, &e, &body, "tree_connect_response")?;
            }
            "create_response" => {
                let a = args::bind(&catalog::CREATE_RESPONSE, &call.args)?;
                let id = if a.text("file_id")?.is_empty() {
                    random(rng)
                } else {
                    file_id(&a, "create_response")?
                };
                let info = file_info(&a, "create_response")?;
                let body = wire::create_response_body(
                    ranged(&a, "create_response", "oplock", 0, 255)? as u8,
                    u32_arg(&a, "create_response", "action")?,
                    &info,
                    &id,
                );
                let e = response(&a, "create_response", cmd::CREATE, 1, true)?;
                finish(out, &e, &body, "create_response")?;
            }
            "read_response" => {
                let a = args::bind(&catalog::READ_RESPONSE, &call.args)?;
                let data = a.bytes("data")?;
                let body =
                    wire::read_response_body(data, u32_arg(&a, "read_response", "remaining")?)
                        .map_err(bad_arg("read_response"))?;
                let e = response(
                    &a,
                    "read_response",
                    cmd::READ,
                    credits_for(data.len()),
                    true,
                )?;
                finish(out, &e, &body, "read_response")?;
            }
            "write_response" => {
                let a = args::bind(&catalog::WRITE_RESPONSE, &call.args)?;
                let count = u32_arg(&a, "write_response", "count")?;
                let body =
                    wire::write_response_body(count, u32_arg(&a, "write_response", "remaining")?);
                let e = response(
                    &a,
                    "write_response",
                    cmd::WRITE,
                    credits_for(count as usize),
                    true,
                )?;
                finish(out, &e, &body, "write_response")?;
            }
            "close_response" => {
                let a = args::bind(&catalog::CLOSE_RESPONSE, &call.args)?;
                let info = file_info(&a, "close_response")?;
                let body = wire::close_response_body(
                    ranged(&a, "close_response", "flags", 0, 0xFFFF)? as u16,
                    &info,
                );
                let e = response(&a, "close_response", cmd::CLOSE, 1, true)?;
                finish(out, &e, &body, "close_response")?;
            }
            "tree_disconnect_response" => {
                let a = args::bind(&catalog::TREE_DISCONNECT_RESPONSE, &call.args)?;
                let e = response(
                    &a,
                    "tree_disconnect_response",
                    cmd::TREE_DISCONNECT,
                    1,
                    true,
                )?;
                finish(out, &e, &wire::empty_body(), "tree_disconnect_response")?;
            }
            "logoff_response" => {
                let a = args::bind(&catalog::LOGOFF_RESPONSE, &call.args)?;
                let e = response(&a, "logoff_response", cmd::LOGOFF, 1, false)?;
                finish(out, &e, &wire::empty_body(), "logoff_response")?;
            }
            "query_directory_response" => {
                let a = args::bind(&catalog::QUERY_DIRECTORY_RESPONSE, &call.args)?;
                let entries = dir_entries(a.text("entries")?)?;
                let info = wire::directory_information(
                    &entries,
                    time_arg(&a, "query_directory_response", "time")?,
                )
                .map_err(bad_arg("query_directory_response"))?;
                let body = wire::query_directory_response_body(&info)
                    .map_err(bad_arg("query_directory_response"))?;
                let e = response(
                    &a,
                    "query_directory_response",
                    cmd::QUERY_DIRECTORY,
                    1,
                    true,
                )?;
                finish(out, &e, &body, "query_directory_response")?;
            }
            "error_response" => {
                let a = args::bind(&catalog::ERROR_RESPONSE, &call.args)?;
                let command =
                    read::command_of(a.text("command")?).map_err(bad_arg("error_response"))?;
                let status = status_arg(&a, "error_response", "status")?;
                if status & 0x8000_0000 == 0 {
                    return Err(invalid(
                        "'status' of 'error_response' must be a warning or an error status",
                    ));
                }
                let charge = u64::from(command != cmd::NEGOTIATE);
                let mut e = response(&a, "error_response", command, charge, true)?;
                e.status = status;
                finish(out, &e, &wire::error_response_body(), "error_response")?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn ntlm(
        &self,
        function: &str,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<bool, FnError> {
        match function {
            "ntlm_negotiate" => {
                let a = args::bind(&catalog::NTLM_NEGOTIATE, &call.args)?;
                let flags = flags_arg(&a, "ntlm_negotiate", ntlm::DEFAULT_NEGOTIATE_FLAGS)?;
                let msg = ntlm::negotiate_message(flags, a.text("domain")?, a.text("workstation")?)
                    .map_err(bad_arg("ntlm_negotiate"))?;
                out.extend(wrap_init(a.boolean("spnego")?, msg));
            }
            "ntlm_challenge" => {
                let a = args::bind(&catalog::NTLM_CHALLENGE, &call.args)?;
                let given = a.bytes("server_challenge")?;
                let challenge = if given.is_empty() {
                    random(rng)
                } else {
                    <[u8; 8]>::try_from(given).map_err(|_| {
                        invalid(format!(
                            "'server_challenge' of 'ntlm_challenge' must be 8 octets, not {}",
                            given.len()
                        ))
                    })?
                };
                let target = a.text("target")?;
                let info = ntlm::target_info(
                    &ntlm::TargetNames {
                        domain: target,
                        computer: a.text("computer")?,
                        dns_domain: a.text("dns_domain")?,
                        dns_computer: a.text("dns_computer")?,
                    },
                    time_arg(&a, "ntlm_challenge", "time")?,
                );
                let flags = flags_arg(&a, "ntlm_challenge", ntlm::DEFAULT_CHALLENGE_FLAGS)?;
                let msg = ntlm::challenge_message(flags, target, challenge, &info)
                    .map_err(bad_arg("ntlm_challenge"))?;
                out.extend(wrap_resp(
                    a.boolean("spnego")?,
                    Some(ntlm::NegState::AcceptIncomplete),
                    true,
                    msg,
                ));
            }
            "ntlm_authenticate" => {
                let a = args::bind(&catalog::NTLM_AUTHENTICATE, &call.args)?;
                let challenge = ntlm::parse_challenge(
                    ntlm::find_message(a.bytes("challenge")?)
                        .map_err(bad_msg("ntlm_authenticate"))?,
                )
                .map_err(bad_msg("ntlm_authenticate"))?;
                let given = a.bytes("client_challenge")?;
                let client_challenge = if given.is_empty() {
                    random(rng)
                } else {
                    <[u8; 8]>::try_from(given).map_err(|_| {
                        invalid(format!(
                            "'client_challenge' of 'ntlm_authenticate' must be 8 octets, not {}",
                            given.len()
                        ))
                    })?
                };
                let msg = ntlm::authenticate_message(
                    &challenge,
                    &ntlm::Credentials {
                        user: a.text("user")?,
                        password: a.text("password")?,
                        domain: a.text("domain")?,
                        workstation: a.text("workstation")?,
                    },
                    client_challenge,
                    time_arg(&a, "ntlm_authenticate", "time")?,
                )
                .map_err(bad_arg("ntlm_authenticate"))?;
                out.extend(wrap_resp(a.boolean("spnego")?, None, false, msg));
            }
            "spnego_accept" => {
                args::bind(&catalog::SPNEGO_ACCEPT, &call.args)?;
                out.extend(ntlm::spnego_resp(
                    Some(ntlm::NegState::AcceptCompleted),
                    false,
                    None,
                ));
            }
            "ntlm_session_key" => {
                let a = args::bind(&catalog::NTLM_SESSION_KEY, &call.args)?;
                let msg = ntlm::find_message(a.bytes("authenticate")?)
                    .map_err(bad_msg("ntlm_session_key"))?;
                let auth = ntlm::parse_authenticate(msg).map_err(bad_msg("ntlm_session_key"))?;
                let key = ntlm::session_key(&auth, a.text("password")?)
                    .map_err(bad_msg("ntlm_session_key"))?;
                push(out, &wire::to_hex(&key));
            }
            "ntlm_field" => {
                let a = args::bind(&catalog::NTLM_FIELD, &call.args)?;
                let msg =
                    ntlm::find_message(a.bytes("security")?).map_err(bad_msg("ntlm_field"))?;
                push(
                    out,
                    &ntlm::field(msg, a.text("field")?).map_err(bad_msg("ntlm_field"))?,
                );
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}

/// Parses the message that a reader gets and maps its errors to protocol errors.
fn message<'a>(bytes: &'a [u8], function: &str) -> Result<Message<'a>, FnError> {
    Message::parse(bytes).map_err(bad_msg(function))
}

impl TemplateFn for Smb2 {
    fn render(
        &mut self,
        call: &FnCall<'_>,
        rng: &mut dyn RngCore,
        out: &mut Vec<u8>,
    ) -> Result<(), FnError> {
        if call.module != MODULE_NAME {
            return Err(FnError::new(
                FnErrorKind::Unsupported,
                format!("module '{}' is not smb2", call.module),
            ));
        }
        let function = call.function;
        if self.requests(function, call, rng, out)?
            || self.responses(function, call, rng, out)?
            || self.ntlm(function, call, rng, out)?
        {
            return Ok(());
        }
        match function {
            "sign" => {
                let a = args::bind(&catalog::SIGN, &call.args)?;
                let given = a.bytes("message")?;
                let key = wire::from_hex(a.text("key")?).map_err(bad_arg("sign"))?;
                let start = given.len() - wire::strip_transport(given).len();
                let mut signed = given.to_vec();
                wire::sign_hmac_sha256(&mut signed[start..], &key).map_err(bad_msg("sign"))?;
                out.extend_from_slice(&signed);
            }
            "message_length" => {
                let a = args::bind(&catalog::MESSAGE_LENGTH, &call.args)?;
                let n = wire::transport_length(a.bytes("header")?)
                    .map_err(bad_msg("message_length"))?;
                push(out, &n.to_string());
            }
            "header_field" => {
                let a = args::bind(&catalog::HEADER_FIELD, &call.args)?;
                let m = message(a.bytes("message")?, "header_field")?;
                push(
                    out,
                    &read::header_field(&m, a.text("field")?).map_err(bad_msg("header_field"))?,
                );
            }
            "body_field" => {
                let a = args::bind(&catalog::BODY_FIELD, &call.args)?;
                let m = message(a.bytes("message")?, "body_field")?;
                push(
                    out,
                    &read::body_field(&m, a.text("field")?).map_err(bad_msg("body_field"))?,
                );
            }
            "security_blob" => {
                let a = args::bind(&catalog::SECURITY_BLOB, &call.args)?;
                let m = message(a.bytes("message")?, "security_blob")?;
                out.extend_from_slice(read::security_blob(&m).map_err(bad_msg("security_blob"))?);
            }
            "data" => {
                let a = args::bind(&catalog::DATA, &call.args)?;
                let m = message(a.bytes("message")?, "data")?;
                out.extend_from_slice(read::data(&m).map_err(bad_msg("data"))?);
            }
            "hex" => {
                let a = args::bind(&catalog::HEX, &call.args)?;
                push(out, &wire::to_hex(a.bytes("bytes")?));
            }
            "unhex" => {
                let a = args::bind(&catalog::UNHEX, &call.args)?;
                out.extend(wire::from_hex(a.text("text")?).map_err(bad_arg("unhex"))?);
            }
            "chunks" => {
                let a = args::bind(&catalog::CHUNKS, &call.args)?;
                let total = u64_arg(&a, "chunks", "total")?;
                let size = ranged(&a, "chunks", "size", 1, i64::MAX)? as u64;
                push(out, &total.div_ceil(size).max(1).to_string());
            }
            "add" => {
                let a = args::bind(&catalog::ADD, &call.args)?;
                let sum = a
                    .int("a")?
                    .checked_add(a.int("b")?)
                    .ok_or_else(|| invalid("'add' overflows a 64 bit integer"))?;
                push(out, &sum.to_string());
            }
            other => {
                return Err(FnError::new(
                    FnErrorKind::Unsupported,
                    format!("smb2 has no function '{other}'"),
                ))
            }
        }
        Ok(())
    }
}
