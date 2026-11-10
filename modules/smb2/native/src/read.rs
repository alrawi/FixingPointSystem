// SPDX-License-Identifier: GPL-3.0-only
//! Readers of SMB2 messages that a peer sent: the header fields, the body
//! fields by command, the security buffer, and the data of a read or a write.
//!
//! The fields come back as text, because a `set` action of a flow keeps text.
//! Octets that must survive, the security buffer and the data, have their own
//! readers that return octets for a nested call.

use crate::wire::{
    cmd, command_code, command_name, dialect_name, from_utf16le, invalid, share_type_name, slice,
    status_name, to_hex, u16_at, u32_at, u64_at, u8_at, Message, WireError, HEADER_LEN,
};

fn flag_text(b: bool) -> String {
    if b { "true" } else { "false" }.to_owned()
}

/// Reads a field of the header, as text.
pub fn header_field(m: &Message<'_>, field: &str) -> Result<String, WireError> {
    let h = &m.header;
    Ok(match field {
        "command" => command_name(h.command).to_owned(),
        "command_code" => h.command.to_string(),
        "status" => status_name(h.status),
        "status_code" => h.status.to_string(),
        "message_id" => h.message_id.to_string(),
        // Session ids use all 64 bits, and a template integer has 63 plus a
        // sign, so the id travels as the two's complement integer. The
        // functions that take a session id read it back the same way.
        "session_id" => (h.session_id as i64).to_string(),
        "tree_id" => h.tree_id.to_string(),
        "credits" => h.credits.to_string(),
        "credit_charge" => h.credit_charge.to_string(),
        "flags" => h.flags.to_string(),
        "response" => flag_text(h.is_response()),
        "signed" => flag_text(h.is_signed()),
        "async" => flag_text(h.flags & crate::wire::flag::ASYNC_COMMAND != 0),
        "next_command" => h.next_command.to_string(),
        other => {
            return Err(invalid(format!(
                "'field' of 'header_field' must be command, command_code, status, status_code, message_id, session_id, tree_id, credits, credit_charge, flags, response, signed, async, or next_command, not '{other}'"
            )))
        }
    })
}

/// Whether the message is an error response, whose body is the 9 octet
/// ErrorResponse structure and not the body of its command.
pub fn is_error_response(m: &Message<'_>) -> bool {
    let s = m.header.status;
    m.header.is_response()
        && s != 0xC000_0016
        && (s & 0xC000_0000 == 0xC000_0000 || s == 0x8000_0006)
}

fn file_id(body: &[u8], at: usize) -> Result<String, WireError> {
    Ok(to_hex(slice(body, at, 16, "a file id")?))
}

fn text_at(
    m: &Message<'_>,
    body: &[u8],
    off_at: usize,
    len_at: usize,
    what: &'static str,
) -> Result<String, WireError> {
    let offset = usize::from(u16_at(body, off_at, what)?);
    let len = usize::from(u16_at(body, len_at, what)?);
    from_utf16le(m.buffer(offset, len, what)?, what)
}

fn no_field(m: &Message<'_>, field: &str) -> WireError {
    let side = if m.header.is_response() {
        "response"
    } else {
        "request"
    };
    invalid(format!(
        "the field '{field}' is not defined for a {} {side}",
        command_name(m.header.command)
    ))
}

