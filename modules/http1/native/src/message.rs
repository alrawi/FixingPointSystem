// SPDX-License-Identifier: GPL-3.0-only
//! HTTP/1.0 and HTTP/1.1 message rendering.
//!
//! A [`Request`] or [`Response`] renders the start line, the header block, the
//! empty line, and the body framed as RFC 9112 section 6 describes. The body
//! is length delimited with `Content-Length` or, in HTTP/1.1, carried in
//! chunks with `Transfer-Encoding: chunked`. Every text field is checked
//! before any byte is written, so a value that would break the framing, such
//! as a header value with a line break, fails the call with
//! [`FnErrorKind::InvalidArgument`].

use std::io::Write as _;

use fps_flow::{FnError, FnErrorKind};

use crate::args::Args;

/// The largest chunk size that a call may request, 16 MiB.
pub const MAX_CHUNK_SIZE: usize = 16 << 20;

/// The protocol version of a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    /// HTTP/1.0, RFC 1945.
    Http10,
    /// HTTP/1.1, RFC 9112.
    Http11,
}

impl Version {
    /// Parses `1.0`, `1.1`, `HTTP/1.0`, or `HTTP/1.1`.
    pub fn parse(s: &str) -> Result<Self, FnError> {
        match s.strip_prefix("HTTP/").unwrap_or(s) {
            "1.0" => Ok(Version::Http10),
            "1.1" => Ok(Version::Http11),
            _ => Err(invalid(format!(
                "version must be '1.0' or '1.1', not '{s}'"
            ))),
        }
    }

    /// Returns the version as the start line writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Version::Http10 => "HTTP/1.0",
            Version::Http11 => "HTTP/1.1",
        }
    }
}

/// How the body of a message is delimited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    /// A `Content-Length` header gives the body length.
    Length,
    /// `Transfer-Encoding: chunked` with chunks of at most the given size,
    /// followed by the last chunk `0` and an empty trailer section.
    Chunked(usize),
}

/// An HTTP request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request<'a> {
    /// Method token, such as `GET` or `POST`.
    pub method: &'a str,
    /// Request target in origin form, such as `/index.html`.
    pub path: &'a str,
    /// `Host` value. HTTP/1.0 omits the header when it is empty.
    pub host: &'a str,
    /// `Accept` value, omitted when empty.
    pub accept: &'a str,
    /// Protocol version.
    pub version: Version,
    /// Body bytes.
    pub body: &'a [u8],
    /// `Content-Type` value, omitted when empty.
    pub content_type: &'a str,
    /// Body framing.
    pub framing: Framing,
    /// `Connection` value, such as `close` or `keep-alive`, omitted when empty.
    pub connection: &'a str,
    /// `User-Agent` value, omitted when empty.
    pub user_agent: &'a str,
}

/// An HTTP response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Response<'a> {
    /// Status code from 100 to 999.
    pub status: u16,
    /// Body bytes.
    pub body: &'a [u8],
    /// `Content-Type` value, omitted when empty or when the status forbids a
    /// body.
    pub content_type: &'a str,
    /// Protocol version.
    pub version: Version,
    /// Body framing.
    pub framing: Framing,
    /// `Connection` value, omitted when empty.
    pub connection: &'a str,
    /// `Server` value, omitted when empty.
    pub server: &'a str,
}

fn invalid(message: String) -> FnError {
    FnError::new(FnErrorKind::InvalidArgument, message)
}

/// Returns true for the `tchar` set of RFC 9110 section 5.6.2.
fn is_tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

fn check_token(what: &str, s: &str) -> Result<(), FnError> {
    if s.is_empty() || !s.bytes().all(is_tchar) {
        return Err(invalid(format!("{what} '{s}' is not an HTTP token")));
    }
    Ok(())
}

/// A field value may hold visible characters, spaces, and tabs, and must not
/// start or end with white space (RFC 9110 section 5.5).
fn check_field_value(what: &str, s: &str) -> Result<(), FnError> {
    let ok = s
        .bytes()
        .all(|b| b == b' ' || b == b'\t' || (0x21..=0x7e).contains(&b) || b >= 0x80)
        && !s.starts_with([' ', '\t'])
        && !s.ends_with([' ', '\t']);
    if !ok {
        return Err(invalid(format!(
            "{what} must not contain control characters or surrounding white space"
        )));
    }
    Ok(())
}

