// SPDX-License-Identifier: GPL-3.0-only
//! Interpreter tests of the ftp scenarios. Every scenario runs as a scenario
//! instance, both sides of the control flow and of every spawned data flow
//! against each other, and the bytes of each connection are parsed with a
//! model of RFC 959, RFC 2428, and RFC 3659.
//!
//! The tests check the session structure first, which is the acceptance of
//! the module: one control connection per scenario and exactly one data
//! connection for each LIST, RETR, and STOR, opened to the port that the 227
//! or 229 reply announced.

mod common;

use common::{run, scenarios, scenarios_with, Conn, Run, SEED};
use fps_flow::Value;
use fps_interp::FlowState;

/// One reply of the server, which may span several lines.
#[derive(Clone, Debug)]
struct Reply {
    code: u16,
    lines: Vec<String>,
}

fn text_lines(bytes: &[u8]) -> Vec<String> {
    let text = std::str::from_utf8(bytes).expect("control bytes are text");
    assert!(
        text.is_empty() || text.ends_with("\r\n"),
        "a line without CRLF at the end"
    );
    let body = text.strip_suffix("\r\n").unwrap_or("");
    if text.is_empty() {
        return Vec::new();
    }
    body.split("\r\n")
        .map(|l| {
            assert!(!l.contains('\r') && !l.contains('\n'), "stray line break");
            l.to_string()
        })
        .collect()
}

/// Splits the server bytes into replies, joining the lines of a multi line
/// reply as RFC 959 section 4.2 defines it: `NNN-` opens and `NNN ` closes.
fn replies(bytes: &[u8]) -> Vec<Reply> {
    let mut out = Vec::new();
    let mut open: Option<Reply> = None;
    for line in text_lines(bytes) {
        if let Some(r) = &mut open {
            // The reply ends with the code and a space. Every other line in
            // between, with or without the code, belongs to the reply.
            let closes = line.len() >= 4
                && line[..3].parse::<u16>().ok() == Some(r.code)
                && line.as_bytes()[3] == b' ';
            r.lines.push(line);
            if closes {
                out.extend(open.take());
            }
            continue;
        }
        let code: u16 = line[..3].parse().expect("reply code");
        let reply = Reply {
            code,
            lines: vec![line.clone()],
        };
        match line.as_bytes()[3] {
            b'-' => open = Some(reply),
            b' ' => out.push(reply),
            other => panic!("bad separator {other} in {line}"),
        }
    }
    assert!(open.is_none(), "a multi line reply that never ended");
    out
}

#[derive(Debug)]
struct Transfer {
    verb: String,
    name: Option<String>,
    mode: &'static str,
    port: u16,
    host: Option<String>,
    size: Option<usize>,
    preliminary: String,
}

/// The port of a 227 reply and the host that it names.
fn parse_227(line: &str) -> (String, u16) {
    let inner = &line[line.find('(').unwrap() + 1..line.rfind(')').unwrap()];
    let n: Vec<u16> = inner.split(',').map(|x| x.parse().unwrap()).collect();
    assert_eq!(n.len(), 6, "{line}");
    assert!(n.iter().all(|x| *x < 256), "{line}");
    (
        format!("{}.{}.{}.{}", n[0], n[1], n[2], n[3]),
        n[4] * 256 + n[5],
    )
}

fn parse_229(line: &str) -> u16 {
    let inner = &line[line.find('(').unwrap() + 1..line.rfind(')').unwrap()];
    let port = inner
        .strip_prefix("|||")
        .and_then(|r| r.strip_suffix('|'))
        .unwrap_or_else(|| panic!("{line}"));
    port.parse().unwrap()
}

/// The result of reading a control connection.
struct Control {
    commands: Vec<String>,
    transfers: Vec<Transfer>,
    logins: Vec<(String, String)>,
}

