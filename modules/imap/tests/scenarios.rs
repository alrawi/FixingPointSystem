// SPDX-License-Identifier: GPL-3.0-only
//! Interpreter tests: every imap scenario runs with its client and server
//! against each other over the in memory duplex, and the bytes in each
//! direction parse as the IMAP dialogue that the scenario describes. The
//! parser below follows RFC 3501 for tagged commands, untagged and tagged
//! responses, continuation requests, and literals, and RFC 2045 for the
//! messages that the literals carry.

mod common;

use common::{run, scenarios, scenarios_with};
use fps_flow::{Scenario, Value};

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// The size that a line announces with a trailing `{n}`.
fn literal_len(line: &str) -> Option<usize> {
    let inner = line.strip_suffix('}')?;
    let (_, n) = inner.rsplit_once('{')?;
    n.parse().ok()
}

/// One command of the client, with the data that follows it.
#[derive(Debug)]
struct Cmd {
    tag: String,
    /// The command name in upper case.
    name: String,
    /// The command line without its CRLF.
    line: String,
    /// The literal that the command announces, which the client sends after
    /// the continuation request.
    literal: Option<Vec<u8>>,
    /// The line that follows AUTHENTICATE or IDLE.
    extra: Option<String>,
}

fn parse_client(mut rest: &[u8]) -> Vec<Cmd> {
    let mut cmds: Vec<Cmd> = Vec::new();
    while !rest.is_empty() {
        let end = find(rest, b"\r\n").expect("every line ends with CRLF");
        let line = String::from_utf8(rest[..end].to_vec()).expect("commands are ASCII");
        rest = &rest[end + 2..];
        let (tag, after) = line.split_once(' ').expect("a command has a tag");
        let expected = format!("A{:03}", cmds.len() + 1);
        assert_eq!(tag, expected, "tags count up from A001: {line}");
        let name = after.split(' ').next().unwrap().to_ascii_uppercase();
        let mut cmd = Cmd {
            tag: tag.to_string(),
            name,
            line: line.clone(),
            literal: None,
            extra: None,
        };
        if let Some(n) = literal_len(&line) {
            assert!(rest.len() >= n + 2, "the literal is complete");
            cmd.literal = Some(rest[..n].to_vec());
            assert_eq!(&rest[n..n + 2], b"\r\n", "CRLF ends the command after the literal");
            rest = &rest[n + 2..];
        }
        if cmd.name == "AUTHENTICATE" || cmd.name == "IDLE" {
            let end = find(rest, b"\r\n").expect("a line follows");
            cmd.extra = Some(String::from_utf8(rest[..end].to_vec()).unwrap());
            rest = &rest[end + 2..];
        }
        cmds.push(cmd);
    }
    cmds
}

#[derive(Debug, PartialEq)]
enum Kind {
    Untagged,
    Tagged,
    Continuation,
}

/// One response of the server. A literal stays in `text` as its `{n}` marker
/// and its bytes go to `literals`.
#[derive(Debug)]
struct Resp {
    kind: Kind,
    text: String,
    literals: Vec<Vec<u8>>,
}

fn parse_server(mut rest: &[u8]) -> Vec<Resp> {
    let mut out = Vec::new();
    while !rest.is_empty() {
        let mut text = String::new();
        let mut literals = Vec::new();
        loop {
            let end = find(rest, b"\r\n").expect("every line ends with CRLF");
            text.push_str(std::str::from_utf8(&rest[..end]).expect("responses are ASCII outside literals"));
            rest = &rest[end + 2..];
            match literal_len(&text) {
                Some(n) if text.starts_with("* ") => {
                    assert!(rest.len() >= n, "the literal is complete");
                    literals.push(rest[..n].to_vec());
                    rest = &rest[n..];
                }
                _ => break,
            }
        }
        let kind = if text.starts_with("* ") {
            Kind::Untagged
        } else if text.starts_with('+') {
            Kind::Continuation
        } else {
            Kind::Tagged
        };
        out.push(Resp {
            kind,
            text,
            literals,
        });
    }
    out
}