fn check_target(path: &str) -> Result<(), FnError> {
    if path.is_empty() || !path.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err(invalid(format!(
            "path '{path}' must be a non empty request target without spaces"
        )));
    }
    Ok(())
}

fn framing(version: Version, chunked: bool, chunk_size: i64) -> Result<Framing, FnError> {
    if !chunked {
        return Ok(Framing::Length);
    }
    if version == Version::Http10 {
        return Err(invalid(
            "chunked transfer coding needs HTTP/1.1".to_string(),
        ));
    }
    match usize::try_from(chunk_size) {
        Ok(n) if (1..=MAX_CHUNK_SIZE).contains(&n) => Ok(Framing::Chunked(n)),
        _ => Err(invalid(format!(
            "chunk_size must lie in [1, {MAX_CHUNK_SIZE}], not {chunk_size}"
        ))),
    }
}

fn header(out: &mut Vec<u8>, name: &str, value: &str) {
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(b": ");
    out.extend_from_slice(value.as_bytes());
    out.extend_from_slice(b"\r\n");
}

fn framing_header(out: &mut Vec<u8>, framing: Framing, len: usize) {
    match framing {
        Framing::Length => {
            out.extend_from_slice(b"Content-Length: ");
            // Writing to a Vec cannot fail.
            let _ = write!(out, "{len}");
            out.extend_from_slice(b"\r\n");
        }
        Framing::Chunked(_) => out.extend_from_slice(b"Transfer-Encoding: chunked\r\n"),
    }
}

/// Appends `body` with the given framing. A chunked body ends with the last
/// chunk and an empty trailer section.
fn write_body(out: &mut Vec<u8>, framing: Framing, body: &[u8]) {
    match framing {
        Framing::Length => out.extend_from_slice(body),
        Framing::Chunked(size) => {
            for chunk in body.chunks(size) {
                let _ = write!(out, "{:x}\r\n", chunk.len());
                out.extend_from_slice(chunk);
                out.extend_from_slice(b"\r\n");
            }
            out.extend_from_slice(b"0\r\n\r\n");
        }
    }
}

/// Methods whose requests carry `Content-Length: 0` even without a body, as
/// RFC 9110 section 8.6 recommends for methods that expect content.
fn expects_content(method: &str) -> bool {
    matches!(method, "POST" | "PUT" | "PATCH")
}

impl<'a> Request<'a> {
    pub(crate) fn from_args(a: &Args<'a>) -> Result<Self, FnError> {
        let version = Version::parse(a.text("version")?)?;
        Ok(Request {
            method: a.text("method")?,
            path: a.text("path")?,
            host: a.text("host")?,
            accept: a.text("accept")?,
            version,
            body: a.bytes("body")?,
            content_type: a.text("content_type")?,
            framing: framing(version, a.boolean("chunked")?, a.int("chunk_size")?)?,
            connection: a.text("connection")?,
            user_agent: a.text("user_agent")?,
        })
    }

    fn check(&self) -> Result<(), FnError> {
        check_token("method", self.method)?;
        check_target(self.path)?;
        if self.host.is_empty() {
            if self.version == Version::Http11 {
                return Err(invalid("an HTTP/1.1 request needs a host".to_string()));
            }
        } else if !self.host.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
            return Err(invalid(format!(
                "host '{}' is not a valid authority",
                self.host
            )));
        }
        check_field_value("accept", self.accept)?;
        check_field_value("content_type", self.content_type)?;
        check_field_value("user_agent", self.user_agent)?;
        if !self.connection.is_empty() {
            check_field_value("connection", self.connection)?;
        }
        Ok(())
    }

    /// Appends the request to `out`.
    pub fn render(&self, out: &mut Vec<u8>) -> Result<(), FnError> {
        self.check()?;
        out.reserve(256 + self.body.len());
        out.extend_from_slice(self.method.as_bytes());
        out.push(b' ');
        out.extend_from_slice(self.path.as_bytes());
        out.push(b' ');
        out.extend_from_slice(self.version.as_str().as_bytes());
        out.extend_from_slice(b"\r\n");
        if !self.host.is_empty() {
            header(out, "Host", self.host);
        }
        if !self.user_agent.is_empty() {
            header(out, "User-Agent", self.user_agent);
        }
        if !self.accept.is_empty() {
            header(out, "Accept", self.accept);
        }
        if !self.content_type.is_empty() {
            header(out, "Content-Type", self.content_type);
        }
        if matches!(self.framing, Framing::Chunked(_))
            || !self.body.is_empty()
            || expects_content(self.method)
        {
            framing_header(out, self.framing, self.body.len());
        }
        if !self.connection.is_empty() {
            header(out, "Connection", self.connection);
        }
        out.extend_from_slice(b"\r\n");
        if matches!(self.framing, Framing::Chunked(_)) || !self.body.is_empty() {
            write_body(out, self.framing, self.body);
        }
        Ok(())
    }
}