/// Reads a field of the body, as text.
pub fn body_field(m: &Message<'_>, field: &str) -> Result<String, WireError> {
    if is_error_response(m) {
        return Err(invalid(format!(
            "the message is an error response with the status {}, and it has no field '{field}'",
            status_name(m.header.status)
        )));
    }
    let body = m.body();
    let resp = m.header.is_response();
    let c = m.header.command;
    let miss = || no_field(m, field);
    let what: &'static str = "a message body";
    let value = match (c, resp) {
        (cmd::NEGOTIATE, false) => {
            m.check_structure(36, "negotiate request")?;
            match field {
                "dialect_count" => u16_at(body, 2, what)?.to_string(),
                "dialects" => {
                    let n = usize::from(u16_at(body, 2, what)?);
                    let raw = slice(body, 36, n * 2, "the dialects")?;
                    raw.chunks_exact(2)
                        .map(|d| dialect_name(u16::from_le_bytes([d[0], d[1]])))
                        .collect::<Vec<_>>()
                        .join(",")
                }
                "security_mode" => u16_at(body, 4, what)?.to_string(),
                "capabilities" => u32_at(body, 8, what)?.to_string(),
                "guid" => file_id(body, 12)?,
                _ => return Err(miss()),
            }
        }
        (cmd::NEGOTIATE, true) => {
            m.check_structure(65, "negotiate response")?;
            match field {
                "dialect" => dialect_name(u16_at(body, 4, what)?),
                "security_mode" => u16_at(body, 2, what)?.to_string(),
                "capabilities" => u32_at(body, 24, what)?.to_string(),
                "guid" => file_id(body, 8)?,
                "max_transact" => u32_at(body, 28, what)?.to_string(),
                "max_read" => u32_at(body, 32, what)?.to_string(),
                "max_write" => u32_at(body, 36, what)?.to_string(),
                _ => return Err(miss()),
            }
        }
        (cmd::SESSION_SETUP, false) => {
            m.check_structure(25, "session setup request")?;
            match field {
                "binding" => flag_text(u8_at(body, 2, what)? & 1 != 0),
                "security_mode" => u8_at(body, 3, what)?.to_string(),
                "previous_session_id" => (u64_at(body, 16, what)? as i64).to_string(),
                _ => return Err(miss()),
            }
        }
        (cmd::SESSION_SETUP, true) => {
            m.check_structure(9, "session setup response")?;
            match field {
                "session_flags" => u16_at(body, 2, what)?.to_string(),
                _ => return Err(miss()),
            }
        }
        (cmd::TREE_CONNECT, false) => {
            m.check_structure(9, "tree connect request")?;
            match field {
                "path" => text_at(m, body, 4, 6, "the tree connect path")?,
                _ => return Err(miss()),
            }
        }
        (cmd::TREE_CONNECT, true) => {
            m.check_structure(16, "tree connect response")?;
            match field {
                "share_type" => share_type_name(u8_at(body, 2, what)?).to_owned(),
                "share_flags" => u32_at(body, 4, what)?.to_string(),
                "capabilities" => u32_at(body, 8, what)?.to_string(),
                "maximal_access" => u32_at(body, 12, what)?.to_string(),
                _ => return Err(miss()),
            }
        }
        (cmd::CREATE, false) => {
            m.check_structure(57, "create request")?;
            match field {
                "name" => text_at(m, body, 44, 46, "the create name")?,
                "oplock" => u8_at(body, 3, what)?.to_string(),
                "desired_access" => u32_at(body, 24, what)?.to_string(),
                "attributes" => u32_at(body, 28, what)?.to_string(),
                "share_access" => u32_at(body, 32, what)?.to_string(),
                "disposition" => u32_at(body, 36, what)?.to_string(),
                "options" => u32_at(body, 40, what)?.to_string(),
                _ => return Err(miss()),
            }
        }
        (cmd::CREATE, true) => {
            m.check_structure(89, "create response")?;
            match field {
                "oplock" => u8_at(body, 2, what)?.to_string(),
                "action" => u32_at(body, 4, what)?.to_string(),
                "allocation_size" => u64_at(body, 40, what)?.to_string(),
                "size" => u64_at(body, 48, what)?.to_string(),
                "attributes" => u32_at(body, 56, what)?.to_string(),
                "file_id" => file_id(body, 64)?,
                _ => return Err(miss()),
            }
        }
        (cmd::READ, false) => {
            m.check_structure(49, "read request")?;
            match field {
                "length" => u32_at(body, 4, what)?.to_string(),
                "offset" => u64_at(body, 8, what)?.to_string(),
                "file_id" => file_id(body, 16)?,
                _ => return Err(miss()),
            }
        }
        (cmd::READ, true) => {
            m.check_structure(17, "read response")?;
            match field {
                "length" => u32_at(body, 4, what)?.to_string(),
                "remaining" => u32_at(body, 8, what)?.to_string(),
                _ => return Err(miss()),
            }
        }
        (cmd::WRITE, false) => {
            m.check_structure(49, "write request")?;
            match field {
                "length" => u32_at(body, 4, what)?.to_string(),
                "offset" => u64_at(body, 8, what)?.to_string(),
                "file_id" => file_id(body, 16)?,
                "remaining" => u32_at(body, 36, what)?.to_string(),
                "flags" => u32_at(body, 44, what)?.to_string(),
                _ => return Err(miss()),
            }
        }
        (cmd::WRITE, true) => {
            m.check_structure(17, "write response")?;
            match field {
                "count" => u32_at(body, 4, what)?.to_string(),
                "remaining" => u32_at(body, 8, what)?.to_string(),
                _ => return Err(miss()),
            }
        }
        (cmd::CLOSE, false) => {
            m.check_structure(24, "close request")?;
            match field {
                "flags" => u16_at(body, 2, what)?.to_string(),
                "file_id" => file_id(body, 8)?,
                _ => return Err(miss()),
            }
        }
        (cmd::CLOSE, true) => {
            m.check_structure(60, "close response")?;
            match field {
                "flags" => u16_at(body, 2, what)?.to_string(),
                "size" => u64_at(body, 48, what)?.to_string(),
                "attributes" => u32_at(body, 56, what)?.to_string(),
                _ => return Err(miss()),
            }
        }
        (cmd::QUERY_DIRECTORY, false) => {
            m.check_structure(33, "query directory request")?;
            match field {
                "flags" => u8_at(body, 3, what)?.to_string(),
                "file_id" => file_id(body, 8)?,
                "pattern" => text_at(m, body, 24, 26, "the search pattern")?,
                "output_length" => u32_at(body, 28, what)?.to_string(),
                _ => return Err(miss()),
            }
        }
        (cmd::QUERY_DIRECTORY, true) => {
            m.check_structure(9, "query directory response")?;
            let offset = usize::from(u16_at(body, 2, what)?);
            let len = u32_at(body, 4, what)? as usize;
            match field {
                "length" => len.to_string(),
                "entry_count" | "entries" => {
                    let entries = crate::wire::parse_directory_information(m.buffer(
                        offset,
                        len,
                        "the directory listing",
                    )?)?;
                    if field == "entry_count" {
                        entries.len().to_string()
                    } else {
                        entries
                            .iter()
                            .map(|e| {
                                format!(
                                    "{} {} {}",
                                    e.size,
                                    if e.directory { "directory" } else { "file" },
                                    e.name
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                }
                _ => return Err(miss()),
            }
        }
        _ => return Err(miss()),
    };
    Ok(value)
}

/// The security buffer of a NEGOTIATE response or a SESSION_SETUP message.
pub fn security_blob<'a>(m: &Message<'a>) -> Result<&'a [u8], WireError> {
    let body = m.body();
    let what = "the security buffer";
    let (offset, len) = match (m.header.command, m.header.is_response()) {
        (cmd::NEGOTIATE, true) => {
            m.check_structure(65, "negotiate response")?;
            (u16_at(body, 56, what)?, u16_at(body, 58, what)?)
        }
        (cmd::SESSION_SETUP, false) => {
            m.check_structure(25, "session setup request")?;
            (u16_at(body, 12, what)?, u16_at(body, 14, what)?)
        }
        (cmd::SESSION_SETUP, true) => {
            m.check_structure(9, "session setup response")?;
            (u16_at(body, 4, what)?, u16_at(body, 6, what)?)
        }
        _ => {
            return Err(invalid(format!(
                "a {} {} has no security buffer",
                command_name(m.header.command),
                if m.header.is_response() {
                    "response"
                } else {
                    "request"
                }
            )))
        }
    };
    m.buffer(usize::from(offset), usize::from(len), what)
}

/// The data of a READ response or a WRITE request.
pub fn data<'a>(m: &Message<'a>) -> Result<&'a [u8], WireError> {
    let body = m.body();
    let what = "the data";
    let (offset, len) = match (m.header.command, m.header.is_response()) {
        (cmd::READ, true) => {
            m.check_structure(17, "read response")?;
            (
                usize::from(u8_at(body, 2, what)?),
                u32_at(body, 4, what)? as usize,
            )
        }
        (cmd::WRITE, false) => {
            m.check_structure(49, "write request")?;
            (
                usize::from(u16_at(body, 2, what)?),
                u32_at(body, 4, what)? as usize,
            )
        }
        _ => {
            return Err(invalid(format!(
                "a {} {} carries no data",
                command_name(m.header.command),
                if m.header.is_response() {
                    "response"
                } else {
                    "request"
                }
            )))
        }
    };
    if offset != 0 && offset < HEADER_LEN {
        return Err(invalid("the data offset lies inside the header"));
    }
    m.buffer(offset, len, what)
}

/// The command code that a name stands for, for the error response.
pub fn command_of(name: &str) -> Result<u16, WireError> {
    command_code(name).ok_or_else(|| invalid(format!("'{name}' is not an SMB2 command name")))
}
