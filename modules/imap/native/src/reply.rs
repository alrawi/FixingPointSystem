// SPDX-License-Identifier: GPL-3.0-only
//! Server replies of the IMAP dialogue.
//!
//! A flow cannot branch on what it reads, so the server side of a scenario
//! answers each command with one function call that reads the command line
//! from `last`. These functions parse the subset of RFC 3501 that the
//! scenarios use, the tag, the sequence set, the `FETCH` data items, and the
//! `STORE` flags, and render the matching untagged responses. A command that
//! the subset does not cover fails the calling session with the reason
//! `extension_protocol_violation`, because the bytes came from the peer.

use fps_flow::{FnError, FnErrorKind};

use crate::catalog::{MAX_MESSAGES, UID_BASE};
use crate::mime;
use crate::rng::Rng;

fn invalid(message: String) -> FnError {
    FnError::new(FnErrorKind::InvalidArgument, message)
}

fn protocol(message: String) -> FnError {
    FnError::new(FnErrorKind::Protocol, message)
}

fn push(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
}

/// One parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Command<'a> {
    pub tag: &'a str,
    /// The command name in upper case, without a `UID` prefix.
    pub name: String,
    /// Whether the command had the `UID` prefix.
    pub uid: bool,
    /// The arguments after the command name.
    pub rest: &'a str,
}

fn tag_ok(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 32
        && tag
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b"(){%*\"\\]+".contains(&b))
}

/// Parses the first line of `line` as `tag [UID] NAME arguments`.
pub(crate) fn parse_command<'a>(function: &str, line: &'a [u8]) -> Result<Command<'a>, FnError> {
    let text = std::str::from_utf8(line)
        .map_err(|_| protocol(format!("the command of '{function}' is not UTF-8 text")))?;
    let first = text.split(['\r', '\n']).next().unwrap_or("").trim();
    let (tag, rest) = first
        .split_once(' ')
        .ok_or_else(|| protocol(format!("the command of '{function}' has no command name")))?;
    if !tag_ok(tag) {
        return Err(protocol(format!(
            "'{tag}' is not a valid tag in the command of '{function}'"
        )));
    }
    let rest = rest.trim_start();
    let (name, rest) = rest.split_once(' ').unwrap_or((rest, ""));
    let mut name = name.to_ascii_uppercase();
    let mut rest = rest.trim();
    let mut uid = false;
    if name == "UID" {
        uid = true;
        let (n, r) = rest.split_once(' ').unwrap_or((rest, ""));
        name = n.to_ascii_uppercase();
        rest = r.trim();
    }
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(protocol(format!(
            "the command of '{function}' has no valid command name"
        )));
    }
    Ok(Command {
        tag,
        name,
        uid,
        rest,
    })
}

/// Expands a sequence set such as `1:4,7,9:*` into message numbers. With
/// `uid` the set names UIDs, which the module maps to message numbers.
fn expand_set(function: &str, set: &str, exists: i64, uid: bool) -> Result<Vec<i64>, FnError> {
    let top = if uid { UID_BASE + exists } else { exists };
    let num = |s: &str| -> Result<i64, FnError> {
        if s == "*" {
            if top < 1 {
                return Err(invalid(format!(
                    "'*' in the sequence set of '{function}' needs 'exists' above zero"
                )));
            }
            return Ok(top);
        }
        s.parse::<i64>()
            .ok()
            .filter(|n| *n >= 1)
            .ok_or_else(|| protocol(format!("'{s}' is not a number of a sequence set")))
    };
    let mut seqs = Vec::new();
    for part in set.split(',') {
        let (a, b) = match part.split_once(':') {
            Some((a, b)) => (num(a)?, num(b)?),
            None => {
                let n = num(part)?;
                (n, n)
            }
        };
        let (lo, hi) = (a.min(b), a.max(b));
        if hi - lo + 1 + seqs.len() as i64 > MAX_MESSAGES {
            return Err(protocol(format!(
                "the sequence set of '{function}' names more than {MAX_MESSAGES} messages"
            )));
        }
        for n in lo..=hi {
            let seq = if uid { n - UID_BASE } else { n };
            if seq < 1 {
                return Err(protocol(format!(
                    "UID {n} in the command of '{function}' names no message"
                )));
            }
            seqs.push(seq);
        }
    }
    Ok(seqs)
}