/// Returns the reason phrase of a status code, or `Unknown` for a code that
/// RFC 9110 does not register.
pub fn reason_phrase(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        413 => "Content Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Unknown",
    }
}

/// Returns true for status codes whose responses never carry content
/// (RFC 9110 section 6.4.1).
fn forbids_body(status: u16) -> bool {
    (100..200).contains(&status) || status == 204 || status == 304
}

impl<'a> Response<'a> {
    pub(crate) fn from_args(a: &Args<'a>) -> Result<Self, FnError> {
        let version = Version::parse(a.text("version")?)?;
        let raw = a.int("status")?;
        let status = u16::try_from(raw)
            .ok()
            .filter(|s| (100..=999).contains(s))
            .ok_or_else(|| invalid(format!("status must lie in [100, 999], not {raw}")))?;
        Ok(Response {
            status,
            body: a.bytes("body")?,
            content_type: a.text("content_type")?,
            version,
            framing: framing(version, a.boolean("chunked")?, a.int("chunk_size")?)?,
            connection: a.text("connection")?,
            server: a.text("server")?,
        })
    }

    fn check(&self) -> Result<(), FnError> {
        if forbids_body(self.status) && !self.body.is_empty() {
            return Err(invalid(format!(
                "a {} response must not carry a body",
                self.status
            )));
        }
        check_field_value("content_type", self.content_type)?;
        check_field_value("connection", self.connection)?;
        check_field_value("server", self.server)?;
        Ok(())
    }