/// Walks the commands of the control connection with the replies of the
/// server and checks every reply code.
fn read_control(conn: &Conn) -> Control {
    let commands = text_lines(&conn.c2s());
    let replies = replies(&conn.s2c());
    assert_eq!(replies[0].code, 220, "greeting");
    assert!(replies[0].lines[0].starts_with("220 "));
    let mut next = 1;
    let mut take = |code: u16, cmd: &str| -> Reply {
        let r = replies
            .get(next)
            .unwrap_or_else(|| panic!("no reply to {cmd}"));
        assert_eq!(r.code, code, "reply to {cmd}: {r:?}");
        next += 1;
        r.clone()
    };
    let mut transfers = Vec::new();
    let mut logins = Vec::new();
    let mut pasv: Option<(&'static str, u16, Option<String>)> = None;
    let mut size: Option<usize> = None;
    let mut user = String::new();
    for cmd in &commands {
        let (verb, arg) = cmd
            .split_once(' ')
            .map_or((cmd.as_str(), None), |(v, a)| (v, Some(a)));
        match verb {
            "USER" => {
                take(331, cmd);
                user = arg.unwrap().to_string();
            }
            "PASS" => {
                take(230, cmd);
                logins.push((user.clone(), arg.unwrap().to_string()));
            }
            "SYST" => {
                let r = take(215, cmd);
                assert!(r.lines[0].starts_with("215 UNIX Type: L8"));
            }
            "FEAT" => {
                let r = take(211, cmd);
                assert!(r.lines.len() > 3, "{r:?}");
                assert_eq!(r.lines[0], "211-Features:");
                assert_eq!(r.lines.last().unwrap(), "211 End");
                assert!(r.lines.iter().any(|l| l == " EPSV"));
            }
            "PWD" => {
                let r = take(257, cmd);
                assert!(r.lines[0].starts_with("257 \"/"), "{r:?}");
            }
            "CWD" => {
                take(250, cmd);
            }
            "TYPE" => {
                take(200, cmd);
            }
            "PASV" => {
                let r = take(227, cmd);
                let (host, port) = parse_227(&r.lines[0]);
                pasv = Some(("PASV", port, Some(host)));
            }
            "EPSV" => {
                let r = take(229, cmd);
                pasv = Some(("EPSV", parse_229(&r.lines[0]), None));
            }
            "SIZE" => {
                let r = take(213, cmd);
                size = Some(r.lines[0][4..].parse().unwrap());
            }
            "ALLO" => {
                take(202, cmd);
                size = Some(arg.unwrap().parse().unwrap());
            }
            "LIST" | "RETR" | "STOR" => {
                let (mode, port, host) = pasv.take().expect("a transfer needs PASV or EPSV first");
                let pre = take(150, cmd).lines[0].clone();
                let done = take(226, cmd);
                assert!(done.lines[0].starts_with("226 "));
                transfers.push(Transfer {
                    verb: verb.to_string(),
                    name: arg.map(str::to_string),
                    mode,
                    port,
                    host,
                    size: if verb == "LIST" { None } else { size },
                    preliminary: pre,
                });
            }
            "QUIT" => {
                take(221, cmd);
            }
            other => panic!("the model does not know the command {other}"),
        }
    }
    assert_eq!(next, replies.len(), "replies without a command");
    assert_eq!(commands.last().map(String::as_str), Some("QUIT"));
    Control {
        commands,
        transfers,
        logins,
    }
}

fn magic_ok(name: &str, data: &[u8]) -> bool {
    match name.rsplit('.').next().unwrap() {
        "pdf" => data.starts_with(b"%PDF-"),
        "zip" => data.starts_with(b"PK\x03\x04"),
        "gz" => data.starts_with(&[0x1f, 0x8b]),
        "png" => data.starts_with(b"\x89PNG\r\n\x1a\n"),
        other => panic!("unknown extension {other}"),
    }
}

/// The transfers that each scenario makes, in order.
fn expected_verbs(scenario: &str, files: usize) -> Vec<&'static str> {
    match scenario {
        "list_pasv" => vec!["LIST"],
        "retr_epsv" => vec!["RETR"],
        "stor_pasv" => vec!["STOR"],
        "mixed_session" => vec!["LIST", "RETR", "STOR"],
        "batch_download" => vec!["RETR"; files],
        other => panic!("scenario {other} has no expectation"),
    }
}

fn expected_modes(scenario: &str, files: usize) -> Vec<&'static str> {
    match scenario {
        "list_pasv" | "stor_pasv" => vec!["PASV"],
        "retr_epsv" => vec!["EPSV"],
        "mixed_session" => vec!["EPSV", "PASV", "EPSV"],
        _ => vec!["PASV"; files],
    }
}