/// The flags of message `seq`. They depend only on the number, so a listing
/// and a later fetch of one message agree.
fn flags_of(seq: i64) -> &'static [&'static str] {
    let mut r = Rng::new((seq as u64) ^ 0x1F5);
    match r.below(20) {
        0..=11 => &["\\Seen"],
        12..=15 => &[],
        16 => &["\\Seen", "\\Flagged"],
        _ => &["\\Answered", "\\Seen"],
    }
}

fn flag_list(flags: &[&str]) -> String {
    format!("({})", flags.join(" "))
}

/// The UID of message `seq`.
fn uid_of(seq: i64) -> i64 {
    UID_BASE + seq
}

/// Splits `text` at spaces that lie outside brackets and parentheses.
fn split_top(text: &str) -> Result<Vec<String>, FnError> {
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    for c in text.chars() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return Err(protocol("unbalanced brackets in a FETCH item list".into()));
        }
        if c == ' ' && depth == 0 {
            if !cur.is_empty() {
                items.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if depth != 0 {
        return Err(protocol("unbalanced brackets in a FETCH item list".into()));
    }
    if !cur.is_empty() {
        items.push(cur);
    }
    Ok(items)
}

/// The part of a message that a body item names.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Section {
    Full,
    Header,
    Fields(Vec<String>),
    Text,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Item {
    Uid,
    Flags,
    Size,
    InternalDate,
    Body {
        /// The name that the response carries.
        label: String,
        section: Section,
        /// A fetch that is not a peek sets the `\Seen` flag.
        peek: bool,
    },
}

fn parse_item(raw: &str) -> Result<Item, FnError> {
    let up = raw.to_ascii_uppercase();
    let body = |label: &str, section, peek| Item::Body {
        label: label.to_string(),
        section,
        peek,
    };
    match up.as_str() {
        "UID" => return Ok(Item::Uid),
        "FLAGS" => return Ok(Item::Flags),
        "RFC822.SIZE" => return Ok(Item::Size),
        "INTERNALDATE" => return Ok(Item::InternalDate),
        "RFC822" => return Ok(body("RFC822", Section::Full, false)),
        "RFC822.HEADER" => return Ok(body("RFC822.HEADER", Section::Header, true)),
        "RFC822.TEXT" => return Ok(body("RFC822.TEXT", Section::Text, false)),
        _ => {}
    }
    let (peek, inner) = if let Some(r) = up.strip_prefix("BODY.PEEK[") {
        (true, r)
    } else if let Some(r) = up.strip_prefix("BODY[") {
        (false, r)
    } else {
        return Err(protocol(format!("the FETCH item '{raw}' is not supported")));
    };
    let section = inner
        .strip_suffix(']')
        .ok_or_else(|| protocol(format!("the FETCH item '{raw}' is not supported")))?;
    match section {
        "" => Ok(body("BODY[]", Section::Full, peek)),
        "HEADER" => Ok(body("BODY[HEADER]", Section::Header, peek)),
        "TEXT" => Ok(body("BODY[TEXT]", Section::Text, peek)),
        other => {
            let list = other
                .strip_prefix("HEADER.FIELDS (")
                .and_then(|r| r.strip_suffix(')'))
                .ok_or_else(|| protocol(format!("the FETCH item '{raw}' is not supported")))?;
            let fields: Vec<String> = list.split_whitespace().map(str::to_string).collect();
            let ok = !fields.is_empty()
                && fields
                    .iter()
                    .all(|f| f.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
            if !ok {
                return Err(protocol(format!("the FETCH item '{raw}' is not supported")));
            }
            let label = format!("BODY[HEADER.FIELDS ({})]", fields.join(" "));
            Ok(body(&label, Section::Fields(fields), peek))
        }
    }
}

/// Splits a message into its header block, which ends with the empty line,
/// and its body.
pub(crate) fn split_message(message: &[u8]) -> (&[u8], &[u8]) {
    match message.windows(4).position(|w| w == b"\r\n\r\n") {
        Some(i) => message.split_at(i + 4),
        None => (message, &[]),
    }
}

/// The header lines whose field name is in `names`, with their continuation
/// lines and the closing empty line.
fn header_fields(header: &[u8], names: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut keep = false;
    for line in header.split_inclusive(|&b| b == b'\n') {
        if line == b"\r\n" {
            break;
        }
        if line.first().is_some_and(|b| *b == b' ' || *b == b'\t') {
            if keep {
                out.extend_from_slice(line);
            }
            continue;
        }
        let name = line.split(|&b| b == b':').next().unwrap_or(&[]);
        keep = names
            .iter()
            .any(|n| n.as_bytes().eq_ignore_ascii_case(name));
        if keep {
            out.extend_from_slice(line);
        }
    }
    out.extend_from_slice(b"\r\n");
    out
}

/// Converts the value of a `Date` header, such as
/// `Sat, 25 Jul 2026 17:57:51 +0000`, to the INTERNALDATE form.
fn internal_date(date: &str) -> Option<String> {
    let t: Vec<&str> = date.split_whitespace().collect();
    let t = if t.first().is_some_and(|w| w.ends_with(',')) {
        &t[1..]
    } else {
        &t[..]
    };
    if t.len() < 5 {
        return None;
    }
    let day: u32 = t[0].parse().ok()?;
    Some(format!("\"{day:02}-{}-{} {} {}\"", t[1], t[2], t[3], t[4]))
}

fn date_of(message: &[u8], seq: i64) -> String {
    let (header, _) = split_message(message);
    let from_header = header
        .split(|&b| b == b'\n')
        .filter_map(|l| std::str::from_utf8(l).ok())
        .find_map(|l| l.strip_prefix("Date:"))
        .and_then(|d| internal_date(d.trim()));
    from_header.unwrap_or_else(|| {
        let mut r = Rng::new((seq as u64) ^ 0xDA7E);
        internal_date(&mime::date(&mut r)).unwrap_or_default()
    })
}

fn literal(out: &mut Vec<u8>, label: &str, data: &[u8]) {
    push(out, &format!("{label} {{{}}}\r\n", data.len()));
    out.extend_from_slice(data);
}

/// Renders the untagged `FETCH` responses of a `FETCH` command. A command
/// that asks for message data needs `message` and one message number.
pub(crate) fn fetch(
    command: &[u8],
    message: &[u8],
    exists: i64,
    out: &mut Vec<u8>,
) -> Result<(), FnError> {
    let c = parse_command("fetch", command)?;
    if c.name != "FETCH" {
        return Err(protocol(format!(
            "'fetch' answers FETCH commands, not {}",
            c.name
        )));
    }
    if !(0..=MAX_MESSAGES).contains(&exists) {
        return Err(invalid(format!(
            "'exists' of 'fetch' must lie between 0 and {MAX_MESSAGES}, not {exists}"
        )));
    }
    let (set, spec) = c
        .rest
        .split_once(' ')
        .ok_or_else(|| protocol("the FETCH command has no data items".into()))?;
    let spec = spec.trim();
    let list = spec
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(spec);
    let mut items = split_top(list)?
        .iter()
        .map(|s| parse_item(s))
        .collect::<Result<Vec<_>, _>>()?;
    if items.is_empty() {
        return Err(protocol("the FETCH command has no data items".into()));
    }
    let seqs = expand_set("fetch", set, exists, c.uid)?;
    let carries = items.iter().any(|i| matches!(i, Item::Body { .. }));
    if carries && message.is_empty() {
        return Err(invalid(
            "'message' of 'fetch' must not be empty when the command asks for message data".into(),
        ));
    }
    if (carries || !message.is_empty()) && seqs.len() != 1 {
        return Err(invalid(
            "'fetch' renders message data for exactly one message number".into(),
        ));
    }
    // A UID FETCH always answers with the UID, and a fetch that is not a peek
    // reports the flag that it sets.
    if c.uid && !items.contains(&Item::Uid) {
        items.insert(0, Item::Uid);
    }
    let marks_seen = items
        .iter()
        .any(|i| matches!(i, Item::Body { peek: false, .. }));
    let (header, body) = split_message(message);

    for seq in seqs {
        let mut flags: Vec<&str> = flags_of(seq).to_vec();
        if marks_seen && !flags.contains(&"\\Seen") {
            flags.push("\\Seen");
        }
        push(out, &format!("* {seq} FETCH ("));
        let mut first = true;
        let mut item_out = |out: &mut Vec<u8>, f: &dyn Fn(&mut Vec<u8>)| {
            if !first {
                out.push(b' ');
            }
            first = false;
            f(out);
        };
        for item in &items {
            match item {
                Item::Uid => item_out(out, &|o| push(o, &format!("UID {}", uid_of(seq)))),
                Item::Flags => item_out(out, &|o| push(o, &format!("FLAGS {}", flag_list(&flags)))),
                Item::Size => {
                    let size = if message.is_empty() {
                        1500 + (Rng::new((seq as u64) ^ 0x51E).below(118_000) as i64)
                    } else {
                        message.len() as i64
                    };
                    item_out(out, &|o| push(o, &format!("RFC822.SIZE {size}")));
                }
                Item::InternalDate => {
                    let d = date_of(message, seq);
                    item_out(out, &|o| push(o, &format!("INTERNALDATE {d}")));
                }
                Item::Body { label, section, .. } => {
                    let data: Vec<u8>;
                    let part: &[u8] = match section {
                        Section::Full => message,
                        Section::Header => header,
                        Section::Text => body,
                        Section::Fields(names) => {
                            data = header_fields(header, names);
                            &data
                        }
                    };
                    item_out(out, &|o| literal(o, label, part));
                }
            }
        }
        // The implied flag of a fetch that is not a peek.
        if marks_seen && !items.contains(&Item::Flags) {
            out.extend_from_slice(format!(" FLAGS {}", flag_list(&flags)).as_bytes());
        }
        push(out, ")\r\n");
    }
    Ok(())
}

/// The arguments of a `STORE` command.
struct Store<'a> {
    uid: bool,
    seqs: Vec<i64>,
    /// `=` replaces the flags, `+` adds them, and `-` removes them.
    mode: char,
    silent: bool,
    given: Vec<&'a str>,
}

fn parse_store<'a>(function: &str, command: &'a [u8]) -> Result<Store<'a>, FnError> {
    let c = parse_command(function, command)?;
    if c.name != "STORE" {
        return Err(protocol(format!(
            "'{function}' answers STORE commands, not {}",
            c.name
        )));
    }
    let mut parts = c.rest.splitn(3, ' ');
    let (set, item, flags) = (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or("").to_ascii_uppercase(),
        parts.next().unwrap_or("").trim(),
    );
    let (mode, silent) = match item.as_str() {
        "FLAGS" => ('=', false),
        "+FLAGS" => ('+', false),
        "-FLAGS" => ('-', false),
        "FLAGS.SILENT" => ('=', true),
        "+FLAGS.SILENT" => ('+', true),
        "-FLAGS.SILENT" => ('-', true),
        other => {
            return Err(protocol(format!(
                "'{other}' is not a STORE data item that '{function}' knows"
            )))
        }
    };
    let list = flags
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(flags);
    let given: Vec<&str> = list.split_whitespace().collect();
    let flag_ok = |f: &str| {
        let atom = f.strip_prefix('\\').unwrap_or(f);
        !atom.is_empty()
            && atom
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'$' || b == b'_' || b == b'-')
    };
    if (given.is_empty() && mode != '=') || !given.iter().all(|f| flag_ok(f)) {
        return Err(protocol(format!("'{flags}' is not a flag list")));
    }
    // The module knows no mailbox, so the sequence set may name any number.
    let seqs = expand_set(function, set, MAX_MESSAGES, c.uid)?;
    Ok(Store {
        uid: c.uid,
        seqs,
        mode,
        silent,
        given,
    })
}

