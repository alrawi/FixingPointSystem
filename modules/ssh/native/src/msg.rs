// SPDX-License-Identifier: GPL-3.0-only
//! The message numbers of RFC 4250 and the payloads that the module sends.
//!
//! A payload starts with the message number. [`crate::transport`] wraps it in
//! a packet. Every channel message addresses channel 0, because each side of
//! a session opens one channel and numbers it 0, as OpenSSH does.

use crate::wire::{protocol, put_bool, put_string, put_u32};
use fps_flow::FnError;

pub(crate) const DISCONNECT: u8 = 1;
pub(crate) const SERVICE_REQUEST: u8 = 5;
pub(crate) const SERVICE_ACCEPT: u8 = 6;
pub(crate) const KEXINIT: u8 = 20;
pub(crate) const NEWKEYS: u8 = 21;
pub(crate) const ECDH_INIT: u8 = 30;
pub(crate) const ECDH_REPLY: u8 = 31;
pub(crate) const USERAUTH_REQUEST: u8 = 50;
pub(crate) const USERAUTH_FAILURE: u8 = 51;
pub(crate) const USERAUTH_SUCCESS: u8 = 52;
pub(crate) const CHANNEL_OPEN: u8 = 90;
pub(crate) const CHANNEL_CONFIRM: u8 = 91;
pub(crate) const WINDOW_ADJUST: u8 = 93;
pub(crate) const CHANNEL_DATA: u8 = 94;
pub(crate) const CHANNEL_EOF: u8 = 96;
pub(crate) const CHANNEL_CLOSE: u8 = 97;
pub(crate) const CHANNEL_REQUEST: u8 = 98;
pub(crate) const CHANNEL_SUCCESS: u8 = 99;

/// The names that `ssh.recv` accepts for its `expect` argument.
const NAMES: &[(&str, u8)] = &[
    ("disconnect", DISCONNECT),
    ("service_request", SERVICE_REQUEST),
    ("service_accept", SERVICE_ACCEPT),
    ("kexinit", KEXINIT),
    ("newkeys", NEWKEYS),
    ("ecdh_init", ECDH_INIT),
    ("ecdh_reply", ECDH_REPLY),
    ("userauth_request", USERAUTH_REQUEST),
    ("userauth_failure", USERAUTH_FAILURE),
    ("userauth_success", USERAUTH_SUCCESS),
    ("channel_open", CHANNEL_OPEN),
    ("channel_confirm", CHANNEL_CONFIRM),
    ("window_adjust", WINDOW_ADJUST),
    ("channel_data", CHANNEL_DATA),
    ("channel_eof", CHANNEL_EOF),
    ("channel_close", CHANNEL_CLOSE),
    ("channel_request", CHANNEL_REQUEST),
    ("channel_success", CHANNEL_SUCCESS),
];

/// Maps the `expect` argument of `ssh.recv` to a message number. `any` gives
/// `None`.
pub(crate) fn expected(name: &str) -> Result<Option<u8>, FnError> {
    if name == "any" {
        return Ok(None);
    }
    NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, code)| Some(*code))
        .ok_or_else(|| {
            let known: Vec<&str> = NAMES.iter().map(|(n, _)| *n).collect();
            protocol(format!(
                "'{name}' names no message, use any or one of {}",
                known.join(", ")
            ))
        })
}

/// The name of message `code`, or its number for a message that the module
/// does not know.
pub(crate) fn name_of(code: u8) -> String {
    NAMES
        .iter()
        .find(|(_, c)| *c == code)
        .map_or_else(|| format!("message {code}"), |(n, _)| (*n).to_string())
}

/// A payload that holds only the message number and the channel number.
pub(crate) fn channel_only(code: u8) -> Vec<u8> {
    let mut p = vec![code];
    put_u32(&mut p, 0);
    p
}

pub(crate) fn service(code: u8, name: &str) -> Vec<u8> {
    let mut p = vec![code];
    put_string(&mut p, name.as_bytes());
    p
}

pub(crate) fn userauth_none(user: &str) -> Vec<u8> {
    let mut p = vec![USERAUTH_REQUEST];
    put_string(&mut p, user.as_bytes());
    put_string(&mut p, b"ssh-connection");
    put_string(&mut p, b"none");
    p
}

