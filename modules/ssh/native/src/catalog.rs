// SPDX-License-Identifier: GPL-3.0-only
//! The template function catalog of the `ssh` module.
//!
//! The entries mirror the `functions` list of `modules/ssh/manifest.yaml`
//! name for name, parameter for parameter, in the same order and with the same
//! types and defaults, as the contract section "Template function catalog"
//! requires.

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

/// The largest number of bytes that `chunk` renders in one call, which is the
/// largest `SSH_MSG_CHANNEL_DATA` payload that the module sends.
pub const MAX_CHUNK: i64 = 32_768;

/// `ssh.credential`, a synthetic user name or password.
pub const CREDENTIAL: FunctionSpec = FunctionSpec {
    name: "credential",
    params: &[
        req("field", ValueType::Text),
        opt("seed", ValueType::Int, DefaultValue::Int(-1)),
    ],
};

/// `ssh.file_name`, a synthetic file name with an extension.
pub const FILE_NAME: FunctionSpec = FunctionSpec {
    name: "file_name",
    params: &[
        req("seed", ValueType::Int),
        opt("ext", ValueType::Text, DefaultValue::Text("bin")),
    ],
};

/// `ssh.command`, a command line that an operator types.
pub const COMMAND: FunctionSpec = FunctionSpec {
    name: "command",
    params: &[opt("seed", ValueType::Int, DefaultValue::Int(-1))],
};

/// `ssh.banner`, the identification line of this side.
pub const BANNER: FunctionSpec = FunctionSpec {
    name: "banner",
    params: &[opt(
        "software",
        ValueType::Text,
        DefaultValue::Text("OpenSSH_9.6p1 Ubuntu-3ubuntu13.5"),
    )],
};

/// `ssh.peer_banner`, which checks and stores the identification line of the peer.
pub const PEER_BANNER: FunctionSpec = FunctionSpec {
    name: "peer_banner",
    params: &[req("line", ValueType::Bytes)],
};

/// `ssh.kexinit`, the `SSH_MSG_KEXINIT` packet.
pub const KEXINIT: FunctionSpec = FunctionSpec {
    name: "kexinit",
    params: &[],
};

/// `ssh.ecdh_init`, the client's `SSH_MSG_KEX_ECDH_INIT` packet.
pub const ECDH_INIT: FunctionSpec = FunctionSpec {
    name: "ecdh_init",
    params: &[],
};

/// `ssh.ecdh_reply`, the server's `SSH_MSG_KEX_ECDH_REPLY` packet.
pub const ECDH_REPLY: FunctionSpec = FunctionSpec {
    name: "ecdh_reply",
    params: &[opt("host_key_seed", ValueType::Int, DefaultValue::Int(1))],
};

/// `ssh.newkeys`, the `SSH_MSG_NEWKEYS` packet, after which this side encrypts.
pub const NEWKEYS: FunctionSpec = FunctionSpec {
    name: "newkeys",
    params: &[],
};

/// `ssh.size`, the number of bytes that follow the 4 byte head of a packet.
pub const SIZE: FunctionSpec = FunctionSpec {
    name: "size",
    params: &[req("head", ValueType::Bytes)],
};

/// `ssh.recv`, which opens a received packet and applies it to the session.
pub const RECV: FunctionSpec = FunctionSpec {
    name: "recv",
    params: &[
        req("packet", ValueType::Bytes),
        opt("expect", ValueType::Text, DefaultValue::Text("any")),
    ],
};

/// `ssh.service_request`, the `SSH_MSG_SERVICE_REQUEST` packet.
pub const SERVICE_REQUEST: FunctionSpec = FunctionSpec {
    name: "service_request",
    params: &[opt(
        "name",
        ValueType::Text,
        DefaultValue::Text("ssh-userauth"),
    )],
};

/// `ssh.service_accept`, the `SSH_MSG_SERVICE_ACCEPT` packet.
pub const SERVICE_ACCEPT: FunctionSpec = FunctionSpec {
    name: "service_accept",
    params: &[opt(
        "name",
        ValueType::Text,
        DefaultValue::Text("ssh-userauth"),
    )],
};

/// `ssh.auth_none`, a `none` authentication request.
pub const AUTH_NONE: FunctionSpec = FunctionSpec {
    name: "auth_none",
    params: &[req("user", ValueType::Text)],
};

/// `ssh.auth_password`, a `password` authentication request.
pub const AUTH_PASSWORD: FunctionSpec = FunctionSpec {
    name: "auth_password",
    params: &[
        req("user", ValueType::Text),
        req("password", ValueType::Bytes),
    ],
};

/// `ssh.auth_failure`, the `SSH_MSG_USERAUTH_FAILURE` packet.
pub const AUTH_FAILURE: FunctionSpec = FunctionSpec {
    name: "auth_failure",
    params: &[opt(
        "methods",
        ValueType::Text,
        DefaultValue::Text("publickey,password"),
    )],
};

/// `ssh.auth_success`, the `SSH_MSG_USERAUTH_SUCCESS` packet.
pub const AUTH_SUCCESS: FunctionSpec = FunctionSpec {
    name: "auth_success",
    params: &[],
};