/// A command with the responses that answer it.
#[derive(Debug)]
struct Exchange<'a> {
    cmd: &'a Cmd,
    untagged: Vec<&'a Resp>,
    continuations: usize,
    tagged: &'a Resp,
}

/// Pairs each command with its responses. Every command ends in exactly one
/// tagged response with the tag of the command.
fn pair<'a>(cmds: &'a [Cmd], resps: &'a [Resp]) -> Vec<Exchange<'a>> {
    assert!(
        resps[0].text.starts_with("* OK [CAPABILITY IMAP4rev1 "),
        "greeting: {}",
        resps[0].text
    );
    let mut i = 1;
    let mut out = Vec::new();
    for cmd in cmds {
        let mut untagged = Vec::new();
        let mut continuations = 0;
        loop {
            let r = &resps[i];
            i += 1;
            match r.kind {
                Kind::Untagged => untagged.push(r),
                Kind::Continuation => continuations += 1,
                Kind::Tagged => {
                    let (tag, rest) = r.text.split_once(' ').unwrap();
                    assert_eq!(tag, cmd.tag, "the tagged response names the tag of its command");
                    assert!(rest.starts_with("OK "), "{}", r.text);
                    out.push(Exchange {
                        cmd,
                        untagged,
                        continuations,
                        tagged: r,
                    });
                    break;
                }
            }
        }
    }
    assert_eq!(i, resps.len(), "the server sent a response without a command");
    out
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
        assert!(line.len() <= 76, "a base64 line of {} characters", line.len());
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
    for h in ["From", "To", "Subject", "Date", "Message-ID", "MIME-Version"] {
        assert!(m.header(h).is_some(), "{name}: no {h} header");
    }
    assert!(m.header("From").unwrap().ends_with("@example.test"), "{name}");
    assert!(m.header("Date").unwrap().contains(" 2026 "), "{name}");
    assert!(m.text.contains("Ref: "), "{name}: the text part comes from content.email");
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

fn names<'a>(ex: &[Exchange<'a>]) -> Vec<&'a str> {
    ex.iter().map(|e| e.cmd.name.as_str()).collect()
}

fn flags_of(text: &str) -> Vec<&str> {
    let inner = text
        .split_once("FLAGS (")
        .expect("a FLAGS list")
        .1
        .split_once(')')
        .unwrap()
        .0;
    inner.split_whitespace().collect()
}

/// `* n SEARCH ...` numbers of an exchange.
fn search_numbers(e: &Exchange<'_>) -> Vec<usize> {
    assert_eq!(e.untagged.len(), 1, "{:?}", e.untagged);
    let line = e.untagged[0].text.strip_prefix("* SEARCH").expect("SEARCH");
    line.split_whitespace().map(|n| n.parse().unwrap()).collect()
}

fn check_login(e: &Exchange<'_>) {
    assert!(e.tagged.text.ends_with(" Logged in"), "{}", e.tagged.text);
    assert!(e.tagged.text.contains(" OK [CAPABILITY IMAP4rev1 "), "{}", e.tagged.text);
    let (user, secret) = match e.cmd.name.as_str() {
        "LOGIN" => {
            let user = e.cmd.line.split('"').nth(1).unwrap().to_string();
            let secret = match &e.cmd.literal {
                Some(l) => {
                    assert_eq!(e.continuations, 1);
                    String::from_utf8(l.clone()).unwrap()
                }
                None => e.cmd.line.split('"').nth(3).unwrap().to_string(),
            };
            (user, secret)
        }
        "AUTHENTICATE" => {
            assert_eq!(e.cmd.line.split(' ').nth(2), Some("PLAIN"));
            assert_eq!(e.continuations, 1, "the server asks for the answer once");
            let token = unbase64(e.cmd.extra.as_ref().unwrap());
            let parts: Vec<&[u8]> = token.split(|&b| b == 0).collect();
            assert_eq!(parts.len(), 3, "authzid NUL user NUL password");
            assert!(parts[0].is_empty());
            (
                String::from_utf8(parts[1].to_vec()).unwrap(),
                String::from_utf8(parts[2].to_vec()).unwrap(),
            )
        }
        other => panic!("{other}"),
    };
    assert!(user.ends_with("@example.test"), "{user}");
    assert!(secret.starts_with("Tst-"), "{secret}");
}