/// Renders the untagged responses of a `STORE` command.
pub(crate) fn store(command: &[u8], out: &mut Vec<u8>) -> Result<(), FnError> {
    let s = parse_store("store_reply", command)?;
    if s.silent {
        return Ok(());
    }
    for seq in s.seqs {
        let mut result: Vec<&str> = match s.mode {
            '=' => Vec::new(),
            _ => flags_of(seq).to_vec(),
        };
        match s.mode {
            '-' => result.retain(|f| !s.given.iter().any(|g| g.eq_ignore_ascii_case(f))),
            _ => {
                for g in &s.given {
                    if !result.iter().any(|f| f.eq_ignore_ascii_case(g)) {
                        result.push(g);
                    }
                }
            }
        }
        let uid = if s.uid {
            format!("UID {} ", uid_of(seq))
        } else {
            String::new()
        };
        push(
            out,
            &format!("* {seq} FETCH ({uid}FLAGS {})\r\n", flag_list(&result)),
        );
    }
    Ok(())
}

/// Renders the `EXPUNGE` responses that follow a `STORE` of the `\Deleted`
/// flag. Each message that goes shifts the numbers of the later ones, so the
/// i-th message of the ascending set reports its number minus i.
pub(crate) fn expunge(stored: &[u8], out: &mut Vec<u8>) -> Result<(), FnError> {
    let s = parse_store("expunge_reply", stored)?;
    if s.mode == '-' || !s.given.iter().any(|f| f.eq_ignore_ascii_case("\\Deleted")) {
        return Err(invalid(
            "the command of 'expunge_reply' must add the \\Deleted flag".into(),
        ));
    }
    let mut seqs = s.seqs;
    seqs.sort_unstable();
    seqs.dedup();
    for (i, seq) in seqs.iter().enumerate() {
        push(out, &format!("* {} EXPUNGE\r\n", seq - i as i64));
    }
    Ok(())
}

