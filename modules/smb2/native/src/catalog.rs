// SPDX-License-Identifier: GPL-3.0-only
//! The template function catalog of the `smb2` module.
//!
//! The entries are the `functions` list that `modules/smb2/manifest.yaml`
//! carries, name for name, parameter for parameter, in the same order and with
//! the same types and defaults, as the contract section "Template function
//! catalog" requires. MOD-13 adds the manifest, and `cargo run -p
//! fps-module-smb2 --example print_catalog` prints the list in manifest
//! syntax.

use fps_flow::ValueType;

/// The default of an optional parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultValue {
    /// A text default.
    Text(&'static str),
    /// An integer default.
    Int(i64),
    /// A boolean default.
    Boolean(bool),
    /// A bytes default. The manifest writes it as hexadecimal digit pairs.
    Bytes(&'static [u8]),
}

/// One parameter of a template function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParamSpec {
    /// Parameter name.
    pub name: &'static str,
    /// Declared type.
    pub ty: ValueType,
    /// The default, or `None` for a required parameter.
    pub default: Option<DefaultValue>,
}

/// One template function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FunctionSpec {
    /// Function name without the module prefix.
    pub name: &'static str,
    /// What the function renders, in one sentence, for the manifest.
    pub summary: &'static str,
    /// Parameters in positional order.
    pub params: &'static [ParamSpec],
}

const fn req(name: &'static str, ty: ValueType) -> ParamSpec {
    ParamSpec {
        name,
        ty,
        default: None,
    }
}

const fn opt(name: &'static str, ty: ValueType, default: DefaultValue) -> ParamSpec {
    ParamSpec {
        name,
        ty,
        default: Some(default),
    }
}

use DefaultValue::{Boolean as B, Bytes as Y, Int as I, Text as T};
use ValueType::{Boolean, Bytes, Int, Text};

/// The default of `credit_charge`, which lets the function choose: 0 for a
/// negotiate, 1 for most messages, and one credit per 64 KiB for a read or a write.
pub const AUTO: i64 = -1;

/// The default of every time parameter, which stands for the fixed time
/// 2025-01-01T00:00:00Z so that a run is reproducible.
pub const AUTO_TIME: i64 = -1;

/// The fixed time of [`AUTO_TIME`] as a `FILETIME`, in 100 ns steps since 1601.
pub const DEFAULT_FILETIME: u64 = 133_801_632_000_000_000;

const MESSAGE_ID: ParamSpec = req("message_id", Int);
const SESSION_ID: ParamSpec = req("session_id", Int);
const TREE_ID: ParamSpec = req("tree_id", Int);
const CREDIT_CHARGE: ParamSpec = opt("credit_charge", Int, I(AUTO));
const CREDITS: ParamSpec = opt("credits", Int, I(1));

macro_rules! function {
    ($ident:ident, $name:literal, $summary:literal, [$($param:expr),* $(,)?]) => {
        #[doc = concat!("`smb2.", $name, "`.")]
        pub const $ident: FunctionSpec = FunctionSpec {
            name: $name,
            summary: $summary,
            params: &[$($param),*],
        };
    };
}

// Requests.

function!(NEGOTIATE, "negotiate",
    "A NEGOTIATE request that offers the dialects in the comma separated list, such as '2.0.2,2.1,3.0,3.0.2,3.1.1'. A list with 3.1.1 adds the preauthentication and encryption contexts. signing: disabled, enabled, required. A capabilities value of -1 chooses 0 for 2.x and DFS, leasing, and large MTU for 3.x. An empty client_guid draws 16 random octets.",
    [
        opt("dialects", Text, T("2.1")),
        opt("message_id", Int, I(0)),
        opt("signing", Text, T("enabled")),
        opt("capabilities", Int, I(-1)),
        opt("client_guid", Bytes, Y(b"")),
        CREDITS,
    ]);

function!(SESSION_SETUP, "session_setup",
    "A SESSION_SETUP request that carries the security buffer, a SPNEGO token such as the result of ntlm_negotiate or ntlm_authenticate. A session_id of 0 opens a session, and the second request of an exchange carries the id that the server chose.",
    [
        req("security", Bytes),
        MESSAGE_ID,
        opt("session_id", Int, I(0)),
        opt("previous_session_id", Int, I(0)),
        opt("signing", Text, T("enabled")),
        opt("binding", Boolean, B(false)),
        CREDIT_CHARGE,
        CREDITS,
    ]);

