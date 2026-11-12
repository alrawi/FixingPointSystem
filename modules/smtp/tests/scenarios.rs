// SPDX-License-Identifier: GPL-3.0-only
//! Interpreter tests: every smtp scenario runs with its client and server
//! against each other over the in memory duplex, and the bytes in each
//! direction parse as the SMTP dialogue that the scenario describes. The
//! parser below follows RFC 5321 for commands, replies, and the transparent
//! form of DATA, and RFC 2045 for the base64 attachment.

mod common;

use common::{in_one_write, run, scenarios, scenarios_with};
use fps_flow::Value;

/// One step of the client stream.
#[derive(Debug, PartialEq)]
enum Step {
    Command(String),
    /// A line that answers a 334 challenge, which is base64 text.
    AuthAnswer(String),
    /// The message of a DATA command after the removal of dot stuffing.
    Data(Vec<u8>),
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Splits the stream that the client wrote into commands, challenge answers,
/// and messages. `challenges` is the number of 334 answers that follow
/// `AUTH LOGIN`.
fn parse_client(mut rest: &[u8]) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut answers = 0;
    let mut in_data = false;
    while !rest.is_empty() {
        if in_data {
            let end = find(rest, b"\r\n.\r\n").expect("the data ends with a dot line");
            let raw = &rest[..end + 2];
            let mut message = Vec::new();
            for line in raw.split_inclusive(|&b| b == b'\n') {
                assert!(line.ends_with(b"\r\n"), "a bare line feed in the data");
                assert!(line.len() <= 1000, "a line longer than RFC 5321 allows");
                assert_ne!(line, b".\r\n", "an unstuffed dot line inside the data");
                message.extend_from_slice(line.strip_prefix(b".").unwrap_or(line));
            }
            steps.push(Step::Data(message));
            rest = &rest[end + 5..];
            in_data = false;
            continue;
        }
        let end = find(rest, b"\r\n").expect("every command ends with CRLF");
        let line = String::from_utf8(rest[..end].to_vec()).expect("commands are ASCII");
        rest = &rest[end + 2..];
        if answers > 0 {
            answers -= 1;
            steps.push(Step::AuthAnswer(line));
            continue;
        }
        if line == "AUTH LOGIN" {
            answers = 2;
        }
        in_data = line == "DATA";
        steps.push(Step::Command(line));
    }
    steps
}

