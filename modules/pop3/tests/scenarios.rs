// SPDX-License-Identifier: GPL-3.0-only
//! Interpreter tests: every pop3 scenario runs with its client and server
//! against each other over the in memory duplex, and the bytes in each
//! direction parse as the POP3 dialogue that the scenario describes. The
//! parser below follows RFC 1939 for the commands, the status lines, the multi
//! line responses with dot stuffing, and RFC 2045 for the messages that RETR
//! carries.

mod common;

use common::{run, scenarios, scenarios_with};
use fps_flow::{Scenario, Value};

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn unbase64(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes().filter(|&c| c != b'=' && c != b'\r' && c != b'\n') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("not base64: {c}"),
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

/// A parsed message: the header lines, the text part, and the decoded
/// attachment.
#[derive(Debug)]
struct Mail {
    headers: Vec<(String, String)>,
    text: String,
    attachment: Option<(String, String, Vec<u8>)>,
}

impl Mail {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn split_head(raw: &str) -> (Vec<(String, String)>, &str) {
    let (head, body) = raw
        .split_once("\r\n\r\n")
        .expect("a blank line ends the headers");
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in head.split("\r\n") {
        if line.starts_with([' ', '\t']) {
            let last = headers.last_mut().expect("a folded line follows a header");
            last.1.push(' ');
            last.1.push_str(line.trim());
        } else {
            let (n, v) = line.split_once(": ").expect("a header line");
            headers.push((n.to_string(), v.to_string()));
        }
    }
    (headers, body)
}

fn parse_mail(message: &[u8]) -> Mail {
    let raw = std::str::from_utf8(message).expect("the test messages are ASCII");
    assert!(raw.is_ascii());
    let (headers, body) = split_head(raw);
    let mail = |text: String, attachment| Mail {
        headers: headers.clone(),
        text,
        attachment,
    };
    let ctype = headers
        .iter()
        .find(|(n, _)| n == "Content-Type")
        .map(|(_, v)| v.clone())
        .expect("Content-Type");
    let Some(boundary) = ctype
        .strip_prefix("multipart/mixed; boundary=\"")
        .map(|b| b.trim_end_matches('"').to_string())
    else {
        assert!(ctype.starts_with("text/plain"), "{ctype}");
        return mail(body.to_string(), None);
    };
    let delimiter = format!("--{boundary}");
    let parts: Vec<&str> = body.split(&delimiter).collect();
    // Preamble, the text part, the attachment, and the epilogue after the close.
    assert_eq!(parts.len(), 4, "{}", parts.len());
    assert!(parts[0].starts_with("This is a multi-part message"));
    assert_eq!(parts[3], "--\r\n");
    let (text_headers, text) = split_head(parts[1].strip_prefix("\r\n").unwrap());
    assert!(text_headers
        .iter()
        .any(|(n, v)| n == "Content-Type" && v.starts_with("text/plain")));
    let (att_headers, encoded) = split_head(parts[2].strip_prefix("\r\n").unwrap());
    let get = |n: &str| {
        att_headers
            .iter()
            .find(|(h, _)| h == n)
            .map(|(_, v)| v.clone())
            .unwrap()
    };
    assert_eq!(get("Content-Transfer-Encoding"), "base64");
    for line in encoded.trim_end().split("\r\n") {
        assert!(
            line.len() <= 76,
            "a base64 line of {} characters",
            line.len()
        );
    }
    let name = get("Content-Disposition")
        .strip_prefix("attachment; filename=\"")
        .and_then(|d| d.strip_suffix('"'))
        .expect("a disposition with a file name")
        .to_string();
    let ct = get("Content-Type");
    let ct = ct.split(';').next().unwrap().to_string();
    mail(text.to_string(), Some((name, ct, unbase64(encoded))))
}

/// Checks a message that a literal carries and returns its attachment size.
/// `kinds` lists the attachments that the scenario may draw.
fn check_message(name: &str, literal: &[u8], kinds: &[(&[u8], &str, &str)]) -> usize {
    let m = parse_mail(literal);
    for h in [
        "From",
        "To",
        "Subject",
        "Date",
        "Message-ID",
        "MIME-Version",
    ] {
        assert!(m.header(h).is_some(), "{name}: no {h} header");
    }
    assert!(
        m.header("From").unwrap().ends_with("@example.test"),
        "{name}"
    );
    assert!(m.header("Date").unwrap().contains(" 2026 "), "{name}");
    assert!(
        m.text.contains("Ref: "),
        "{name}: the text part comes from content.email"
    );
    let (file, ctype, bytes) = m.attachment.expect("an attachment");
    let hit = kinds
        .iter()
        .find(|(magic, f, c)| *f == file && *c == ctype && bytes.starts_with(magic));
    assert!(hit.is_some(), "{name}: {file} {ctype} {:02x?}", &bytes[..8]);
    bytes.len()
}

fn var(s: &Scenario, name: &str) -> usize {
    s.vars
        .get(name)
        .and_then(Value::as_i64)
        .unwrap_or_else(|| panic!("no variable {name}")) as usize
}

/// One command of the client.
#[derive(Debug)]
struct Cmd {
    /// The command name in upper case.
    name: String,
    /// The arguments.
    args: Vec<String>,
    /// The command line without its CRLF.
    line: String,
}

impl Cmd {
    /// Whether the response is multi line, as RFC 1939 and RFC 2449 define.
    fn multi_line(&self) -> bool {
        match self.name.as_str() {
            "CAPA" | "RETR" | "TOP" => true,
            "LIST" | "UIDL" => self.args.is_empty(),
            _ => false,
        }
    }
}

fn parse_client(rest: &[u8]) -> Vec<Cmd> {
    let text = std::str::from_utf8(rest).expect("commands are ASCII");
    assert!(text.is_empty() || text.ends_with("\r\n"));
    text.split_terminator("\r\n")
        .map(|line| {
            assert!(!line.contains(['\r', '\n']), "{line}");
            let mut w = line.split(' ');
            let name = w.next().unwrap().to_string();
            assert!(name.bytes().all(|b| b.is_ascii_uppercase()), "{line}");
            assert!((3..=4).contains(&name.len()), "{line}");
            Cmd {
                name,
                args: w.map(str::to_string).collect(),
                line: line.to_string(),
            }
        })
        .collect()
}

/// One response of the server.
#[derive(Debug)]
struct Resp {
    /// The status line without its CRLF.
    status: String,
    /// The payload of a multi line response as it stands on the wire, dot
    /// stuffed and with the closing line.
    wire: Vec<u8>,
    /// The payload after the removal of the stuffing and the closing line.
    payload: Vec<u8>,
}

/// Removes the stuffing of RFC 1939 from the lines before the closing line.
fn unstuff(wire: &[u8]) -> Vec<u8> {
    assert!(wire.ends_with(b".\r\n"), "the closing line");
    let body = &wire[..wire.len() - 3];
    assert!(body.is_empty() || body.ends_with(b"\r\n"));
    let mut out = Vec::new();
    for line in body.split_inclusive(|&b| b == b'\n') {
        assert_ne!(line, b".\r\n", "a lone dot before the end");
        out.extend_from_slice(line.strip_prefix(b".").unwrap_or(line));
    }
    out
}

fn take_line(rest: &mut &[u8]) -> String {
    let end = find(rest, b"\r\n").expect("every line ends with CRLF");
    let l = String::from_utf8(rest[..end].to_vec()).expect("status lines are ASCII");
    *rest = &rest[end + 2..];
    l
}

/// Parses the greeting and then one response per command.
fn parse_server(mut rest: &[u8], cmds: &[Cmd]) -> (String, Vec<Resp>) {
    let greeting = take_line(&mut rest);
    let mut out = Vec::new();
    for c in cmds {
        let status = take_line(&mut rest);
        assert!(
            status.starts_with("+OK") || status.starts_with("-ERR"),
            "{}: {status}",
            c.line
        );
        let (mut wire, mut payload) = (Vec::new(), Vec::new());
        if c.multi_line() && status.starts_with("+OK") {
            // The payload ends at the first line with a single dot, which is
            // the first CRLF dot CRLF, or the dot line itself when the
            // payload is empty.
            let end = match find(rest, b"\r\n.\r\n") {
                Some(p) if !rest.starts_with(b".\r\n") => p + 2,
                _ => {
                    assert!(rest.starts_with(b".\r\n"), "{}: no closing line", c.line);
                    0
                }
            };
            wire = rest[..end + 3].to_vec();
            payload = unstuff(&wire);
            rest = &rest[end + 3..];
        }
        out.push(Resp {
            status,
            wire,
            payload,
        });
    }
    assert!(
        rest.is_empty(),
        "{} bytes after the last response",
        rest.len()
    );
    (greeting, out)
}

fn number_after(status: &str, prefix: &str) -> usize {
    status
        .strip_prefix(prefix)
        .unwrap_or_else(|| panic!("{status}"))
        .split(' ')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

/// The lines of a LIST or UIDL payload as pairs.
fn pairs(payload: &[u8]) -> Vec<(usize, String)> {
    String::from_utf8(payload.to_vec())
        .unwrap()
        .split_terminator("\r\n")
        .map(|l| {
            let (n, v) = l.split_once(' ').expect("number and value");
            (n.parse().unwrap(), v.to_string())
        })
        .collect()
}

fn names(cmds: &[Cmd]) -> Vec<&str> {
    cmds.iter().map(|c| c.name.as_str()).collect()
}

type Kinds = &'static [(&'static [u8], &'static str, &'static str)];

struct Expect {
    login: &'static str,
    /// The attachments that a RETR may carry, as magic bytes, file name, and
    /// content type.
    kinds: Kinds,
    /// Whether the text part carries the dot lines.
    dots: bool,
}

fn expectations(name: &str) -> Expect {
    const ALL: Kinds = &[
        (b"%PDF-", "statement.pdf", "application/pdf"),
        (b"\xff\xd8\xff", "photo.jpg", "image/jpeg"),
        (b"PK", "archive.zip", "application/zip"),
    ];
    const DELETED: Kinds = &[
        (b"%PDF-", "receipt.pdf", "application/pdf"),
        (b"\xff\xd8\xff", "scan.jpg", "image/jpeg"),
        (b"\x1f\x8b", "export.gz", "application/gzip"),
    ];
    const APOP: Kinds = &[(b"%PDF-", "statement.pdf", "application/pdf")];
    const TOP: Kinds = &[(b"%PDF-", "invoice.pdf", "application/pdf")];
    match name {
        "retrieve_all" => Expect {
            login: "USER",
            kinds: ALL,
            dots: true,
        },
        "apop_login" | "apop_fallback" => Expect {
            login: "APOP",
            kinds: APOP,
            dots: false,
        },
        "top_preview" => Expect {
            login: "USER",
            kinds: TOP,
            dots: false,
        },
        "fetch_delete" => Expect {
            login: "USER",
            kinds: DELETED,
            dots: true,
        },
        _ => Expect {
            login: "USER",
            kinds: &[],
            dots: false,
        },
    }
}

/// Runs a scenario and checks the whole dialogue. It returns the attachment
/// sizes.
async fn check(name: &str, s: &Scenario, session_id: u64) -> Vec<usize> {
    let r = run(s, 42, session_id).await;
    assert!(r.client.is_ok(), "{name} client: {:?}", r.client);
    assert!(r.server.is_ok(), "{name} server: {:?}", r.server);
    let cmds = parse_client(&r.c2s);
    let (greeting, resps) = parse_server(&r.s2c, &cmds);
    let messages = var(s, "messages");
    let fetches = s.vars.get("fetches").and_then(Value::as_i64).unwrap_or(0) as usize;

    // The greeting carries the timestamp banner of RFC 1939.
    let banner = greeting
        .strip_prefix("+OK Dovecot ready. <")
        .and_then(|g| g.strip_suffix("@pop3.example.test>"))
        .unwrap_or_else(|| panic!("{name}: {greeting}"));
    let (pid, clock) = banner.split_once('.').expect("pid.clock");
    assert!(pid.parse::<u32>().is_ok() && clock.parse::<u64>().unwrap() > 1_767_225_600);

    let mut listed: Vec<(usize, String)> = Vec::new();
    let mut uids: Vec<(usize, String)> = Vec::new();
    let mut stat_total = None;
    let mut deleted = 0;
    let mut retrieved = Vec::new();
    for (c, resp) in cmds.iter().zip(&resps) {
        // The one rejection of the scenarios is the APOP that apop_fallback
        // sends first.
        let rejected = name == "apop_fallback" && c.name == "APOP";
        if rejected {
            assert_eq!(resp.status, "-ERR [AUTH] Authentication failed.");
        } else {
            assert!(
                resp.status.starts_with("+OK"),
                "{name}: {} {}",
                c.line,
                resp.status
            );
        }
        match c.name.as_str() {
            "CAPA" => {
                let text = String::from_utf8(resp.payload.clone()).unwrap();
                let lines: Vec<&str> = text.split("\r\n").collect();
                assert!(lines.contains(&"UIDL") && lines.contains(&"TOP"), "{name}");
                assert!(
                    !lines.contains(&"STLS"),
                    "{name}: plain POP3 offers no STLS"
                );
            }
            "USER" => {
                assert_eq!(c.args.len(), 1);
                assert!(c.args[0].ends_with("@example.test"), "{}", c.line);
            }
            "PASS" => {
                assert_eq!(c.args.len(), 1);
                assert!(
                    c.args[0].starts_with("Tst-"),
                    "{name}: synthetic secrets only"
                );
                assert_eq!(resp.status, "+OK Logged in.");
            }
            "APOP" => {
                assert_eq!(c.args.len(), 2, "{}", c.line);
                assert!(c.args[0].ends_with("@example.test"));
                let d = &c.args[1];
                assert!(
                    d.len() == 32
                        && d.bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                    "{name}: digest {d}"
                );
            }
            "STAT" => {
                assert_eq!(number_after(&resp.status, "+OK "), messages, "{name}");
                stat_total = resp
                    .status
                    .rsplit(' ')
                    .next()
                    .unwrap()
                    .parse::<usize>()
                    .ok();
            }
            "LIST" if c.args.is_empty() => {
                assert_eq!(resp.status, format!("+OK {messages} messages:"));
                listed = pairs(&resp.payload);
                let numbers: Vec<usize> = listed.iter().map(|p| p.0).collect();
                assert_eq!(numbers, (1..=messages).collect::<Vec<_>>());
                let total: usize = listed.iter().map(|p| p.1.parse::<usize>().unwrap()).sum();
                if let Some(t) = stat_total {
                    assert_eq!(t, total, "{name}: STAT and LIST agree");
                }
            }
            "LIST" => {
                let n: usize = c.args[0].parse().unwrap();
                let want = &listed[n - 1];
                assert_eq!(resp.status, format!("+OK {n} {}", want.1), "{name}");
            }
            "UIDL" if c.args.is_empty() => {
                assert_eq!(resp.status, "+OK");
                uids = pairs(&resp.payload);
                assert_eq!(uids.len(), messages, "{name}");
                for (n, u) in &uids {
                    assert_eq!(u.len(), 16, "{u}");
                    assert!(u
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b)));
                    assert_eq!(u[..8], format!("{:08X}", 1000 + n), "UID of message {n}");
                    assert_eq!(u[8..], uids[0].1[8..], "one UID validity");
                }
            }
            "UIDL" => {
                let n: usize = c.args[0].parse().unwrap();
                if uids.is_empty() {
                    assert!(
                        resp.status
                            .starts_with(&format!("+OK {n} {:08X}", 1000 + n)),
                        "{}",
                        resp.status
                    );
                } else {
                    assert_eq!(resp.status, format!("+OK {n} {}", uids[n - 1].1));
                }
            }
            "RETR" => {
                let size = number_after(&resp.status, "+OK ");
                assert_eq!(resp.status, format!("+OK {size} octets"));
                assert_eq!(resp.payload.len(), size, "{name}: the announced size");
                // Only the closing line is a lone dot on the wire.
                let mut closing = b"\r\n".to_vec();
                closing.extend_from_slice(&resp.wire);
                let lone = closing.windows(5).filter(|w| *w == b"\r\n.\r\n").count();
                assert_eq!(lone, 1, "{name}");
                retrieved.push((resp.wire.clone(), resp.payload.clone()));
            }
            "TOP" => {
                assert_eq!(resp.status, "+OK");
                let lines: usize = c.args[1].parse().unwrap();
                let text = std::str::from_utf8(&resp.payload).unwrap();
                let (head, body) = text
                    .split_once("\r\n\r\n")
                    .expect("a blank line after the headers");
                assert!(
                    head.starts_with("From: ") && head.contains("\r\nSubject: "),
                    "{name}"
                );
                let shown = body.split_terminator("\r\n").count();
                assert_eq!(shown, lines, "{name}: TOP n {lines} gives {lines} lines");
            }
            "DELE" => {
                assert_eq!(resp.status, "+OK Marked to be deleted.");
                deleted += 1;
            }
            "RSET" => assert_eq!(resp.status, "+OK"),
            "QUIT" => assert_eq!(resp.status, "+OK Logging out."),
            other => panic!("{name}: unexpected command {other}"),
        }
    }
    assert_eq!(cmds.last().unwrap().name, "QUIT", "{name}");
    assert_eq!(cmds.iter().filter(|c| c.name == "QUIT").count(), 1);

    let e = expectations(name);
    let login = cmds.iter().position(|c| c.name == e.login);
    assert!(login.is_some(), "{name}: a {} command", e.login);
    let mut sizes = Vec::new();
    for (wire, payload) in &retrieved {
        sizes.push(check_message(name, payload, e.kinds));
        if e.dots {
            let text = String::from_utf8_lossy(payload);
            assert!(text.contains("\r\n.\r\n"), "{name}: the lone dot line");
            assert!(
                text.contains("\r\n.. the "),
                "{name}: the line that starts with two dots"
            );
            // On the wire the lone dot goes out as two and two as three.
            let w = String::from_utf8_lossy(wire);
            assert!(
                w.contains("\r\n..\r\n"),
                "{name}: the lone dot goes out as two"
            );
            assert!(
                w.contains("\r\n... the "),
                "{name}: two dots go out as three"
            );
        }
    }
    match name {
        "retrieve_all" => {
            assert_eq!(
                names(&cmds)[..7],
                ["CAPA", "USER", "PASS", "STAT", "LIST", "UIDL", "RETR"]
            );
            assert_eq!(retrieved.len(), fetches);
            for (i, c) in cmds.iter().filter(|c| c.name == "RETR").enumerate() {
                assert_eq!(c.args, [(i + 1).to_string()], "RETR counts up");
            }
        }
        "apop_login" => {
            assert_eq!(
                names(&cmds),
                ["APOP", "STAT", "LIST", "UIDL", "LIST", "UIDL", "RETR", "QUIT"]
            );
        }
        "apop_fallback" => {
            assert_eq!(
                names(&cmds),
                ["APOP", "USER", "PASS", "STAT", "LIST", "RETR", "QUIT"]
            );
            // The digest and the password come from one secret, and the
            // login is the same name in both commands.
            assert_eq!(cmds[0].args[0], cmds[1].args[0]);
            assert_eq!(retrieved.len(), 1);
        }
        "top_preview" => {
            let lines = var(s, "preview_lines");
            let tops: Vec<&Cmd> = cmds.iter().filter(|c| c.name == "TOP").collect();
            assert_eq!(tops.len(), fetches);
            for c in tops {
                assert_eq!(c.args[1], lines.to_string());
            }
            assert_eq!(retrieved.len(), 1);
        }
        "fetch_delete" => {
            assert_eq!(deleted, fetches + 1);
            let n = cmds.len();
            assert_eq!(names(&cmds)[n - 3..], ["DELE", "RSET", "QUIT"]);
            // Each RETR is followed by the DELE of the same message.
            for w in cmds.windows(2) {
                if w[0].name == "RETR" {
                    assert_eq!((w[1].name.as_str(), &w[1].args), ("DELE", &w[0].args));
                }
            }
            assert_eq!(retrieved.len(), fetches);
        }
        "poll_new_mail" => {
            assert_eq!(names(&cmds), ["USER", "PASS", "STAT", "UIDL", "QUIT"]);
            assert!(retrieved.is_empty());
        }
        other => panic!("unknown scenario {other}"),
    }
    sizes
}