/// `ssh.channel_open`, which opens a `session` channel.
pub const CHANNEL_OPEN: FunctionSpec = FunctionSpec {
    name: "channel_open",
    params: &[
        opt("window", ValueType::Int, DefaultValue::Int(2097152)),
        opt("max_packet", ValueType::Int, DefaultValue::Int(32768)),
    ],
};

/// `ssh.channel_confirm`, which confirms the channel.
pub const CHANNEL_CONFIRM: FunctionSpec = FunctionSpec {
    name: "channel_confirm",
    params: &[
        opt("window", ValueType::Int, DefaultValue::Int(2097152)),
        opt("max_packet", ValueType::Int, DefaultValue::Int(32768)),
    ],
};

/// `ssh.channel_request`, a `pty-req`, `shell`, or `exec` request.
pub const CHANNEL_REQUEST: FunctionSpec = FunctionSpec {
    name: "channel_request",
    params: &[
        req("kind", ValueType::Text),
        opt("command", ValueType::Bytes, DefaultValue::Bytes(b"")),
        opt("reply", ValueType::Boolean, DefaultValue::Boolean(true)),
    ],
};

/// `ssh.channel_success`, the `SSH_MSG_CHANNEL_SUCCESS` packet.
pub const CHANNEL_SUCCESS: FunctionSpec = FunctionSpec {
    name: "channel_success",
    params: &[],
};

/// `ssh.exit_status`, the `exit-status` request of a finished command.
pub const EXIT_STATUS: FunctionSpec = FunctionSpec {
    name: "exit_status",
    params: &[opt("code", ValueType::Int, DefaultValue::Int(0))],
};

/// `ssh.channel_data`, a `SSH_MSG_CHANNEL_DATA` packet around the given bytes.
pub const CHANNEL_DATA: FunctionSpec = FunctionSpec {
    name: "channel_data",
    params: &[req("data", ValueType::Bytes)],
};

/// `ssh.window_adjust`, a `SSH_MSG_CHANNEL_WINDOW_ADJUST` packet.
pub const WINDOW_ADJUST: FunctionSpec = FunctionSpec {
    name: "window_adjust",
    params: &[req("count", ValueType::Int)],
};

/// `ssh.channel_eof`, the `SSH_MSG_CHANNEL_EOF` packet.
pub const CHANNEL_EOF: FunctionSpec = FunctionSpec {
    name: "channel_eof",
    params: &[],
};

/// `ssh.channel_close`, the `SSH_MSG_CHANNEL_CLOSE` packet.
pub const CHANNEL_CLOSE: FunctionSpec = FunctionSpec {
    name: "channel_close",
    params: &[],
};

/// `ssh.disconnect`, the `SSH_MSG_DISCONNECT` packet.
pub const DISCONNECT: FunctionSpec = FunctionSpec {
    name: "disconnect",
    params: &[opt(
        "reason",
        ValueType::Text,
        DefaultValue::Text("disconnected by user"),
    )],
};

/// `ssh.chunk`, exactly `size` bytes of text, typed keys, or random bytes.
pub const CHUNK: FunctionSpec = FunctionSpec {
    name: "chunk",
    params: &[
        req("size", ValueType::Int),
        opt("kind", ValueType::Text, DefaultValue::Text("text")),
    ],
};

/// `ssh.scp_header`, the `C0644 size name` line of the scp protocol.
pub const SCP_HEADER: FunctionSpec = FunctionSpec {
    name: "scp_header",
    params: &[
        req("size", ValueType::Int),
        req("name", ValueType::Text),
        opt("mode", ValueType::Text, DefaultValue::Text("0644")),
    ],
};

/// `ssh.div`, the integer quotient of two numbers.
pub const DIV: FunctionSpec = FunctionSpec {
    name: "div",
    params: &[req("a", ValueType::Int), req("b", ValueType::Int)],
};

/// `ssh.rem`, the remainder of an integer division.
pub const REM: FunctionSpec = FunctionSpec {
    name: "rem",
    params: &[req("a", ValueType::Int), req("b", ValueType::Int)],
};

/// `ssh.flag`, 1 for a number above zero and 0 otherwise.
pub const FLAG: FunctionSpec = FunctionSpec {
    name: "flag",
    params: &[req("n", ValueType::Int)],
};

/// Every template function of the module, in the order of the manifest.
pub const FUNCTIONS: &[FunctionSpec] = &[
    CREDENTIAL,
    FILE_NAME,
    COMMAND,
    BANNER,
    PEER_BANNER,
    KEXINIT,
    ECDH_INIT,
    ECDH_REPLY,
    NEWKEYS,
    SIZE,
    RECV,
    SERVICE_REQUEST,
    SERVICE_ACCEPT,
    AUTH_NONE,
    AUTH_PASSWORD,
    AUTH_FAILURE,
    AUTH_SUCCESS,
    CHANNEL_OPEN,
    CHANNEL_CONFIRM,
    CHANNEL_REQUEST,
    CHANNEL_SUCCESS,
    EXIT_STATUS,
    CHANNEL_DATA,
    WINDOW_ADJUST,
    CHANNEL_EOF,
    CHANNEL_CLOSE,
    DISCONNECT,
    CHUNK,
    SCP_HEADER,
    DIV,
    REM,
    FLAG,
];