/// Checks everything about one run of one scenario and returns the control
/// connection that it read.
fn check(name: &str, run: &Run, files: usize, list_entries: usize) -> Control {
    run.result
        .as_ref()
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    for r in &run.records {
        assert_eq!(r.failed, None, "{name} {} {:?}", r.flow, r.side);
    }
    for (flow, state) in &run.states {
        assert_eq!(*state, Some(FlowState::Done), "{name} flow {flow}");
    }

    // The session structure: one control connection on port 21 and one data
    // connection per transfer.
    let control_conns = run.conns_of("control");
    assert_eq!(control_conns.len(), 1, "{name}: control connections");
    assert_eq!(control_conns[0].port, 21);
    let ctl = read_control(&control_conns[0]);
    let all = run.net.conns();
    let data: Vec<&Conn> = all.iter().filter(|c| c.flow != "control").collect();
    let verbs: Vec<&str> = ctl.transfers.iter().map(|t| t.verb.as_str()).collect();
    assert_eq!(verbs, expected_verbs(name, files), "{name}: transfers");
    assert_eq!(data.len(), ctl.transfers.len(), "{name}: data connections");
    let modes: Vec<&str> = ctl.transfers.iter().map(|t| t.mode).collect();
    assert_eq!(modes, expected_modes(name, files), "{name}: passive modes");
    // Each side of each flow session records once: two for the control flow
    // and two for each data flow.
    assert_eq!(run.records.len(), 2 + 2 * data.len(), "{name}: records");
    let mut ids: Vec<u64> = run.records.iter().map(|r| r.session_id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(
        ids.len(),
        1 + data.len(),
        "{name}: one session id per connection"
    );

    for (t, c) in ctl.transfers.iter().zip(&data) {
        assert_eq!(
            c.port, t.port,
            "{name}: data connection to the announced port"
        );
        assert!((50000..=50999).contains(&t.port), "{name}: port {}", t.port);
        match t.mode {
            "PASV" => assert_eq!(t.host.as_deref(), Some("192.0.2.10")),
            _ => assert_eq!(t.host, None),
        }
        match t.verb.as_str() {
            "LIST" => {
                assert!(
                    c.c2s().is_empty(),
                    "{name}: the client sends nothing on LIST"
                );
                let lines = text_lines(&c.s2c());
                assert_eq!(lines.len(), list_entries, "{name}: listing lines");
                for l in &lines {
                    let f: Vec<&str> = l.split_whitespace().collect();
                    assert!(f.len() >= 9, "{l}");
                    assert!(matches!(f[0].as_bytes()[0], b'-' | b'd'), "{l}");
                    assert!(f[4].parse::<u64>().is_ok(), "{l}");
                }
                assert!(t
                    .preliminary
                    .starts_with("150 Here comes the directory listing"));
            }
            "RETR" => {
                let file = t.name.as_deref().unwrap();
                let bytes = c.s2c();
                assert!(
                    c.c2s().is_empty(),
                    "{name}: the client sends nothing on RETR"
                );
                assert_eq!(Some(bytes.len()), t.size, "{name}: SIZE equals the data");
                assert!(
                    magic_ok(file, &bytes),
                    "{name}: {file} has the wrong format"
                );
                assert_eq!(
                    t.preliminary,
                    format!(
                        "150 Opening BINARY mode data connection for {file} ({} bytes).",
                        bytes.len()
                    )
                );
            }
            "STOR" => {
                let file = t.name.as_deref().unwrap();
                let bytes = c.c2s();
                assert!(
                    c.s2c().is_empty(),
                    "{name}: the server sends nothing on STOR"
                );
                assert_eq!(Some(bytes.len()), t.size, "{name}: ALLO equals the data");
                assert!(
                    magic_ok(file, &bytes),
                    "{name}: {file} has the wrong format"
                );
                assert_eq!(t.preliminary, "150 Ok to send data.");
            }
            other => panic!("{other}"),
        }
    }

    // The synthetic credentials.
    assert_eq!(ctl.logins.len(), 1);
    let (user, password) = &ctl.logins[0];
    assert!(user.contains('.') && user.is_ascii(), "{user}");
    assert!(password.starts_with("Tst-"), "{password}");
    ctl
}

#[tokio::test(start_paused = true)]
async fn every_scenario_has_one_control_and_one_data_connection_per_transfer() {
    for (name, s) in scenarios() {
        for session_id in [1, 7, 301] {
            let r = run(&s, SEED, session_id).await;
            let files = 4;
            let ctl = check(&name, &r, files, 24);
            assert!(!ctl.transfers.is_empty(), "{name}");
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_batch_scenario_spawns_its_data_flow_once_per_file() {
    for files in [1, 3, 6] {
        let all = scenarios_with(&[("files", Value::Int(files))]);
        let (name, s) = all.iter().find(|(n, _)| n == "batch_download").unwrap();
        let r = run(s, SEED, 1).await;
        let ctl = check(name, &r, files as usize, 24);
        assert_eq!(ctl.transfers.len(), files as usize);
        // The k-th spawn of the client meets the k-th spawn of the server, so
        // every data connection carries a file of the size that its SIZE
        // reply announced. The ports come from one draw each.
        let sizes: Vec<usize> = ctl.transfers.iter().map(|t| t.size.unwrap()).collect();
        assert!(sizes.iter().all(|s| *s >= 40_000), "{sizes:?}");
    }
}

#[tokio::test(start_paused = true)]
async fn the_listing_length_follows_the_variable() {
    let all = scenarios_with(&[("list_entries", Value::Int(7))]);
    for (name, s) in &all {
        if name == "list_pasv" || name == "mixed_session" {
            let r = run(s, SEED, 1).await;
            check(name, &r, 4, 7);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn small_files_and_other_ports_work() {
    let all = scenarios_with(&[
        ("file_min", Value::Int(300)),
        ("file_max", Value::Int(400)),
        ("data_port_min", Value::Int(50000)),
        ("data_port_max", Value::Int(50000)),
    ]);
    for (name, s) in &all {
        let r = run(s, SEED, 1).await;
        let ctl = check(name, &r, 4, 24);
        assert!(ctl.transfers.iter().all(|t| t.port == 50000));
        for t in ctl.transfers.iter().filter(|t| t.verb != "LIST") {
            // The content module writes at least the size that it is asked for.
            assert!(t.size.unwrap() >= 300, "{name}: {t:?}");
        }
    }
}

#[tokio::test(start_paused = true)]
async fn sessions_draw_their_own_sizes_and_ports() {
    let all = scenarios();
    let (_, s) = all.iter().find(|(n, _)| n == "retr_epsv").unwrap();
    let mut sizes = Vec::new();
    let mut ports = Vec::new();
    for seed in 1..=8 {
        let r = run(s, seed, 1).await;
        let ctl = check("retr_epsv", &r, 1, 24);
        sizes.push(ctl.transfers[0].size.unwrap());
        ports.push(ctl.transfers[0].port);
    }
    sizes.sort_unstable();
    sizes.dedup();
    ports.sort_unstable();
    ports.dedup();
    assert!(sizes.len() >= 6, "sizes {sizes:?}");
    assert!(ports.len() >= 6, "ports {ports:?}");
}

#[tokio::test(start_paused = true)]
async fn commands_use_the_synthetic_names_of_the_module() {
    let all = scenarios();
    let (_, s) = all.iter().find(|(n, _)| n == "mixed_session").unwrap();
    let r = run(s, SEED, 1).await;
    let ctl = check("mixed_session", &r, 4, 24);
    for t in ctl.transfers.iter().filter(|t| t.verb != "LIST") {
        let file = t.name.as_deref().unwrap();
        assert!(file.contains("-2026") && file.is_ascii(), "{file}");
    }
    assert!(ctl.commands.iter().any(|c| c.starts_with("CWD /srv/ftp/")));
}

/// The sessions that the estimate averages over. Sizes that the session
/// generator draws vary a lot from session to session, so one session says
/// little about the mean that the controller needs.
const ESTIMATE_SESSIONS: u64 = 100;

/// A session of each scenario at the default parameters moves the payload
/// bytes of `expected_bytes` in the manifest, both directions of every
/// connection, within 2 percent. A scenario whose size depends on the session
/// generator is averaged over [`ESTIMATE_SESSIONS`] session ids. The controller
/// turns byte weights into session shares with these estimates, so a change of
/// a template or a fixture that alters the size of a session must change the
/// manifest.
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
            r.result.as_ref().unwrap_or_else(|e| panic!("{name}: {e}"));
            sizes.push(
                r.net
                    .conns()
                    .iter()
                    .map(|c| c.c2s().len() + c.s2c().len())
                    .sum::<usize>(),
            );
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