fn check_select(e: &Exchange<'_>, messages: usize, read_only: bool) {
    let lines: Vec<&str> = e.untagged.iter().map(|r| r.text.as_str()).collect();
    assert!(lines[0].starts_with("* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)"));
    assert!(lines.contains(&format!("* {messages} EXISTS").as_str()), "{lines:?}");
    assert!(lines.contains(&"* 0 RECENT"));
    assert!(lines.iter().any(|l| l.starts_with("* OK [UIDVALIDITY ")));
    assert!(
        lines.contains(&format!("* OK [UIDNEXT {}] Predicted next UID", 1001 + messages).as_str()),
        "{lines:?}"
    );
    assert_eq!(
        lines.iter().any(|l| l.contains("[PERMANENTFLAGS ()]")),
        read_only
    );
    let code = if read_only { "[READ-ONLY]" } else { "[READ-WRITE]" };
    assert!(e.tagged.text.contains(code), "{}", e.tagged.text);
}

/// The UID FETCH data of one message: `* n FETCH (UID 1000+n ...`.
fn check_fetch_head(e: &Exchange<'_>, seq: usize) -> String {
    assert_eq!(e.untagged.len(), 1, "{:?}", e.untagged);
    let r = e.untagged[0];
    let head = format!("* {seq} FETCH (UID {} ", 1000 + seq);
    assert!(r.text.starts_with(&head), "{} vs {head}", r.text);
    assert!(r.text.ends_with(')'), "{}", r.text);
    r.text.clone()
}

fn check_header_literal(text: &str, literal: &[u8], allowed: &[&str]) {
    let raw = std::str::from_utf8(literal).unwrap();
    assert!(raw.ends_with("\r\n\r\n"), "the header block ends with an empty line");
    assert_eq!(raw.matches("\r\n\r\n").count(), 1);
    let (headers, _) = split_head(raw);
    for (n, _) in &headers {
        assert!(
            allowed.iter().any(|a| a.eq_ignore_ascii_case(n)),
            "{n} was not asked for: {text}"
        );
    }
}