/// Splits the stream that the server wrote into reply codes, with the number
/// of lines of each reply.
fn parse_server(data: &[u8]) -> Vec<(u16, usize)> {
    let text = std::str::from_utf8(data).expect("replies are ASCII");
    let mut replies = Vec::new();
    let mut open: Option<(u16, usize)> = None;
    for line in text.split_terminator("\r\n") {
        let code: u16 = line[..3].parse().expect("a reply starts with a code");
        let sep = line.as_bytes()[3];
        assert!(sep == b'-' || sep == b' ', "{line}");
        let lines = match open {
            Some((c, n)) => {
                assert_eq!(c, code, "a continuation line changed the code");
                n + 1
            }
            None => 1,
        };
        open = if sep == b'-' {
            Some((code, lines))
        } else {
            None
        };
        if open.is_none() {
            replies.push((code, lines));
        }
    }
    assert!(open.is_none(), "the last reply is not finished");
    replies
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

/// What one scenario puts on the wire.
struct Expect {
    auth: Option<&'static str>,
    messages: usize,
    recipients: usize,
    /// Magic bytes of the attachment, its file name, and its content type.
    attachment: Option<(&'static [u8], &'static str, &'static str)>,
    pipelined: bool,
}

fn expected(name: &str) -> Expect {
    match name {
        "relay" => Expect {
            auth: None,
            messages: 1,
            recipients: 3,
            attachment: Some((b"\x89PNG\r\n\x1a\n", "scan.png", "image/png")),
            pipelined: false,
        },
        "relay_pipelined" => Expect {
            auth: None,
            messages: 1,
            recipients: 4,
            attachment: Some((b"%PDF-", "invoice.pdf", "application/pdf")),
            pipelined: true,
        },
        "text_only" => Expect {
            auth: None,
            messages: 1,
            recipients: 1,
            attachment: None,
            pipelined: false,
        },
        "submission_plain" => Expect {
            auth: Some("PLAIN"),
            messages: 1,
            recipients: 2,
            attachment: Some((b"PK", "archive.zip", "application/zip")),
            pipelined: false,
        },
        "submission_login" => Expect {
            auth: Some("LOGIN"),
            messages: 1,
            recipients: 2,
            attachment: Some((b"\xff\xd8\xff", "photo.jpg", "image/jpeg")),
            pipelined: false,
        },
        "submission_batch" => Expect {
            auth: Some("PLAIN"),
            messages: 3,
            recipients: 3,
            attachment: Some((b"\x1f\x8b", "export.csv.gz", "application/gzip")),
            pipelined: true,
        },
        other => panic!("no expectation for scenario {other}"),
    }
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
    let Some((_, boundary)) = ctype
        .strip_prefix("multipart/mixed; boundary=\"")
        .map(|b| ("", b.trim_end_matches('"').to_string()))
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
    let disposition = get("Content-Disposition");
    let name = disposition
        .strip_prefix("attachment; filename=\"")
        .and_then(|d| d.strip_suffix('"'))
        .expect("a disposition with a file name")
        .to_string();
    let ct = get("Content-Type");
    let ct = ct.split(';').next().unwrap().to_string();
    mail(text.to_string(), Some((name, ct, unbase64(encoded))))
}

fn addresses(list: &str) -> Vec<String> {
    list.split(',').map(|a| a.trim().to_string()).collect()
}

/// Runs one session of a scenario and checks the dialogue against `e`. It
/// returns the sizes of the decoded attachments.
async fn check(name: &str, s: &fps_flow::Scenario, session_id: u64) -> Vec<usize> {
    let e = expected(name);
    let r = run(s, 42, session_id).await;
    assert!(r.client.is_ok(), "{name} client: {:?}", r.client);
    assert!(r.server.is_ok(), "{name} server: {:?}", r.server);
    assert_eq!(r.client_stats.bytes_tx, r.server_stats.bytes_rx, "{name}");
    assert_eq!(r.server_stats.bytes_tx, r.client_stats.bytes_rx, "{name}");

    let steps = parse_client(&r.c2s);
    let replies = parse_server(&r.s2c);

    // The command sequence.
    let cmds: Vec<&str> = steps
        .iter()
        .filter_map(|s| match s {
            Step::Command(c) => Some(c.as_str()),
            _ => None,
        })
        .collect();
    assert!(cmds[0].starts_with("EHLO "), "{name}: {}", cmds[0]);
    assert_eq!(*cmds.last().unwrap(), "QUIT", "{name}");
    let count = |p: &str| cmds.iter().filter(|c| c.starts_with(p)).count();
    assert_eq!(count("MAIL FROM:<"), e.messages, "{name}");
    assert_eq!(count("RCPT TO:<"), e.messages * e.recipients, "{name}");
    assert_eq!(count("DATA"), e.messages, "{name}");
    assert_eq!(count("AUTH "), usize::from(e.auth.is_some()), "{name}");

    // The replies follow the commands one to one, and the EHLO reply has
    // several lines.
    let mut want: Vec<u16> = vec![220];
    for st in &steps {
        want.push(match st {
            Step::Command(c) if c.starts_with("EHLO") => 250,
            Step::Command(c) if c == "AUTH PLAIN" || c.starts_with("AUTH PLAIN ") => 235,
            Step::Command(c) if c == "AUTH LOGIN" => 334,
            Step::AuthAnswer(_) => {
                // The first answer meets 334, and the second meets 235.
                let n = want.iter().filter(|&&c| c == 334).count();
                if n % 2 == 1 {
                    334
                } else {
                    235
                }
            }
            Step::Command(c) if c == "DATA" => 354,
            Step::Command(c) if c == "QUIT" => 221,
            Step::Command(_) | Step::Data(_) => 250,
        });
    }
    let got: Vec<u16> = replies.iter().map(|r| r.0).collect();
    assert_eq!(got, want, "{name}");
    assert!(
        replies[1].1 >= 5,
        "{name}: the EHLO reply has {} lines",
        replies[1].1
    );
    assert!(
        replies[2..]
            .iter()
            .all(|r| r.1 == 1 || r.0 == 334 && r.1 == 1),
        "{name}"
    );

    // Pipelining: the commands of a batch arrive without a reply in between,
    // so a pipelined scenario writes MAIL, RCPT, and DATA as one block.
    let from = find(&r.c2s, b"MAIL FROM:<").unwrap();
    let to = from + find(&r.c2s[from..], b"DATA\r\n").unwrap() + 6;
    assert_eq!(
        in_one_write(&r.c2s_ends, from, to),
        e.pipelined,
        "{name}: MAIL FROM, RCPT TO, and DATA share a write only when pipelined"
    );
    if e.pipelined {
        // The server answers the batch in one write as well, from the reply to
        // MAIL FROM up to the 354 line.
        let text = &r.s2c;
        let first = find(text, b"250 2.1.0 Ok\r\n").unwrap();
        let last = first + find(&text[first..], b"354 ").unwrap() + 4;
        assert!(in_one_write(&r.s2c_ends, first, last), "{name}");
    }

    // The authentication.
    let answers: Vec<&String> = steps
        .iter()
        .filter_map(|s| match s {
            Step::AuthAnswer(a) => Some(a),
            _ => None,
        })
        .collect();
    let (user, password) = match e.auth {
        None => (None, None),
        Some("PLAIN") => {
            let line = cmds.iter().find(|c| c.starts_with("AUTH PLAIN ")).unwrap();
            let token = unbase64(line.strip_prefix("AUTH PLAIN ").unwrap());
            let fields: Vec<&[u8]> = token.split(|&b| b == 0).collect();
            assert_eq!(fields.len(), 3, "{name}");
            assert!(fields[0].is_empty(), "{name}: no authorization identity");
            (
                Some(String::from_utf8(fields[1].to_vec()).unwrap()),
                Some(String::from_utf8(fields[2].to_vec()).unwrap()),
            )
        }
        Some(_) => {
            assert_eq!(answers.len(), 2, "{name}");
            (
                Some(String::from_utf8(unbase64(answers[0])).unwrap()),
                Some(String::from_utf8(unbase64(answers[1])).unwrap()),
            )
        }
    };
    if let (Some(u), Some(p)) = (&user, &password) {
        assert!(u.ends_with("@example.test"), "{name}: {u}");
        assert!(p.starts_with("Tst-"), "{name}: {p}");
        for c in &cmds {
            if let Some(from) = c.strip_prefix("MAIL FROM:<") {
                assert_eq!(from, format!("{u}>"), "{name}");
            }
        }
    }

    // The messages.
    let rcpts: Vec<String> = cmds
        .iter()
        .filter_map(|c| c.strip_prefix("RCPT TO:<"))
        .map(|a| a.trim_end_matches('>').to_string())
        .collect();
    let mails: Vec<Mail> = steps
        .iter()
        .filter_map(|s| match s {
            Step::Data(d) => Some(parse_mail(d)),
            _ => None,
        })
        .collect();
    assert_eq!(mails.len(), e.messages, "{name}");
    let mut sizes = Vec::new();
    for (i, m) in mails.iter().enumerate() {
        let mut listed = addresses(m.header("To").unwrap());
        if let Some(cc) = m.header("Cc") {
            listed.extend(addresses(cc));
        }
        let mut sent: Vec<String> = rcpts[i * e.recipients..(i + 1) * e.recipients].to_vec();
        listed.sort();
        sent.sort();
        assert_eq!(
            listed, sent,
            "{name}: message {i} headers and RCPT TO differ"
        );
        assert!(m.header("From").unwrap().ends_with("@example.test"));
        assert!(m.header("Message-ID").unwrap().ends_with("@example.test>"));
        assert!(m.header("Date").unwrap().contains(" 2026 "));
        assert_eq!(m.header("MIME-Version"), Some("1.0"));
        assert!(
            m.text.contains("Ref: "),
            "{name}: the text part comes from content.email"
        );
        match (&e.attachment, &m.attachment) {
            (None, None) => {}
            (Some((magic, file, ctype)), Some((f, c, bytes))) => {
                assert_eq!(f, file, "{name}");
                assert_eq!(c, ctype, "{name}");
                assert!(bytes.starts_with(magic), "{name}: {:02x?}", &bytes[..8]);
                sizes.push(bytes.len());
            }
            (want, got) => panic!("{name}: attachment {got:?} but expected {want:?}"),
        }
        // The dot stuffing round trip: scenarios that add dot lines to the
        // text part get them back unchanged.
        if name != "text_only" && name != "submission_login" {
            assert!(
                m.text
                    .contains("\r\n.\r\nThe line above holds a single dot.\r\n"),
                "{name}: {:?}",
                m.text
            );
        }
    }
    sizes
}

#[test]
fn the_manifest_lists_six_scenarios() {
    let names: Vec<String> = scenarios().into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        [
            "relay",
            "relay_pipelined",
            "text_only",
            "submission_plain",
            "submission_login",
            "submission_batch"
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
        let e = expected(&name);
        if e.attachment.is_none() {
            continue;
        }
        let mut all = Vec::new();
        for session_id in 1..=8 {
            all.extend(check(&name, &s, session_id).await);
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
async fn the_variables_change_the_number_of_recipients_and_messages() {
    let small = [
        ("recipients", Value::Int(1)),
        ("messages", Value::Int(2)),
        ("attachment_min", Value::Int(500)),
        ("attachment_max", Value::Int(900)),
    ];
    for (name, s) in scenarios_with(&small) {
        if name != "relay_pipelined" && name != "submission_batch" {
            continue;
        }
        let r = run(&s, 42, 1).await;
        assert!(r.client.is_ok() && r.server.is_ok(), "{name}");
        let steps = parse_client(&r.c2s);
        let rcpt = steps
            .iter()
            .filter(|s| matches!(s, Step::Command(c) if c.starts_with("RCPT TO:<")))
            .count();
        let data = steps.iter().filter(|s| matches!(s, Step::Data(_))).count();
        let messages = if name == "submission_batch" { 2 } else { 1 };
        assert_eq!((rcpt, data), (messages, messages), "{name}");
    }
}

#[tokio::test(start_paused = true)]
async fn the_server_replies_with_a_realistic_multi_line_ehlo() {
    for (name, s) in scenarios() {
        let r = run(&s, 42, 1).await;
        let text = String::from_utf8(r.s2c).unwrap();
        let lines: Vec<&str> = text.split("\r\n").skip(1).collect();
        let end = lines.iter().position(|l| l.starts_with("250 ")).unwrap();
        let ehlo = &lines[..=end];
        assert!(ehlo.len() >= 6, "{name}");
        assert!(ehlo[0].starts_with("250-"), "{name}");
        assert!(ehlo.iter().any(|l| l.contains("PIPELINING")), "{name}");
        assert_eq!(
            ehlo.iter().any(|l| l.contains("AUTH ")),
            name.starts_with("submission"),
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
    let all = common::all_scenarios_with(&[]);
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