function!(
    TREE_CONNECT,
    "tree_connect",
    "A TREE_CONNECT request for a share path such as '\\\\server\\share'.",
    [
        req("path", Text),
        MESSAGE_ID,
        SESSION_ID,
        CREDIT_CHARGE,
        CREDITS
    ]
);

function!(CREATE, "create",
    "A CREATE request for a path below the share, with backslashes, or an empty name for the root of the share. The defaults open an existing file for reading (read data, attributes, and read control, share all, FILE_OPEN, FILE_NON_DIRECTORY_FILE). Use options 1 for a directory, disposition 2 to create a file that must not exist, and disposition 5 to overwrite or create.",
    [
        req("name", Text),
        MESSAGE_ID,
        SESSION_ID,
        TREE_ID,
        opt("desired_access", Int, I(0x0012_0089)),
        opt("file_attributes", Int, I(0)),
        opt("share_access", Int, I(7)),
        opt("disposition", Int, I(1)),
        opt("options", Int, I(0x40)),
        opt("oplock", Int, I(0)),
        CREDIT_CHARGE,
        CREDITS,
    ]);

function!(READ, "read",
    "A READ request for length octets at offset from the file with the 32 digit hexadecimal file id of a create response. A credit_charge of -1 charges one credit per 64 KiB.",
    [
        req("file_id", Text),
        req("length", Int),
        MESSAGE_ID,
        SESSION_ID,
        TREE_ID,
        opt("offset", Int, I(0)),
        opt("minimum_count", Int, I(0)),
        opt("remaining", Int, I(0)),
        CREDIT_CHARGE,
        CREDITS,
    ]);

function!(WRITE, "write",
    "A WRITE request that carries data at offset into the file with the 32 digit hexadecimal file id. The data is binary, so a nested call such as content.binary fits. A credit_charge of -1 charges one credit per 64 KiB.",
    [
        req("file_id", Text),
        req("data", Bytes),
        MESSAGE_ID,
        SESSION_ID,
        TREE_ID,
        opt("offset", Int, I(0)),
        opt("remaining", Int, I(0)),
        opt("flags", Int, I(0)),
        CREDIT_CHARGE,
        CREDITS,
    ]);

function!(CLOSE, "close",
    "A CLOSE request for the file with the 32 digit hexadecimal file id. A flags value of 1 asks for the attributes in the response.",
    [
        req("file_id", Text),
        MESSAGE_ID,
        SESSION_ID,
        TREE_ID,
        opt("flags", Int, I(0)),
        CREDIT_CHARGE,
        CREDITS,
    ]);

function!(
    TREE_DISCONNECT,
    "tree_disconnect",
    "A TREE_DISCONNECT request.",
    [MESSAGE_ID, SESSION_ID, TREE_ID, CREDIT_CHARGE, CREDITS]
);

function!(
    LOGOFF,
    "logoff",
    "A LOGOFF request, which ends the session.",
    [MESSAGE_ID, SESSION_ID, CREDIT_CHARGE, CREDITS]
);

function!(QUERY_DIRECTORY, "query_directory",
    "A QUERY_DIRECTORY request for the entries that match the pattern in the open directory with the 32 digit hexadecimal file id, as FileIdBothDirectoryInformation. restart starts the scan again, and single returns one entry.",
    [
        req("file_id", Text),
        MESSAGE_ID,
        SESSION_ID,
        TREE_ID,
        opt("pattern", Text, T("*")),
        opt("restart", Boolean, B(true)),
        opt("single", Boolean, B(false)),
        opt("output_length", Int, I(65536)),
        CREDIT_CHARGE,
        CREDITS,
    ]);

// Responses.