/// Renders the untagged responses of `SELECT` and `EXAMINE`.
pub(crate) fn select_data(
    exists: i64,
    recent: i64,
    unseen: i64,
    uidvalidity: Option<i64>,
    read_only: bool,
    drawn: u64,
    out: &mut Vec<u8>,
) -> Result<(), FnError> {
    if !(0..=MAX_MESSAGES).contains(&exists) {
        return Err(invalid(format!(
            "'exists' of 'select_data' must lie between 0 and {MAX_MESSAGES}, not {exists}"
        )));
    }
    if !(0..=exists).contains(&recent) || !(0..=exists).contains(&unseen) {
        return Err(invalid(
            "'recent' and 'unseen' of 'select_data' must lie between 0 and 'exists'".into(),
        ));
    }
    let validity = match uidvalidity {
        Some(v) if (1..=i64::from(u32::MAX)).contains(&v) => v,
        Some(v) => {
            return Err(invalid(format!(
                "'uidvalidity' of 'select_data' must lie between 1 and 4294967295, not {v}"
            )))
        }
        None => 1_000_000_000 + (drawn % 700_000_000) as i64,
    };
    push(
        out,
        "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n",
    );
    if read_only {
        push(
            out,
            "* OK [PERMANENTFLAGS ()] No permanent flags permitted.\r\n",
        );
    } else {
        push(
            out,
            "* OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft \\*)] Flags permitted.\r\n",
        );
    }
    push(out, &format!("* {exists} EXISTS\r\n* {recent} RECENT\r\n"));
    if unseen > 0 {
        push(out, &format!("* OK [UNSEEN {unseen}] First unseen.\r\n"));
    }
    push(
        out,
        &format!(
            "* OK [UIDVALIDITY {validity}] UIDs valid\r\n* OK [UIDNEXT {}] Predicted next UID\r\n",
            UID_BASE + exists + 1
        ),
    );
    Ok(())
}