/// Runs one session of a scenario and checks the dialogue. It returns the
/// sizes of the attachments of the messages that arrived whole.
async fn check(name: &str, s: &Scenario, session_id: u64) -> Vec<usize> {
    let r = run(s, 42, session_id).await;
    assert!(r.client.is_ok(), "{name} client: {:?}", r.client);
    assert!(r.server.is_ok(), "{name} server: {:?}", r.server);
    assert_eq!(r.client_stats.bytes_tx, r.server_stats.bytes_rx, "{name}");
    assert_eq!(r.server_stats.bytes_tx, r.client_stats.bytes_rx, "{name}");

    let cmds = parse_client(&r.c2s);
    let resps = parse_server(&r.s2c);
    let ex = pair(&cmds, &resps);
    let messages = var(s, "messages");
    let mut sizes = Vec::new();

    // Every session starts with CAPABILITY and ends with LOGOUT, and the
    // greeting and the CAPABILITY response advertise the same protocol.
    assert_eq!(ex[0].cmd.name, "CAPABILITY", "{name}");
    assert!(ex[0].untagged[0].text.starts_with("* CAPABILITY IMAP4rev1 "), "{name}");
    assert_eq!(ex.last().unwrap().cmd.line.split(' ').nth(1), Some("LOGOUT"), "{name}");
    assert_eq!(ex.last().unwrap().untagged[0].text, "* BYE Logging out", "{name}");
    // Only the commands that need one get a continuation request.
    for e in &ex {
        let wants = usize::from(e.cmd.literal.is_some() || e.cmd.name == "AUTHENTICATE" || e.cmd.name == "IDLE");
        assert_eq!(e.continuations, wants, "{name}: {}", e.cmd.line);
    }

    match name {
        "mailbox_sync" => {
            let mut want = vec!["CAPABILITY", "LOGIN", "SELECT", "FETCH"];
            want.extend(std::iter::repeat("FETCH").take(messages));
            want.push("LOGOUT");
            assert_eq!(names(&ex), want);
            check_login(&ex[1]);
            check_select(&ex[2], messages, false);
            // The listing names every message with its UID, flags, and size.
            assert_eq!(ex[3].cmd.line, format!("A004 FETCH 1:{messages} (UID FLAGS RFC822.SIZE)"));
            assert_eq!(ex[3].untagged.len(), messages);
            for (i, r) in ex[3].untagged.iter().enumerate() {
                let n = i + 1;
                let head = format!("* {n} FETCH (UID {} FLAGS (", 1000 + n);
                assert!(r.text.starts_with(&head), "{}", r.text);
                assert!(r.text.contains(") RFC822.SIZE "), "{}", r.text);
            }
            // One header fetch per message, in order, with a literal each.
            for n in 1..=messages {
                let e = &ex[3 + n];
                assert_eq!(
                    e.cmd.line,
                    format!("A{:03} FETCH {n} (UID FLAGS BODY.PEEK[HEADER.FIELDS (FROM TO CC SUBJECT DATE MESSAGE-ID)])", 4 + n)
                );
                let text = check_fetch_head(e, n);
                assert!(text.contains(" BODY[HEADER.FIELDS (FROM TO CC SUBJECT DATE MESSAGE-ID)] {"), "{text}");
                let lit = &e.untagged[0].literals;
                assert_eq!(lit.len(), 1);
                check_header_literal(&text, &lit[0], &["From", "To", "Cc", "Subject", "Date", "Message-ID"]);
                let raw = std::str::from_utf8(&lit[0]).unwrap();
                assert!(raw.starts_with("From: ") && raw.contains("\r\nSubject: "), "{raw}");
            }
        }
        "fetch_messages" => {
            let fetches = var(s, "fetches");
            let mut want = vec!["CAPABILITY", "LOGIN", "EXAMINE"];
            want.extend(std::iter::repeat("FETCH").take(fetches));
            want.push("LOGOUT");
            assert_eq!(names(&ex), want);
            check_login(&ex[1]);
            check_select(&ex[2], messages, true);
            let kinds: [(&[u8], &str, &str); 3] = [
                (b"%PDF-", "statement.pdf", "application/pdf"),
                (b"\xff\xd8\xff", "photo.jpg", "image/jpeg"),
                (b"PK", "archive.zip", "application/zip"),
            ];
            for e in &ex[3..3 + fetches] {
                let seq: usize = e.cmd.line.split(' ').nth(2).unwrap().parse().unwrap();
                assert!((1..=messages).contains(&seq), "{}", e.cmd.line);
                assert!(e.cmd.line.ends_with(" (UID FLAGS BODY.PEEK[])"), "{}", e.cmd.line);
                let text = check_fetch_head(e, seq);
                let lit = &e.untagged[0].literals;
                assert_eq!(lit.len(), 1);
                // A peek leaves the flags alone, so the response ends with
                // the literal.
                assert!(text.contains(&format!(" BODY[] {{{}}}", lit[0].len())), "{text}");
                assert!(text.ends_with(&format!("{{{}}})", lit[0].len())), "{text}");
                sizes.push(check_message(name, &lit[0], &kinds));
            }
        }
        "search_store" => {
            assert_eq!(
                names(&ex),
                ["CAPABILITY", "LOGIN", "SELECT", "SEARCH", "FETCH", "STORE", "SEARCH", "STORE", "EXPUNGE", "CLOSE", "LOGOUT"]
            );
            check_login(&ex[1]);
            check_select(&ex[2], messages, false);
            assert_eq!(ex[3].cmd.line, "A004 SEARCH UNSEEN");
            let hits = search_numbers(&ex[3]);
            assert_eq!(hits.len(), var(s, "hits"));
            assert!(hits.windows(2).all(|w| w[0] < w[1]));
            assert!(hits.iter().all(|h| (1..=messages).contains(h)));
            // The client fetches and flags the first hit.
            let h = hits[0];
            assert_eq!(ex[4].cmd.line, format!("A005 FETCH {h} (UID FLAGS BODY[])"));
            let text = check_fetch_head(&ex[4], h);
            // A fetch that is not a peek reports the flag that it sets.
            assert!(flags_of(&text).contains(&"\\Seen"), "{text}");
            let lit = &ex[4].untagged[0].literals;
            sizes.push(check_message(name, &lit[0], &[(b"\x1f\x8b", "digest.csv.gz", "application/gzip")]));
            assert_eq!(ex[5].cmd.line, format!("A006 STORE {h} +FLAGS (\\Seen \\Flagged)"));
            assert_eq!(ex[5].untagged.len(), 1);
            assert!(ex[5].untagged[0].text.starts_with(&format!("* {h} FETCH (FLAGS (")));
            let f = flags_of(&ex[5].untagged[0].text);
            assert!(f.contains(&"\\Seen") && f.contains(&"\\Flagged"), "{f:?}");
            // The second search names the message that the client deletes.
            assert!(ex[6].cmd.line.starts_with("A007 SEARCH FROM \""));
            assert!(ex[6].cmd.line.ends_with("@example.test\" SINCE 1-Jan-2026"));
            let junk = search_numbers(&ex[6])[0];
            assert_eq!(ex[7].cmd.line, format!("A008 STORE {junk} +FLAGS (\\Deleted)"));
            assert!(flags_of(&ex[7].untagged[0].text).contains(&"\\Deleted"));
            assert_eq!(ex[8].untagged.len(), 1);
            assert_eq!(ex[8].untagged[0].text, format!("* {junk} EXPUNGE"));
        }
        "authenticate_plain" => {
            assert_eq!(
                names(&ex),
                ["CAPABILITY", "AUTHENTICATE", "STATUS", "SELECT", "FETCH", "NOOP", "LOGOUT"]
            );
            check_login(&ex[1]);
            assert_eq!(
                ex[2].untagged[0].text,
                format!("* STATUS \"INBOX\" (MESSAGES {messages} UNSEEN 1 UIDNEXT {})", 1001 + messages)
            );
            check_select(&ex[3], messages, false);
            assert_eq!(ex[4].cmd.line, format!("A005 FETCH {messages} (UID FLAGS BODY[HEADER])"));
            let text = check_fetch_head(&ex[4], messages);
            assert!(flags_of(&text).contains(&"\\Seen"), "{text}");
            let lit = &ex[4].untagged[0].literals;
            let raw = std::str::from_utf8(&lit[0]).unwrap();
            assert!(raw.ends_with("\r\n\r\n") && raw.contains("MIME-Version: 1.0\r\n"), "{raw}");
            assert!(raw.contains("Content-Type: multipart/mixed; boundary="), "{raw}");
            assert!(ex[5].tagged.text.contains("NOOP completed"));
        }
        "login_append" => {
            assert_eq!(names(&ex), ["CAPABILITY", "LOGIN", "STATUS", "APPEND", "LOGOUT"]);
            assert!(ex[1].cmd.literal.is_some(), "the password is a literal");
            check_login(&ex[1]);
            assert_eq!(
                ex[2].untagged[0].text,
                format!("* STATUS \"Sent\" (MESSAGES {messages} UIDNEXT {})", 1001 + messages)
            );
            let a = &ex[3];
            assert!(a.cmd.line.starts_with("A004 APPEND \"Sent\" (\\Seen) {"), "{}", a.cmd.line);
            let lit = a.cmd.literal.as_ref().unwrap();
            assert!(lit.ends_with(b"\r\n"));
            sizes.push(check_message(name, lit, &[(b"%PDF-", "proposal.pdf", "application/pdf")]));
            let m = parse_mail(lit);
            assert!(m.header("Cc").is_some());
            assert!(
                a.tagged.text.contains(&format!(" OK [APPENDUID ")) && a.tagged.text.contains(&format!(" {}] Append completed", 1001 + messages)),
                "{}",
                a.tagged.text
            );
        }
        "idle_push" => {
            assert_eq!(names(&ex), ["CAPABILITY", "LOGIN", "SELECT", "IDLE", "FETCH", "LOGOUT"]);
            check_login(&ex[1]);
            check_select(&ex[2], messages, false);
            assert_eq!(ex[3].cmd.extra.as_deref(), Some("DONE"));
            let pushed: Vec<&str> = ex[3].untagged.iter().map(|r| r.text.as_str()).collect();
            assert_eq!(pushed, [format!("* {} EXISTS", messages + 1), "* 1 RECENT".to_string()]);
            assert!(ex[3].tagged.text.contains("Idle completed"));
            let new = messages + 1;
            assert_eq!(ex[4].cmd.line, format!("A005 FETCH {new} (UID FLAGS BODY.PEEK[])"));
            check_fetch_head(&ex[4], new);
            sizes.push(check_message(name, &ex[4].untagged[0].literals[0], &[(b"\xff\xd8\xff", "scan.jpg", "image/jpeg")]));
        }
        other => panic!("no expectation for scenario {other}"),
    }
    sizes
}