#[test]
fn the_manifest_lists_six_scenarios() {
    let names: Vec<String> = scenarios().into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        [
            "retrieve_all",
            "apop_login",
            "apop_fallback",
            "top_preview",
            "fetch_delete",
            "poll_new_mail"
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn every_scenario_exchanges_a_well_formed_dialogue() {
    for (name, s) in scenarios() {
        for session_id in 1..=4 {
            check(&name, &s, session_id).await;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn attachment_sizes_vary_between_sessions_within_the_configured_range() {
    for (name, s) in scenarios() {
        let mut all = Vec::new();
        for session_id in 1..=8 {
            all.extend(check(&name, &s, session_id).await);
        }
        if all.is_empty() {
            continue;
        }
        all.sort_unstable();
        all.dedup();
        assert!(all.len() >= 6, "{name}: only {} distinct sizes", all.len());
        // The generators finish the element they write, so a size may exceed
        // the maximum by a little. Base64 decoding returns the exact bytes.
        assert!(
            all[0] >= 24_000 && *all.last().unwrap() <= 160_000 + 2048,
            "{name}: {all:?}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn the_variables_change_the_size_of_the_maildrop_and_the_work() {
    let small = [
        ("messages", Value::Int(2)),
        ("fetches", Value::Int(1)),
        ("preview_lines", Value::Int(3)),
        ("attachment_min", Value::Int(500)),
        ("attachment_max", Value::Int(900)),
    ];
    for (name, s) in scenarios_with(&small) {
        let r = run(&s, 42, 1).await;
        assert!(r.client.is_ok() && r.server.is_ok(), "{name}");
        let cmds = parse_client(&r.c2s);
        let retrs = cmds.iter().filter(|c| c.name == "RETR").count();
        let want = usize::from(name != "poll_new_mail");
        assert_eq!(retrs, want, "{name}");
        // The scenarios with a minimum attachment of 500 bytes still run
        // against the checks of the full range, except for the size range.
        let sizes = check(&name, &s, 2).await;
        assert!(
            sizes.iter().all(|&n| (500..=900 + 2048).contains(&n)),
            "{name}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_one_message_maildrop_still_runs() {
    let one = [("messages", Value::Int(1)), ("fetches", Value::Int(1))];
    for (name, s) in scenarios_with(&one) {
        check(&name, &s, 3).await;
    }
}

#[tokio::test(start_paused = true)]
async fn the_apop_digest_follows_the_banner_of_each_session() {
    for wanted in ["apop_login", "apop_fallback"] {
        let scenario = scenarios()
            .into_iter()
            .find(|(n, _)| n == wanted)
            .unwrap()
            .1;
        let mut digests = std::collections::HashSet::new();
        let mut banners = std::collections::HashSet::new();
        for session_id in 1..=8 {
            let r = run(&scenario, 42, session_id).await;
            let cmds = parse_client(&r.c2s);
            digests.insert(cmds[0].args[1].clone());
            let greeting = String::from_utf8_lossy(&r.s2c);
            banners.insert(greeting.lines().next().unwrap().to_string());
        }
        assert_eq!(
            banners.len(),
            8,
            "{wanted}: every greeting has its own banner"
        );
        assert_eq!(
            digests.len(),
            8,
            "{wanted}: every digest follows its banner and secret"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn plain_pop3_offers_no_starttls_and_uses_port_110() {
    for (name, s) in scenarios() {
        assert_eq!(var(&s, "server_port"), 110, "{name}");
        let r = run(&s, 42, 1).await;
        let text = String::from_utf8_lossy(&r.s2c).into_owned();
        assert!(!text.contains("STLS"), "{name}");
        // The rejected APOP of apop_fallback is the only error status.
        assert_eq!(
            text.matches("-ERR").count(),
            usize::from(name == "apop_fallback"),
            "{name}"
        );
    }
}

/// The sessions that the estimate averages over. Sizes that the session
/// generator draws vary from session to session, so one session says little
/// about the mean that the controller needs.
const ESTIMATE_SESSIONS: u64 = 100;

/// A session of each scenario at the default parameters moves the payload
/// bytes of `expected_bytes` in the manifest, both directions of the stream
/// above any TLS layer, within 2 percent. A scenario whose size depends on the
/// session generator is averaged over [`ESTIMATE_SESSIONS`] session ids. The
/// controller turns byte weights into session shares with these estimates, so a
/// change of a template or a fixture that alters the size of a session must
/// change the manifest.
#[tokio::test(start_paused = true)]
async fn default_parameters_move_the_expected_bytes() {
    let doc: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(common::module_dir().join("manifest.yaml")).unwrap(),
    )
    .unwrap();
    let all = common::scenarios();
    let mut bad = Vec::new();
    if let Some(keys) = doc["expected_bytes"].as_mapping() {
        for key in keys.keys() {
            let key = key.as_str().unwrap_or_default();
            if !all.iter().any(|(name, _)| name == key) {
                bad.push(format!("{key}: an estimate for no scenario"));
            }
        }
    }
    for (name, s) in all {
        let mut sizes = Vec::new();
        for id in 1..=ESTIMATE_SESSIONS {
            let r = common::run(&s, common::SEED, id).await;
            assert!(r.client.is_ok(), "{name}: {:?}", r.client);
            assert!(r.server.is_ok(), "{name}: {:?}", r.server);
            sizes.push(r.c2s.len() + r.s2c.len());
        }
        let mean = sizes.iter().sum::<usize>() as f64 / sizes.len() as f64;
        match doc["expected_bytes"][name.as_str()].as_f64() {
            Some(want) if (mean - want).abs() / want < 0.02 => {}
            Some(want) => bad.push(format!(
                "{name}: {mean:.0} bytes against the estimate {want}"
            )),
            None => bad.push(format!("{name}: {mean:.0} bytes and no estimate")),
        }
        eprintln!(
            "MEASURED {name} mean {mean:.1} min {} max {}",
            sizes.iter().min().unwrap(),
            sizes.iter().max().unwrap()
        );
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