/// Renders the `SEARCH` response that lists `hits` of `exists` messages.
pub(crate) fn search_reply(
    exists: i64,
    hits: i64,
    rng: &mut Rng,
    out: &mut Vec<u8>,
) -> Result<(), FnError> {
    if !(0..=MAX_MESSAGES).contains(&exists) || !(0..=exists).contains(&hits) {
        return Err(invalid(format!(
            "'hits' of 'search_reply' must lie between 0 and 'exists', and 'exists' between 0 and {MAX_MESSAGES}, not {hits} and {exists}"
        )));
    }
    let mut pool: Vec<i64> = (1..=exists).collect();
    for i in 0..hits as usize {
        let j = i + rng.below(pool.len() - i);
        pool.swap(i, j);
    }
    let mut chosen = pool[..hits as usize].to_vec();
    chosen.sort_unstable();
    push(out, "* SEARCH");
    for n in chosen {
        push(out, &format!(" {n}"));
    }
    push(out, "\r\n");
    Ok(())
}

/// Returns the `index`th message number of the `SEARCH` response in `response`.
pub(crate) fn search_hit(response: &[u8], index: i64) -> Result<i64, FnError> {
    let text = std::str::from_utf8(response)
        .map_err(|_| protocol("the response of 'search_hit' is not UTF-8 text".into()))?;
    let line = text
        .split("\r\n")
        .find_map(|l| l.strip_prefix("* SEARCH"))
        .ok_or_else(|| protocol("the response of 'search_hit' has no SEARCH line".into()))?;
    let hits: Vec<i64> = line
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| {
            protocol("the SEARCH line of 'search_hit' holds a value that is no number".into())
        })?;
    usize::try_from(index - 1)
        .ok()
        .and_then(|i| hits.get(i).copied())
        .ok_or_else(|| {
            protocol(format!(
                "the SEARCH response has no hit number {index}, it lists {}",
                hits.len()
            ))
        })
}