function!(NEGOTIATE_RESPONSE, "negotiate_response",
    "A NEGOTIATE response that selects the dialect, such as '2.1' or '3.1.1'. An empty security buffer stands for the SPNEGO token that offers NTLMSSP alone, which is what a server without Kerberos sends. A capabilities value of -1 chooses 0 for 2.x and DFS, leasing, and large MTU for 3.x. An empty server_guid draws 16 random octets. A time of -1 is the fixed time of the module.",
    [
        opt("dialect", Text, T("2.1")),
        opt("message_id", Int, I(0)),
        opt("signing", Text, T("enabled")),
        opt("server_guid", Bytes, Y(b"")),
        opt("max_size", Int, I(8_388_608)),
        opt("capabilities", Int, I(-1)),
        opt("security", Bytes, Y(b"")),
        opt("system_time", Int, I(AUTO_TIME)),
        opt("start_time", Int, I(AUTO_TIME)),
        CREDITS,
    ]);

function!(SESSION_SETUP_RESPONSE, "session_setup_response",
    "A SESSION_SETUP response with the security buffer and a status such as more_processing_required, which answers the first request of an exchange, or success. session_flags: 1 guest, 2 null, 4 encrypt data.",
    [
        req("security", Bytes),
        MESSAGE_ID,
        SESSION_ID,
        opt("status", Text, T("success")),
        opt("session_flags", Int, I(0)),
        CREDIT_CHARGE,
        CREDITS,
    ]);

function!(TREE_CONNECT_RESPONSE, "tree_connect_response",
    "A TREE_CONNECT response whose header carries the tree id that the server assigns. share_type: disk, pipe, print.",
    [
        MESSAGE_ID,
        SESSION_ID,
        TREE_ID,
        opt("share_type", Text, T("disk")),
        opt("share_flags", Int, I(0)),
        opt("capabilities", Int, I(0)),
        opt("maximal_access", Int, I(0x001F_01FF)),
        CREDIT_CHARGE,
        CREDITS,
    ]);

function!(CREATE_RESPONSE, "create_response",
    "A CREATE response for a file of size octets. An empty file_id draws a random 32 digit hexadecimal id, and an allocation_size of -1 rounds the size up to 4096. The attributes are 32 for a file and 16 for a directory. A time of -1 is the fixed time of the module.",
    [
        MESSAGE_ID,
        SESSION_ID,
        TREE_ID,
        opt("file_id", Text, T("")),
        opt("size", Int, I(0)),
        opt("attributes", Int, I(0x20)),
        opt("action", Int, I(1)),
        opt("oplock", Int, I(0)),
        opt("time", Int, I(AUTO_TIME)),
        opt("allocation_size", Int, I(-1)),
        CREDIT_CHARGE,
        CREDITS,
    ]);

function!(READ_RESPONSE, "read_response",
    "A READ response that carries data, with the number of octets that remain in the file after it.",
    [
        req("data", Bytes),
        MESSAGE_ID,
        SESSION_ID,
        TREE_ID,
        opt("remaining", Int, I(0)),
        CREDIT_CHARGE,
        CREDITS,
    ]);

function!(
    WRITE_RESPONSE,
    "write_response",
    "A WRITE response that acknowledges count octets.",
    [
        req("count", Int),
        MESSAGE_ID,
        SESSION_ID,
        TREE_ID,
        opt("remaining", Int, I(0)),
        CREDIT_CHARGE,
        CREDITS,
    ]
);

function!(CLOSE_RESPONSE, "close_response",
    "A CLOSE response. The times, sizes, and attributes are zero unless flags is 1. A time of -1 is the fixed time of the module, and an allocation_size of -1 rounds the size up to 4096.",
    [
        MESSAGE_ID,
        SESSION_ID,
        TREE_ID,
        opt("flags", Int, I(0)),
        opt("size", Int, I(0)),
        opt("attributes", Int, I(0x20)),
        opt("time", Int, I(AUTO_TIME)),
        opt("allocation_size", Int, I(-1)),
        CREDIT_CHARGE,
        CREDITS,
    ]);

function!(
    TREE_DISCONNECT_RESPONSE,
    "tree_disconnect_response",
    "A TREE_DISCONNECT response.",
    [MESSAGE_ID, SESSION_ID, TREE_ID, CREDIT_CHARGE, CREDITS]
);

function!(
    LOGOFF_RESPONSE,
    "logoff_response",
    "A LOGOFF response.",
    [MESSAGE_ID, SESSION_ID, CREDIT_CHARGE, CREDITS]
);