#[test]
fn the_manifest_lists_six_scenarios() {
    let names: Vec<String> = scenarios().into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        [
            "mailbox_sync",
            "fetch_messages",
            "search_store",
            "authenticate_plain",
            "login_append",
            "idle_push"
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
async fn the_variables_change_the_number_of_messages_and_fetches() {
    let small = [
        ("messages", Value::Int(2)),
        ("fetches", Value::Int(1)),
        ("hits", Value::Int(1)),
        ("attachment_min", Value::Int(500)),
        ("attachment_max", Value::Int(900)),
    ];
    for (name, s) in scenarios_with(&small) {
        let r = run(&s, 42, 1).await;
        assert!(r.client.is_ok() && r.server.is_ok(), "{name}");
        let cmds = parse_client(&r.c2s);
        let fetches = cmds.iter().filter(|c| c.name == "FETCH").count();
        let want = match name.as_str() {
            "mailbox_sync" => 1 + 2,
            "login_append" => 0,
            _ => 1,
        };
        assert_eq!(fetches, want, "{name}");
        check(&name, &s, 2).await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_one_message_mailbox_still_runs() {
    let one = [("messages", Value::Int(1)), ("hits", Value::Int(1))];
    for (name, s) in scenarios_with(&one) {
        check(&name, &s, 3).await;
    }
}

#[tokio::test(start_paused = true)]
async fn the_greeting_and_the_capability_list_look_like_a_dovecot_server() {
    for (name, s) in scenarios() {
        let r = run(&s, 42, 1).await;
        let text = String::from_utf8_lossy(&r.s2c).into_owned();
        let lines: Vec<&str> = text.split("\r\n").take(3).collect();
        assert!(lines[0].starts_with("* OK [CAPABILITY IMAP4rev1 "), "{name}");
        assert!(lines[0].ends_with("imap.example.test Dovecot ready."), "{name}");
        assert!(lines[1].starts_with("* CAPABILITY IMAP4rev1 "), "{name}");
        assert!(lines[2].starts_with("A001 OK Pre-login capabilities listed"), "{name}");
        // Plain IMAP: no STARTTLS and no disabled LOGIN.
        assert!(!lines[1].contains("STARTTLS") && !lines[1].contains("LOGINDISABLED"), "{name}");
    }
}