    /// Appends the response to `out`.
    pub fn render(&self, out: &mut Vec<u8>) -> Result<(), FnError> {
        self.check()?;
        out.reserve(256 + self.body.len());
        out.extend_from_slice(self.version.as_str().as_bytes());
        // Writing to a Vec cannot fail.
        let _ = write!(out, " {} {}\r\n", self.status, reason_phrase(self.status));
        if !self.server.is_empty() {
            header(out, "Server", self.server);
        }
        let has_body = !forbids_body(self.status);
        if has_body {
            if !self.content_type.is_empty() {
                header(out, "Content-Type", self.content_type);
            }
            framing_header(out, self.framing, self.body.len());
        }
        if !self.connection.is_empty() {
            header(out, "Connection", self.connection);
        }
        out.extend_from_slice(b"\r\n");
        if has_body {
            write_body(out, self.framing, self.body);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(path: &'static str) -> Request<'static> {
        Request {
            method: "GET",
            path,
            host: "www.example.test",
            accept: "*/*",
            version: Version::Http11,
            body: b"",
            content_type: "",
            framing: Framing::Length,
            connection: "",
            user_agent: "ua",
        }
    }

    fn ok(body: &'static [u8]) -> Response<'static> {
        Response {
            status: 200,
            body,
            content_type: "text/plain",
            version: Version::Http11,
            framing: Framing::Length,
            connection: "",
            server: "fps/0.1",
        }
    }

    fn text(r: Result<Vec<u8>, FnError>) -> String {
        String::from_utf8(r.unwrap()).unwrap()
    }

    fn req(r: Request<'_>) -> Result<Vec<u8>, FnError> {
        let mut out = Vec::new();
        r.render(&mut out).map(|()| out)
    }

    fn resp(r: Response<'_>) -> Result<Vec<u8>, FnError> {
        let mut out = Vec::new();
        r.render(&mut out).map(|()| out)
    }

    #[test]
    fn get_has_no_framing_header() {
        assert_eq!(
            text(req(get("/a"))),
            "GET /a HTTP/1.1\r\nHost: www.example.test\r\nUser-Agent: ua\r\nAccept: */*\r\n\r\n"
        );
    }

    #[test]
    fn post_carries_content_length_and_body() {
        let mut r = get("/up");
        r.method = "POST";
        r.body = b"abc";
        r.content_type = "text/plain";
        assert_eq!(
            text(req(r)),
            "POST /up HTTP/1.1\r\nHost: www.example.test\r\nUser-Agent: ua\r\nAccept: */*\r\n\
             Content-Type: text/plain\r\nContent-Length: 3\r\n\r\nabc"
        );
        r.body = b"";
        assert!(text(req(r)).ends_with("Content-Length: 0\r\n\r\n"));
    }

    #[test]
    fn chunked_request_splits_the_body() {
        let mut r = get("/up");
        r.method = "PUT";
        r.body = b"abcdefghijklmnopq";
        r.framing = Framing::Chunked(16);
        assert!(text(req(r)).ends_with(
            "Transfer-Encoding: chunked\r\n\r\n10\r\nabcdefghijklmnop\r\n1\r\nq\r\n0\r\n\r\n"
        ));
    }

    #[test]
    fn http10_request_may_omit_host() {
        let mut r = get("/");
        r.version = Version::Http10;
        r.host = "";
        r.connection = "keep-alive";
        assert_eq!(
            text(req(r)),
            "GET / HTTP/1.0\r\nUser-Agent: ua\r\nAccept: */*\r\nConnection: keep-alive\r\n\r\n"
        );
        r.version = Version::Http11;
        assert_eq!(req(r).unwrap_err().kind, FnErrorKind::InvalidArgument);
    }

    #[test]
    fn response_with_length_and_chunks() {
        assert_eq!(
            text(resp(ok(b"hello"))),
            "HTTP/1.1 200 OK\r\nServer: fps/0.1\r\nContent-Type: text/plain\r\n\
             Content-Length: 5\r\n\r\nhello"
        );
        let mut r = ok(b"hello");
        r.framing = Framing::Chunked(2);
        r.connection = "close";
        assert_eq!(
            text(resp(r)),
            "HTTP/1.1 200 OK\r\nServer: fps/0.1\r\nContent-Type: text/plain\r\n\
             Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n\
             2\r\nhe\r\n2\r\nll\r\n1\r\no\r\n0\r\n\r\n"
        );
        r.body = b"";
        assert!(text(resp(r)).ends_with("\r\n\r\n0\r\n\r\n"));
    }

    #[test]
    fn bodiless_statuses_have_no_framing() {
        let mut r = ok(b"");
        r.status = 304;
        assert_eq!(
            text(resp(r)),
            "HTTP/1.1 304 Not Modified\r\nServer: fps/0.1\r\n\r\n"
        );
        r.body = b"x";
        assert_eq!(resp(r).unwrap_err().kind, FnErrorKind::InvalidArgument);
    }

    #[test]
    fn unknown_status_gets_a_generic_reason() {
        let mut r = ok(b"");
        r.status = 599;
        r.version = Version::Http10;
        assert!(text(resp(r)).starts_with("HTTP/1.0 599 Unknown\r\n"));
    }

    #[test]
    fn rejects_values_that_break_framing() {
        let mut r = get("/a b");
        assert!(req(r).is_err());
        r = get("/a");
        r.method = "G T";
        assert!(req(r).is_err());
        r = get("/a");
        r.user_agent = "x\r\nInjected: 1";
        assert!(req(r).is_err());
        r = get("/a");
        r.host = "a\nb";
        assert!(req(r).is_err());
        let mut s = ok(b"");
        s.server = " padded";
        assert!(resp(s).is_err());
    }

    #[test]
    fn versions_and_chunk_sizes_are_checked() {
        assert_eq!(Version::parse("HTTP/1.1").unwrap(), Version::Http11);
        assert_eq!(Version::parse("1.0").unwrap(), Version::Http10);
        assert!(Version::parse("2").is_err());
        assert!(framing(Version::Http10, true, 10).is_err());
        assert!(framing(Version::Http11, true, 0).is_err());
        assert!(framing(Version::Http11, true, -1).is_err());
        assert!(framing(Version::Http11, true, (MAX_CHUNK_SIZE + 1) as i64).is_err());
        assert_eq!(
            framing(Version::Http11, true, 5).unwrap(),
            Framing::Chunked(5)
        );
        assert_eq!(framing(Version::Http10, false, 0).unwrap(), Framing::Length);
    }
}