function!(QUERY_DIRECTORY_RESPONSE, "query_directory_response",
    "A QUERY_DIRECTORY response for the entries, one per line as 'size kind name' where kind is file or directory and the name may hold spaces. A time of -1 is the fixed time of the module.",
    [
        req("entries", Text),
        MESSAGE_ID,
        SESSION_ID,
        TREE_ID,
        opt("time", Int, I(AUTO_TIME)),
        CREDIT_CHARGE,
        CREDITS,
    ]);

function!(ERROR_RESPONSE, "error_response",
    "An error response to a request of the named command, with a status such as access_denied, object_name_not_found, end_of_file, no_more_files, or logon_failure, or a number such as 0xC0000022.",
    [
        req("command", Text),
        req("status", Text),
        MESSAGE_ID,
        opt("session_id", Int, I(0)),
        opt("tree_id", Int, I(0)),
        CREDIT_CHARGE,
        CREDITS,
    ]);

// NTLM and SPNEGO.

function!(NTLM_NEGOTIATE, "ntlm_negotiate",
    "The NTLMSSP NEGOTIATE_MESSAGE of a client, wrapped in a SPNEGO NegTokenInit unless spnego is false. A flags value of -1 is what a Windows client offers.",
    [
        opt("domain", Text, T("")),
        opt("workstation", Text, T("")),
        opt("flags", Int, I(-1)),
        opt("spnego", Boolean, B(true)),
    ]);

function!(NTLM_CHALLENGE, "ntlm_challenge",
    "The NTLMSSP CHALLENGE_MESSAGE of a server with target information that names the domain and computer, wrapped in a SPNEGO NegTokenResp with the state accept-incomplete unless spnego is false. An empty server_challenge draws 8 random octets. A time of -1 is the fixed time of the module.",
    [
        opt("target", Text, T("FPS")),
        opt("computer", Text, T("FPSSERVER")),
        opt("dns_domain", Text, T("")),
        opt("dns_computer", Text, T("")),
        opt("server_challenge", Bytes, Y(b"")),
        opt("time", Int, I(AUTO_TIME)),
        opt("flags", Int, I(-1)),
        opt("spnego", Boolean, B(true)),
    ]);

function!(NTLM_AUTHENTICATE, "ntlm_authenticate",
    "The NTLMSSP AUTHENTICATE_MESSAGE of a client that answers a challenge, with a full NTLMv2 response computed from the user and the password, wrapped in a SPNEGO NegTokenResp unless spnego is false. The challenge is the security buffer of the session setup response, or a bare challenge message. An empty client_challenge draws 8 random octets.",
    [
        req("challenge", Bytes),
        req("user", Text),
        req("password", Text),
        opt("domain", Text, T("")),
        opt("workstation", Text, T("")),
        opt("client_challenge", Bytes, Y(b"")),
        opt("time", Int, I(AUTO_TIME)),
        opt("spnego", Boolean, B(true)),
    ]);

function!(SPNEGO_ACCEPT, "spnego_accept",
    "The SPNEGO NegTokenResp with the state accept-completed, which a server sends in the final SESSION_SETUP response.",
    []);

function!(NTLM_SESSION_KEY, "ntlm_session_key",
    "The 16 octet session key, as 32 hexadecimal digits, that follows from an AUTHENTICATE_MESSAGE and the password. Both sides derive the same key, which SMB 2.x signs with.",
    [req("authenticate", Bytes), req("password", Text)]);

function!(SIGN, "sign",
    "The SMB2 message with the signed flag set and the signature written as SMB 2.x computes it, HMAC-SHA256 over the message with the session key of 32 hexadecimal digits. A transport header in front of the message stays in front.",
    [req("message", Bytes), req("key", Text)]);

// Readers.

function!(MESSAGE_LENGTH, "message_length",
    "The length of the SMB2 message that follows a 4 octet transport header, as decimal digits. A flow reads 4 octets, then this many.",
    [req("header", Bytes)]);

function!(HEADER_FIELD, "header_field",
    "A field of the SMB2 header of a message: command, command_code, status, status_code, message_id, session_id, tree_id, credits, credit_charge, flags, response, signed, async, or next_command. The message may start with a transport header.",
    [req("message", Bytes), req("field", Text)]);