/// Returns the size that the last `{n}` or `{n+}` of a command line announces.
pub(crate) fn literal_size(command: &[u8]) -> Result<i64, FnError> {
    let text = std::str::from_utf8(command)
        .map_err(|_| protocol("the command of 'literal_size' is not UTF-8 text".into()))?;
    let line = text.split(['\r', '\n']).next().unwrap_or("").trim_end();
    let inner = line
        .strip_suffix('}')
        .and_then(|l| l.rsplit_once('{'))
        .map(|(_, n)| n.strip_suffix('+').unwrap_or(n))
        .ok_or_else(|| {
            protocol("the command of 'literal_size' does not end in a literal size".into())
        })?;
    inner
        .parse::<i64>()
        .ok()
        .filter(|n| (0..=64 << 20).contains(n) && inner.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| protocol(format!("'{inner}' is not a valid literal size")))
}

/// Returns the number or tag that follows `value` by `step`. A tag is letters
/// followed by digits, and the digits keep their width.
pub(crate) fn next(value: &str, step: i64) -> Result<String, FnError> {
    let split = value
        .find(|c: char| c.is_ascii_digit())
        .ok_or_else(|| invalid(format!("'{value}' holds no number for 'next' to count")))?;
    let (prefix, digits) = value.split_at(split);
    if !prefix.bytes().all(|b| b.is_ascii_alphabetic())
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid(format!(
            "'{value}' is neither a number nor letters followed by a number"
        )));
    }
    let n: i64 = digits
        .parse()
        .map_err(|_| invalid(format!("'{value}' is too large for 'next'")))?;
    let n = n
        .checked_add(step)
        .filter(|n| *n >= 0)
        .ok_or_else(|| invalid(format!("'step' of 'next' takes '{value}' below zero")))?;
    Ok(format!("{prefix}{n:0width$}", width = digits.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: impl FnOnce(&mut Vec<u8>) -> Result<(), FnError>) -> String {
        let mut out = Vec::new();
        f(&mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    const MSG: &[u8] = b"From: a.b@example.test\r\nTo: c.d@example.test,\r\n e.f@example.test\r\nSubject: Hi\r\nDate: Sat, 25 Jul 2026 17:57:51 +0000\r\nX-Mailer: m\r\n\r\nbody line\r\n";

    #[test]
    fn commands_parse_with_and_without_uid() {
        let c = parse_command("t", b"A001 uid fetch 3 (FLAGS)\r\n").unwrap();
        assert_eq!(
            (c.tag, c.name.as_str(), c.uid, c.rest),
            ("A001", "FETCH", true, "3 (FLAGS)")
        );
        let c = parse_command("t", b"a1 NOOP\r\n").unwrap();
        assert_eq!((c.name.as_str(), c.rest), ("NOOP", ""));
        for bad in [
            &b"\r\n"[..],
            b"A001\r\n",
            b"A(1 NOOP\r\n",
            b"+ NOOP\r\n",
            &[0xff, 0xfe],
        ] {
            assert_eq!(
                parse_command("t", bad).unwrap_err().kind,
                FnErrorKind::Protocol
            );
        }
    }

    #[test]
    fn next_counts_tags_and_numbers() {
        assert_eq!(next("A009", 1).unwrap(), "A010");
        assert_eq!(next("A000", 1).unwrap(), "A001");
        assert_eq!(next("A999", 1).unwrap(), "A1000");
        assert_eq!(next("6", 1).unwrap(), "7");
        assert_eq!(next("12", 3).unwrap(), "15");
        assert!(next("A000", -1).is_err());
        assert!(next("abc", 1).is_err());
        assert!(next("A1B2", 1).is_err());
    }

    #[test]
    fn literal_sizes_come_from_the_end_of_the_line() {
        assert_eq!(literal_size(b"A002 LOGIN user {12}\r\n").unwrap(), 12);
        assert_eq!(
            literal_size(b"A003 APPEND Sent (\\Seen) {4096+}\r\n").unwrap(),
            4096
        );
        for bad in [
            &b"A002 NOOP\r\n"[..],
            b"A002 X {}\r\n",
            b"A002 X {-1}\r\n",
            b"A002 X {9999999999}\r\n",
        ] {
            assert!(literal_size(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn sets_expand_and_uid_sets_map_to_numbers() {
        assert_eq!(
            expand_set("t", "1:3,7", 9, false).unwrap(),
            vec![1, 2, 3, 7]
        );
        assert_eq!(expand_set("t", "5:*", 7, false).unwrap(), vec![5, 6, 7]);
        assert_eq!(expand_set("t", "3:1", 7, false).unwrap(), vec![1, 2, 3]);
        assert_eq!(
            expand_set("t", "1002:1004", 7, true).unwrap(),
            vec![2, 3, 4]
        );
        assert!(expand_set("t", "0", 7, false).is_err());
        assert!(expand_set("t", "x", 7, false).is_err());
        assert!(expand_set("t", "*", 0, false).is_err());
        assert!(expand_set("t", "1000", 7, true).is_err());
    }

    #[test]
    fn a_flag_listing_is_stable_per_message() {
        let a = run(|o| fetch(b"A004 FETCH 1:4 (UID FLAGS)\r\n", b"", 4, o));
        let b = run(|o| fetch(b"A009 FETCH 3 (FLAGS)\r\n", b"", 4, o));
        let lines: Vec<&str> = a.split_terminator("\r\n").collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[2].starts_with("* 3 FETCH (UID 1003 FLAGS ("));
        assert_eq!(
            b.trim_end().split_once("FLAGS ").unwrap().1,
            lines[2].split_once("FLAGS ").unwrap().1
        );
    }

    #[test]
    fn a_header_fetch_returns_the_listed_fields_with_folded_lines() {
        let s = run(|o| {
            fetch(
                b"A005 FETCH 2 (UID BODY.PEEK[HEADER.FIELDS (From To Subject)])\r\n",
                MSG,
                5,
                o,
            )
        });
        let want = "From: a.b@example.test\r\nTo: c.d@example.test,\r\n e.f@example.test\r\nSubject: Hi\r\n\r\n";
        assert_eq!(
            s,
            format!(
                "* 2 FETCH (UID 1002 BODY[HEADER.FIELDS (FROM TO SUBJECT)] {{{}}}\r\n{want})\r\n",
                want.len()
            )
        );
    }

    #[test]
    fn a_full_fetch_carries_the_message_as_one_literal() {
        let s = run(|o| {
            fetch(
                b"A006 FETCH 1 (RFC822.SIZE INTERNALDATE BODY[])\r\n",
                MSG,
                5,
                o,
            )
        });
        assert!(s.starts_with("* 1 FETCH (RFC822.SIZE "));
        assert!(s.contains(" INTERNALDATE \"25-Jul-2026 17:57:51 +0000\" "));
        assert!(s.contains(&format!("BODY[] {{{}}}\r\n", MSG.len())));
        // The literal is followed by the implied flags of a fetch that is not
        // a peek, and the response ends with one closing parenthesis.
        assert!(s.ends_with(&format!(
            "body line\r\n FLAGS {})\r\n",
            flag_list(&{
                let mut f = flags_of(1).to_vec();
                if !f.contains(&"\\Seen") {
                    f.push("\\Seen");
                }
                f
            })
        )));
        let peek = run(|o| fetch(b"A006 FETCH 1 (BODY.PEEK[])\r\n", MSG, 5, o));
        assert!(peek.ends_with("body line\r\n)\r\n"));
        let text = run(|o| fetch(b"A006 FETCH 1 (BODY.PEEK[TEXT])\r\n", MSG, 5, o));
        assert!(text.contains("BODY[TEXT] {11}\r\nbody line\r\n)"));
    }

    #[test]
    fn a_uid_fetch_answers_with_the_uid() {
        let s = run(|o| fetch(b"A007 UID FETCH 1003 (FLAGS)\r\n", b"", 5, o));
        assert!(s.starts_with("* 3 FETCH (UID 1003 FLAGS ("), "{s}");
    }

    #[test]
    fn fetch_refuses_what_it_cannot_answer() {
        let cases: Vec<(&[u8], &[u8])> = vec![
            (b"A1 FETCH 1 (ENVELOPE)\r\n", b""),
            (b"A1 FETCH 1 (BODY[1.MIME])\r\n", MSG),
            (b"A1 FETCH 1 (BODY[]<0.10>)\r\n", MSG),
            (b"A1 NOOP\r\n", b""),
            (b"A1 FETCH 1\r\n", b""),
            (b"A1 FETCH 1 (BODY[])\r\n", b""),
            (b"A1 FETCH 1:2 (BODY[])\r\n", MSG),
        ];
        for (cmd, msg) in cases {
            assert!(fetch(cmd, msg, 5, &mut Vec::new()).is_err(), "{cmd:?}");
        }
    }

    #[test]
    fn store_answers_with_the_new_flags() {
        let s = run(|o| store(b"A010 STORE 3 +FLAGS (\\Seen \\Flagged)\r\n", o));
        assert!(s.starts_with("* 3 FETCH (FLAGS ("));
        assert!(s.contains("\\Seen") && s.contains("\\Flagged"));
        assert_eq!(
            run(|o| store(b"A010 STORE 3 FLAGS (\\Deleted)\r\n", o)),
            "* 3 FETCH (FLAGS (\\Deleted))\r\n"
        );
        assert_eq!(
            run(|o| store(
                b"A010 STORE 1:2 -FLAGS (\\Seen \\Flagged \\Answered)\r\n",
                o
            )),
            "* 1 FETCH (FLAGS ())\r\n* 2 FETCH (FLAGS ())\r\n"
        );
        assert_eq!(
            run(|o| store(b"A010 STORE 3 +FLAGS.SILENT (\\Seen)\r\n", o)),
            ""
        );
        let u = run(|o| store(b"A010 UID STORE 1004 +FLAGS (\\Deleted)\r\n", o));
        assert!(u.starts_with("* 4 FETCH (UID 1004 FLAGS ("));
        for bad in [
            &b"A1 STORE 3 +FLAGS\r\n"[..],
            b"A1 STORE 3 ~FLAGS (\\Seen)\r\n",
            b"A1 STORE 3 +FLAGS (a\"b)\r\n",
            b"A1 FETCH 3 (FLAGS)\r\n",
        ] {
            assert!(store(bad, &mut Vec::new()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn expunge_reports_shifted_numbers() {
        assert_eq!(
            run(|o| expunge(b"A1 STORE 3 +FLAGS (\\Deleted)\r\n", o)),
            "* 3 EXPUNGE\r\n"
        );
        assert_eq!(
            run(|o| expunge(b"A1 STORE 2:4 +FLAGS.SILENT (\\Seen \\Deleted)\r\n", o)),
            "* 2 EXPUNGE\r\n* 2 EXPUNGE\r\n* 2 EXPUNGE\r\n"
        );
        assert!(expunge(b"A1 STORE 3 +FLAGS (\\Seen)\r\n", &mut Vec::new()).is_err());
        assert!(expunge(b"A1 STORE 3 -FLAGS (\\Deleted)\r\n", &mut Vec::new()).is_err());
    }

    #[test]
    fn select_data_follows_rfc_3501() {
        let s = run(|o| select_data(12, 2, 4, Some(1_700_000_001), false, 0, o));
        assert_eq!(
            s,
            "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n* OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft \\*)] Flags permitted.\r\n* 12 EXISTS\r\n* 2 RECENT\r\n* OK [UNSEEN 4] First unseen.\r\n* OK [UIDVALIDITY 1700000001] UIDs valid\r\n* OK [UIDNEXT 1013] Predicted next UID\r\n"
        );
        let r = run(|o| select_data(0, 0, 0, None, true, 7, o));
        assert!(
            r.contains("PERMANENTFLAGS ()") && !r.contains("UNSEEN") && r.contains("* 0 EXISTS")
        );
        assert!(select_data(3, 4, 0, None, false, 0, &mut Vec::new()).is_err());
        assert!(select_data(3, 0, 0, Some(0), false, 0, &mut Vec::new()).is_err());
    }

    #[test]
    fn search_lists_distinct_ascending_numbers_and_the_client_reads_them() {
        let mut r = Rng::new(5);
        for hits in 0..=6 {
            let s = run(|o| search_reply(6, hits, &mut r, o));
            let nums: Vec<i64> = s
                .split_whitespace()
                .skip(2)
                .map(|n| n.parse().unwrap())
                .collect();
            assert_eq!(nums.len(), hits as usize);
            assert!(
                nums.windows(2).all(|w| w[0] < w[1]) && nums.iter().all(|n| (1..=6).contains(n))
            );
        }
        assert!(search_reply(3, 4, &mut r, &mut Vec::new()).is_err());
        let resp = b"* SEARCH 2 5 9\r\nA011 OK Search completed (0.001 + 0.000 secs).\r\n";
        assert_eq!(search_hit(resp, 1).unwrap(), 2);
        assert_eq!(search_hit(resp, 3).unwrap(), 9);
        assert!(search_hit(resp, 4).is_err());
        assert!(search_hit(b"* SEARCH\r\n", 1).is_err());
        assert!(search_hit(b"A1 OK done\r\n", 1).is_err());
    }
}