pub(crate) fn userauth_password(user: &str, password: &str) -> Vec<u8> {
    let mut p = vec![USERAUTH_REQUEST];
    put_string(&mut p, user.as_bytes());
    put_string(&mut p, b"ssh-connection");
    put_string(&mut p, b"password");
    put_bool(&mut p, false);
    put_string(&mut p, password.as_bytes());
    p
}

pub(crate) fn userauth_failure(methods: &str) -> Vec<u8> {
    let mut p = vec![USERAUTH_FAILURE];
    put_string(&mut p, methods.as_bytes());
    put_bool(&mut p, false);
    p
}

pub(crate) fn channel_open(window: u32, max_packet: u32) -> Vec<u8> {
    let mut p = vec![CHANNEL_OPEN];
    put_string(&mut p, b"session");
    put_u32(&mut p, 0);
    put_u32(&mut p, window);
    put_u32(&mut p, max_packet);
    p
}

pub(crate) fn channel_confirm(window: u32, max_packet: u32) -> Vec<u8> {
    let mut p = vec![CHANNEL_CONFIRM];
    put_u32(&mut p, 0);
    put_u32(&mut p, 0);
    put_u32(&mut p, window);
    put_u32(&mut p, max_packet);
    p
}

/// The terminal modes of a typical interactive client: interrupt, erase,
/// echo, and the input and output speed, then the end marker.
const PTY_MODES: &[u8] = &[
    1, 0, 0, 0, 3, // VINTR
    3, 0, 0, 0, 127, // VERASE
    53, 0, 0, 0, 1, // ECHO
    128, 0, 0, 0x96, 0, // TTY_OP_ISPEED, 38400
    129, 0, 0, 0x96, 0, // TTY_OP_OSPEED, 38400
    0, // TTY_OP_END
];

/// A `pty-req`, `shell`, or `exec` request.
pub(crate) fn channel_request(kind: &str, command: &str, reply: bool) -> Result<Vec<u8>, FnError> {
    let mut p = vec![CHANNEL_REQUEST];
    put_u32(&mut p, 0);
    put_string(&mut p, kind.as_bytes());
    put_bool(&mut p, reply);
    match kind {
        "pty-req" => {
            put_string(&mut p, b"xterm-256color");
            for v in [80, 24, 0, 0] {
                put_u32(&mut p, v);
            }
            put_string(&mut p, PTY_MODES);
        }
        "shell" => {}
        "exec" => {
            if command.is_empty() {
                return Err(protocol("an exec request needs a command"));
            }
            put_string(&mut p, command.as_bytes());
        }
        other => {
            return Err(protocol(format!(
                "request kind '{other}' is not supported, use pty-req, shell, or exec"
            )))
        }
    }
    Ok(p)
}

pub(crate) fn exit_status(code: u32) -> Vec<u8> {
    let mut p = vec![CHANNEL_REQUEST];
    put_u32(&mut p, 0);
    put_string(&mut p, b"exit-status");
    put_bool(&mut p, false);
    put_u32(&mut p, code);
    p
}

pub(crate) fn channel_data(data: &[u8]) -> Vec<u8> {
    let mut p = vec![CHANNEL_DATA];
    put_u32(&mut p, 0);
    put_string(&mut p, data);
    p
}

pub(crate) fn window_adjust(count: u32) -> Vec<u8> {
    let mut p = vec![WINDOW_ADJUST];
    put_u32(&mut p, 0);
    put_u32(&mut p, count);
    p
}

/// `SSH_MSG_DISCONNECT` with the reason code 11, `by application`.
pub(crate) fn disconnect(reason: &str) -> Vec<u8> {
    let mut p = vec![DISCONNECT];
    put_u32(&mut p, 11);
    put_string(&mut p, reason.as_bytes());
    put_string(&mut p, b"");
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_map_to_numbers_and_back() {
        for (n, c) in NAMES {
            assert_eq!(expected(n).unwrap(), Some(*c));
            assert_eq!(name_of(*c), *n);
        }
        assert_eq!(expected("any").unwrap(), None);
        assert!(expected("nonsense").is_err());
        assert_eq!(name_of(200), "message 200");
    }

    #[test]
    fn a_keystroke_payload_is_ten_bytes() {
        assert_eq!(channel_data(b"x").len(), 10);
    }

    #[test]
    fn requests_check_their_kind() {
        assert!(channel_request("exec", "", true).is_err());
        assert!(channel_request("subsystem", "sftp", true).is_err());
        assert!(channel_request("shell", "", true).is_ok());
        assert!(channel_request("pty-req", "", true).unwrap().len() > 40);
    }
}