function!(BODY_FIELD, "body_field",
    "A field of the body of a message, by command. negotiate: dialects or dialect, security_mode, capabilities, guid, max_read, max_write. session_setup: session_flags, previous_session_id, binding. tree_connect: path, share_type, maximal_access. create: name, file_id, size, attributes, action, desired_access, disposition, options. read, write, and close: file_id, offset, length, count, remaining, flags. query_directory: pattern, flags, output_length, entries, and entry_count.",
    [req("message", Bytes), req("field", Text)]);

function!(SECURITY_BLOB, "security_blob",
    "The security buffer of a NEGOTIATE response or a SESSION_SETUP request or response, as octets.",
    [req("message", Bytes)]);

function!(NTLM_FIELD, "ntlm_field",
    "A field of the NTLMSSP message inside a security buffer: type, flags, user, domain, workstation, target, challenge, computer, dns_domain, dns_computer, time, or nt_response.",
    [req("security", Bytes), req("field", Text)]);

function!(
    DATA,
    "data",
    "The data of a READ response or a WRITE request, as octets.",
    [req("message", Bytes)]
);

function!(HEX, "hex",
    "The octets as lowercase hexadecimal digits, which a set action keeps without loss, unlike octets.",
    [req("bytes", Bytes)]);

function!(
    UNHEX,
    "unhex",
    "The octets of hexadecimal digits, the inverse of hex.",
    [req("text", Text)]
);

function!(CHUNKS, "chunks",
    "The number of requests of at most size octets that move total octets, at least 1, as decimal digits.",
    [req("total", Int), req("size", Int)]);

function!(ADD, "add",
    "The sum of a and b as decimal digits. A flow has no arithmetic and no loop counter, so a set action numbers the messages of a loop with it.",
    [req("a", Int), opt("b", Int, I(1))]);

/// The template functions of the module, in the order of the manifest.
pub const FUNCTIONS: &[FunctionSpec] = &[
    NEGOTIATE,
    SESSION_SETUP,
    TREE_CONNECT,
    CREATE,
    READ,
    WRITE,
    CLOSE,
    TREE_DISCONNECT,
    LOGOFF,
    QUERY_DIRECTORY,
    NEGOTIATE_RESPONSE,
    SESSION_SETUP_RESPONSE,
    TREE_CONNECT_RESPONSE,
    CREATE_RESPONSE,
    READ_RESPONSE,
    WRITE_RESPONSE,
    CLOSE_RESPONSE,
    TREE_DISCONNECT_RESPONSE,
    LOGOFF_RESPONSE,
    QUERY_DIRECTORY_RESPONSE,
    ERROR_RESPONSE,
    NTLM_NEGOTIATE,
    NTLM_CHALLENGE,
    NTLM_AUTHENTICATE,
    SPNEGO_ACCEPT,
    NTLM_SESSION_KEY,
    SIGN,
    MESSAGE_LENGTH,
    HEADER_FIELD,
    BODY_FIELD,
    SECURITY_BLOB,
    NTLM_FIELD,
    DATA,
    HEX,
    UNHEX,
    CHUNKS,
    ADD,
];

fn yaml_text(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Renders [`FUNCTIONS`] as the `functions` list of a module manifest, in the
/// syntax of `modules/imap/manifest.yaml`. MOD-13 pastes the result into
/// `modules/smb2/manifest.yaml`, and the catalog test compares the two.
pub fn manifest_functions() -> String {
    let mut out = String::from("functions:\n");
    for f in FUNCTIONS {
        out.push_str(&format!("  - name: {}\n", f.name));
        out.push_str(&format!("    summary: {}\n", yaml_text(f.summary)));
        if f.params.is_empty() {
            out.push_str("    params: []\n");
            continue;
        }
        out.push_str("    params:\n");
        for p in f.params {
            let mut line = format!("      - {{ name: {}, type: {}", p.name, p.ty);
            match p.default {
                None => {}
                Some(DefaultValue::Text(t)) => {
                    line.push_str(&format!(", default: {}", yaml_text(t)))
                }
                Some(DefaultValue::Int(i)) => line.push_str(&format!(", default: {i}")),
                Some(DefaultValue::Boolean(b)) => line.push_str(&format!(", default: {b}")),
                Some(DefaultValue::Bytes(b)) => {
                    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
                    line.push_str(&format!(", default: {}", yaml_text(&hex)));
                }
            }
            line.push_str(" }\n");
            out.push_str(&line);
        }
    }
    out
}
