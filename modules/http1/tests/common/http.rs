// SPDX-License-Identifier: GPL-3.0-only
//! A parser for HTTP/1 byte streams with the framing rules of RFC 9112, which
//! the scenario tests use to check what each side wrote.
#![allow(dead_code)] // Each test file uses a different subset of the helpers.

/// One parsed HTTP/1 message head and the length of its body.
#[derive(Debug)]
pub struct Message {
    pub start: String,
    pub headers: Vec<(String, String)>,
    pub body_len: usize,
    pub chunked: bool,
}

impl Message {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

pub fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Splits a byte stream into HTTP/1 messages with the framing rules of RFC
/// 9112 section 6. `responses` selects the rule that a 1xx, 204, or 304
/// response has no body.
pub fn parse_stream(mut data: &[u8], responses: bool) -> Vec<Message> {
    let mut out = Vec::new();
    while !data.is_empty() {
        let end = find(data, b"\r\n\r\n").expect("header block ends");
        let head = std::str::from_utf8(&data[..end]).expect("head is UTF-8");
        let mut lines = head.split("\r\n");
        let start = lines.next().unwrap().to_string();
        let headers: Vec<(String, String)> = lines
            .map(|l| {
                let (n, v) = l.split_once(": ").expect("field line");
                (n.to_string(), v.to_string())
            })
            .collect();
        data = &data[end + 4..];
        let mut m = Message {
            start,
            headers,
            body_len: 0,
            chunked: false,
        };
        let status: Option<u16> =
            responses.then(|| m.start.split(' ').nth(1).unwrap().parse().unwrap());
        let bodiless = status.is_some_and(|s| s < 200 || s == 204 || s == 304);
        if bodiless {
        } else if m.header("Transfer-Encoding") == Some("chunked") {
            m.chunked = true;
            loop {
                let line_end = find(data, b"\r\n").unwrap();
                let size =
                    usize::from_str_radix(std::str::from_utf8(&data[..line_end]).unwrap(), 16)
                        .unwrap();
                data = &data[line_end + 2..];
                if size == 0 {
                    assert_eq!(&data[..2], b"\r\n", "empty trailer section");
                    data = &data[2..];
                    break;
                }
                assert_eq!(&data[size..size + 2], b"\r\n", "chunk ends with CRLF");
                m.body_len += size;
                data = &data[size + 2..];
            }
        } else if let Some(len) = m.header("Content-Length") {
            m.body_len = len.parse().unwrap();
            data = &data[m.body_len..];
        } else {
            assert!(!responses, "a response without framing");
        }
        out.push(m);
    }
    out
}
