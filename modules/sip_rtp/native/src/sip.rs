// SPDX-License-Identifier: GPL-3.0-only
//! SIP messages of RFC 3261 over UDP.
//!
//! The module writes the messages of one call, which are the INVITE, the
//! provisional and final responses, the ACK, the BYE, and the response to the
//! BYE. It reads the headers that tie a message to its dialog, so every
//! response and in dialog request repeats what the request carried.

use rand::RngCore;

use crate::codec::Codec;
use crate::sdp::{self, Offered, Sdp};

/// The value of the `User-Agent` and `Server` headers.
pub(crate) const AGENT: &str = "FixingPoint-SIP/0.1";

/// The methods that the module lists in `Allow`.
const ALLOW: &str = "INVITE, ACK, CANCEL, BYE, OPTIONS, NOTIFY";

/// A parsed SIP message that borrows its bytes.
#[derive(Debug)]
pub(crate) struct Message<'a> {
    /// The request line or the status line.
    pub start: &'a str,
    /// The headers in order of appearance, names as written.
    pub headers: Vec<(&'a str, &'a str)>,
    /// Everything after the empty line.
    pub body: &'a str,
}

impl<'a> Message<'a> {
    /// Parses a message and checks `Content-Length` against the body.
    pub(crate) fn parse(bytes: &'a [u8]) -> Result<Message<'a>, String> {
        let text = std::str::from_utf8(bytes).map_err(|_| "the SIP message is not UTF-8")?;
        let (head, body) = text
            .split_once("\r\n\r\n")
            .ok_or("the SIP message has no empty line after its headers")?;
        let mut lines = head.split("\r\n");
        let start = lines.next().unwrap_or("");
        let mut headers = Vec::new();
        for line in lines {
            let (name, value) = line
                .split_once(':')
                .ok_or_else(|| format!("bad SIP header line '{line}'"))?;
            headers.push((name.trim(), value.trim()));
        }
        let m = Message {
            start,
            headers,
            body,
        };
        if let Some(len) = m.header("Content-Length") {
            let len: usize = len
                .parse()
                .map_err(|_| format!("bad Content-Length '{len}'"))?;
            if len != body.len() {
                return Err(format!(
                    "Content-Length says {len} bytes and the body has {}",
                    body.len()
                ));
            }
        }
        Ok(m)
    }

    /// Returns the first header with this name, ignoring case.
    pub(crate) fn header(&self, name: &str) -> Option<&'a str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| *v)
    }

    /// Returns every header with this name, in order.
    pub(crate) fn all(&self, name: &str) -> Vec<&'a str> {
        self.headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| *v)
            .collect()
    }

    fn need(&self, name: &str) -> Result<&'a str, String> {
        self.header(name)
            .ok_or_else(|| format!("the SIP message has no {name} header"))
    }

    /// Returns the method of a request, or `None` for a response.
    pub(crate) fn method(&self) -> Option<&'a str> {
        if self.start.starts_with("SIP/2.0 ") {
            None
        } else {
            self.start.split(' ').next()
        }
    }

    /// Returns the status code of a response, or `None` for a request.
    pub(crate) fn status(&self) -> Option<u16> {
        self.start
            .strip_prefix("SIP/2.0 ")
            .and_then(|r| r.split(' ').next())
            .and_then(|c| c.parse().ok())
    }

    /// Returns the method that the `CSeq` header names.
    pub(crate) fn cseq(&self) -> Result<(u32, &'a str), String> {
        let v = self.need("CSeq")?;
        let (n, m) = v.split_once(' ').ok_or_else(|| format!("bad CSeq '{v}'"))?;
        Ok((n.parse().map_err(|_| format!("bad CSeq '{v}'"))?, m.trim()))
    }
}

/// Returns the URI of an address header, which is the text inside `<` and
/// `>`, or the text up to the first `;` when it has no brackets.
pub(crate) fn uri_of(value: &str) -> &str {
    match (value.find('<'), value.find('>')) {
        (Some(a), Some(b)) if a < b => &value[a + 1..b],
        _ => value.split(';').next().unwrap_or(value).trim(),
    }
}

/// Splits `sip:user@host:port;params` into the user and the host with its
/// port.
fn user_and_host(uri: &str) -> (&str, &str) {
    let rest = uri.strip_prefix("sip:").unwrap_or(uri);
    let rest = rest.split(';').next().unwrap_or(rest);
    match rest.split_once('@') {
        Some((u, h)) => (u, h),
        None => ("", rest),
    }
}

/// Returns the `tag` parameter of an address header.
fn tag_of(value: &str) -> Option<&str> {
    value
        .split(';')
        .skip(1)
        .find_map(|p| p.trim().strip_prefix("tag="))
}

/// A 64 bit FNV-1a hash, which derives the tag of the answering side from the
/// `Call-ID`, so that every response of one dialog carries the same tag
/// without the module keeping state.
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xCBF2_9CE4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3)
    })
}

fn branch(rng: &mut dyn RngCore) -> String {
    format!("z9hG4bK{:016x}", rng.next_u64())
}

/// The reason phrase of a status code that the module writes.
pub(crate) fn reason(code: u16) -> Option<&'static str> {
    Some(match code {
        100 => "Trying",
        180 => "Ringing",
        183 => "Session Progress",
        200 => "OK",
        404 => "Not Found",
        480 => "Temporarily Unavailable",
        486 => "Busy Here",
        488 => "Not Acceptable Here",
        603 => "Decline",
        _ => return None,
    })
}

/// Joins the start line, the headers, and the body into one message with the
/// `Content-Length` header that RFC 3261 requires over UDP.
fn assemble(start: &str, headers: &[(&str, String)], body: &str, content_type: bool) -> String {
    let mut m = String::with_capacity(start.len() + body.len() + 400);
    m.push_str(start);
    m.push_str("\r\n");
    for (n, v) in headers {
        m.push_str(n);
        m.push_str(": ");
        m.push_str(v);
        m.push_str("\r\n");
    }
    if content_type && !body.is_empty() {
        m.push_str("Content-Type: application/sdp\r\n");
    }
    m.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    m.push_str(body);
    m
}

/// Parameters of an INVITE.
pub(crate) struct InviteParams<'a> {
    /// The host of the request URI and the `To` header.
    pub server: &'a str,
    /// The address of the user agent in `Via`, `Contact`, and the SDP.
    pub client: &'a str,
    /// The audio port of the offer.
    pub rtp_port: u16,
    /// The codecs of the offer, in order of preference.
    pub codecs: Vec<&'static Codec>,
    /// The packet time in milliseconds.
    pub ptime: u32,
    /// The SIP port that `Via` and `Contact` name.
    pub sip_port: u16,
}

/// Writes an INVITE with an SDP offer.
pub(crate) fn invite(p: &InviteParams<'_>, rng: &mut dyn RngCore) -> String {
    let caller = 1000 + rng.next_u32() % 9000;
    let callee = 1000 + rng.next_u32() % 9000;
    let call_id = format!("{:016x}{:08x}@{}", rng.next_u64(), rng.next_u32(), p.client);
    let from_tag = format!("{:08x}", rng.next_u32());
    let session = rng.next_u32() >> 1;
    let lines: Vec<Offered> = p
        .codecs
        .iter()
        .map(|c| Offered { codec: c, pt: c.pt })
        .collect();
    let body = sdp::build(
        &caller.to_string(),
        p.client,
        p.rtp_port,
        &lines,
        Some(sdp::event_pt()),
        p.ptime,
        session,
    );
    let uri = format!("sip:{callee}@{}", p.server);
    let headers = [
        (
            "Via",
            format!(
                "SIP/2.0/UDP {}:{};rport;branch={}",
                p.client,
                p.sip_port,
                branch(rng)
            ),
        ),
        ("Max-Forwards", "70".to_string()),
        (
            "From",
            format!("<sip:{caller}@{}>;tag={from_tag}", p.client),
        ),
        ("To", format!("<{uri}>")),
        ("Call-ID", call_id),
        ("CSeq", "1 INVITE".to_string()),
        (
            "Contact",
            format!("<sip:{caller}@{}:{}>", p.client, p.sip_port),
        ),
        ("Allow", ALLOW.to_string()),
        ("Supported", "replaces, timer".to_string()),
        ("User-Agent", AGENT.to_string()),
    ];
    assemble(&format!("INVITE {uri} SIP/2.0"), &headers, &body, true)
}

/// Parameters of a response.
pub(crate) struct ResponseParams<'a> {
    /// The status code.
    pub code: u16,
    /// The address of the server in `Contact` and the SDP answer. Empty takes
    /// the host of the request URI.
    pub server: &'a str,
    /// The audio port of the answer, or 0 for no answer.
    pub rtp_port: u16,
    /// The codecs that the server accepts.
    pub accept: Vec<&'static Codec>,
    /// The SIP port that `Contact` names.
    pub sip_port: u16,
}

/// Selects the codec of an offer: the first one that the module knows and
/// that `accept` lists, or the first known one when `accept` is empty.
pub(crate) fn select(sdp: &Sdp, accept: &[&'static Codec]) -> Result<Offered, String> {
    sdp.offered()
        .into_iter()
        .find(|o| accept.is_empty() || accept.iter().any(|a| a.name == o.codec.name))
        .ok_or_else(|| "no codec of the offer is one that the server accepts".to_string())
}

/// Writes a response to a request, copying the headers that tie it to the
/// request and adding the tag of the answering side.
pub(crate) fn response(request: &[u8], p: &ResponseParams<'_>) -> Result<String, String> {
    let req = Message::parse(request)?;
    let method = req
        .method()
        .ok_or("the argument of 'response' is a response, not a request")?;
    let reason =
        reason(p.code).ok_or_else(|| format!("status code {} is not supported", p.code))?;
    let call_id = req.need("Call-ID")?;
    let to = req.need("To")?;
    let to = if tag_of(to).is_some() || p.code == 100 {
        to.to_string()
    } else {
        format!("{to};tag={:08x}", fnv(call_id) as u32)
    };
    let mut headers: Vec<(&str, String)> = Vec::new();
    for via in req.all("Via") {
        headers.push(("Via", via.to_string()));
    }
    headers.push(("From", req.need("From")?.to_string()));
    headers.push(("To", to));
    headers.push(("Call-ID", call_id.to_string()));
    headers.push(("CSeq", req.need("CSeq")?.to_string()));
    let mut body = String::new();
    if method == "INVITE" && (p.code == 180 || p.code == 200) {
        let (user, rhost) = user_and_host(uri_of(req.need("To")?));
        let host = if p.server.is_empty() {
            rhost.split(':').next().unwrap_or(rhost)
        } else {
            p.server
        };
        headers.push(("Contact", format!("<sip:{user}@{host}:{}>", p.sip_port)));
        if p.code == 200 {
            if p.rtp_port == 0 {
                return Err("a 200 response to an INVITE needs an 'rtp_port'".into());
            }
            let offer = Sdp::parse(req.body)?;
            let chosen = select(&offer, &p.accept)?;
            let event = offer.telephone_event();
            let ptime = offer.ptime.unwrap_or(20);
            // The answer repeats the payload type of the offer, as RFC 3264 section 6.1 asks.
            body = sdp::build(
                user,
                host,
                p.rtp_port,
                &[chosen],
                event,
                ptime,
                fnv(call_id) as u32 >> 1,
            );
            headers.push(("Allow", ALLOW.to_string()));
        }
    }
    headers.push(("Server", AGENT.to_string()));
    Ok(assemble(
        &format!("SIP/2.0 {} {reason}", p.code),
        &headers,
        &body,
        true,
    ))
}

/// Writes the ACK of a final response, or the BYE that ends a dialog that a
/// 2xx response opened.
///
/// An ACK for a 2xx response is a new transaction and gets a new branch. An
/// ACK for a response of 300 or above belongs to the INVITE transaction and
/// repeats its branch (RFC 3261 section 17.1.1.3).
pub(crate) fn in_dialog(
    response: &[u8],
    bye: bool,
    rng: &mut dyn RngCore,
) -> Result<String, String> {
    let res = Message::parse(response)?;
    let code = res
        .status()
        .ok_or("the argument must be a response, not a request")?;
    let (n, method) = res.cseq()?;
    if method != "INVITE" {
        return Err(format!("the response answers {method}, not an INVITE"));
    }
    if code < 200 {
        return Err(format!(
            "a {code} response is provisional and has no ACK or BYE"
        ));
    }
    if bye && code >= 300 {
        return Err(format!("a BYE needs a 2xx response, not {code}"));
    }
    let via = res.need("Via")?;
    let via = if bye || code < 300 {
        // Replace the branch by a fresh one and keep every other parameter.
        let mut parts = via.split(';');
        let mut out = parts.next().unwrap_or("").to_string();
        for p in parts {
            out.push(';');
            if p.starts_with("branch=") {
                out.push_str(&format!("branch={}", branch(rng)));
            } else {
                out.push_str(p);
            }
        }
        out
    } else {
        via.to_string()
    };
    let target = res
        .header("Contact")
        .map_or_else(|| uri_of(res.need("To").unwrap_or("")), uri_of);
    if !target.starts_with("sip:") {
        return Err(format!("no request URI in the response: '{target}'"));
    }
    let name = if bye { "BYE" } else { "ACK" };
    let headers = [
        ("Via", via),
        ("Max-Forwards", "70".to_string()),
        ("From", res.need("From")?.to_string()),
        ("To", res.need("To")?.to_string()),
        ("Call-ID", res.need("Call-ID")?.to_string()),
        ("CSeq", format!("{} {name}", if bye { n + 1 } else { n })),
        ("User-Agent", AGENT.to_string()),
    ];
    Ok(assemble(
        &format!("{name} {target} SIP/2.0"),
        &headers,
        "",
        false,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::parse_list;

    fn rng() -> rand_pcg::Pcg64Mcg {
        rand_pcg::Pcg64Mcg::new(42)
    }

    fn offer(codecs: &str) -> String {
        invite(
            &InviteParams {
                server: "192.0.2.20",
                client: "192.0.2.10",
                rtp_port: 16384,
                codecs: parse_list("t", codecs).unwrap(),
                ptime: 20,
                sip_port: 5060,
            },
            &mut rng(),
        )
    }

    fn answer(request: &str, code: u16) -> String {
        response(
            request.as_bytes(),
            &ResponseParams {
                code,
                server: "192.0.2.20",
                rtp_port: 20000,
                accept: parse_list("t", "PCMU,PCMA,G729,opus").unwrap(),
                sip_port: 5060,
            },
        )
        .unwrap()
    }

    #[test]
    fn the_invite_is_a_well_formed_request_with_an_offer() {
        let text = offer("opus,PCMU");
        let m = Message::parse(text.as_bytes()).unwrap();
        assert_eq!(m.method(), Some("INVITE"));
        assert!(m.start.ends_with("@192.0.2.20 SIP/2.0"), "{}", m.start);
        assert_eq!(m.cseq().unwrap(), (1, "INVITE"));
        assert_eq!(m.header("Max-Forwards"), Some("70"));
        assert!(m
            .header("Via")
            .unwrap()
            .starts_with("SIP/2.0/UDP 192.0.2.10:5060;rport;branch=z9hG4bK"));
        assert!(m.header("Call-ID").unwrap().ends_with("@192.0.2.10"));
        assert_eq!(m.header("Content-Type"), Some("application/sdp"));
        let sdp = Sdp::parse(m.body).unwrap();
        assert_eq!(sdp.port, 16384);
        assert_eq!(sdp.payloads, [111, 0, 101]);
    }

    #[test]
    fn a_response_repeats_the_request_and_adds_one_tag() {
        let req = offer("PCMU,PCMA");
        let trying = answer(&req, 100);
        let ringing = answer(&req, 180);
        let ok = answer(&req, 200);
        let r = Message::parse(req.as_bytes()).unwrap();
        for text in [&trying, &ringing, &ok] {
            let m = Message::parse(text.as_bytes()).unwrap();
            for h in ["Via", "From", "Call-ID", "CSeq"] {
                assert_eq!(m.header(h), r.header(h), "{h}");
            }
        }
        let tag = |t: &str| {
            let m = Message::parse(t.as_bytes()).unwrap();
            tag_of(m.header("To").unwrap()).map(str::to_string)
        };
        assert_eq!(tag(&trying), None);
        assert!(tag(&ringing).is_some());
        assert_eq!(tag(&ringing), tag(&ok));
        assert!(ringing.starts_with("SIP/2.0 180 Ringing\r\n"));
        let m = Message::parse(ok.as_bytes()).unwrap();
        let sdp = Sdp::parse(m.body).unwrap();
        assert_eq!(sdp.port, 20000);
        assert_eq!(sdp.payloads, [0, 101]);
        assert_eq!(select(&sdp, &[]).unwrap().codec.name, "PCMU");
    }

    #[test]
    fn the_answer_picks_the_first_offered_codec_that_the_server_accepts() {
        let req = offer("G722,G729,PCMA");
        let ok = answer(&req, 200);
        let m = Message::parse(ok.as_bytes()).unwrap();
        let sdp = Sdp::parse(m.body).unwrap();
        // G722 is not in the accept list of the test, G729 is.
        assert_eq!(sdp.payloads, [18, 101]);
        let none = response(
            offer("G722").as_bytes(),
            &ResponseParams {
                code: 200,
                server: "",
                rtp_port: 20000,
                accept: parse_list("t", "PCMU").unwrap(),
                sip_port: 5060,
            },
        );
        assert!(none.unwrap_err().contains("no codec"));
    }

    #[test]
    fn the_ack_and_the_bye_continue_the_dialog() {
        let req = offer("PCMU");
        let ok = answer(&req, 200);
        let ack = in_dialog(ok.as_bytes(), false, &mut rng()).unwrap();
        let bye = in_dialog(ok.as_bytes(), true, &mut rng()).unwrap();
        let (a, b) = (
            Message::parse(ack.as_bytes()).unwrap(),
            Message::parse(bye.as_bytes()).unwrap(),
        );
        let o = Message::parse(ok.as_bytes()).unwrap();
        assert!(a.start.starts_with("ACK sip:"));
        assert!(a.start.contains("@192.0.2.20:5060"));
        assert_eq!(a.cseq().unwrap(), (1, "ACK"));
        assert_eq!(b.cseq().unwrap(), (2, "BYE"));
        for m in [&a, &b] {
            assert_eq!(m.header("Call-ID"), o.header("Call-ID"));
            assert_eq!(m.header("From"), o.header("From"));
            assert_eq!(m.header("To"), o.header("To"));
            assert_ne!(m.header("Via"), o.header("Via"));
        }
        assert_eq!(b.method(), Some("BYE"));
        let bye_ok = answer(&bye, 200);
        let m = Message::parse(bye_ok.as_bytes()).unwrap();
        assert_eq!(m.cseq().unwrap(), (2, "BYE"));
        assert_eq!(m.header("Content-Length"), Some("0"));
        assert!(m.header("Contact").is_none());
    }

    #[test]
    fn an_ack_for_a_failure_repeats_the_branch_of_the_invite() {
        let req = offer("PCMU");
        let busy = answer(&req, 486);
        let ack = in_dialog(busy.as_bytes(), false, &mut rng()).unwrap();
        let (r, a) = (
            Message::parse(req.as_bytes()).unwrap(),
            Message::parse(ack.as_bytes()).unwrap(),
        );
        assert_eq!(a.header("Via"), r.header("Via"));
        assert_eq!(a.cseq().unwrap(), (1, "ACK"));
        assert!(in_dialog(busy.as_bytes(), true, &mut rng()).is_err());
    }

    #[test]
    fn provisional_responses_and_requests_are_refused() {
        let req = offer("PCMU");
        let ringing = answer(&req, 180);
        assert!(in_dialog(ringing.as_bytes(), false, &mut rng()).is_err());
        assert!(in_dialog(req.as_bytes(), false, &mut rng()).is_err());
        assert!(response(
            answer(&req, 200).as_bytes(),
            &ResponseParams {
                code: 200,
                server: "",
                rtp_port: 1,
                accept: vec![],
                sip_port: 5060
            }
        )
        .is_err());
    }

    #[test]
    fn a_bad_content_length_is_refused() {
        let mut text = offer("PCMU");
        text.push('x');
        assert!(Message::parse(text.as_bytes())
            .unwrap_err()
            .contains("Content-Length"));
        assert!(Message::parse(b"INVITE sip:a SIP/2.0\r\nVia x\r\n\r\n").is_err());
        assert!(Message::parse(b"no empty line").is_err());
    }

    #[test]
    fn uri_helpers_handle_brackets_and_parameters() {
        assert_eq!(uri_of("<sip:a@h:5060>;tag=x"), "sip:a@h:5060");
        assert_eq!(uri_of("sip:a@h;tag=x"), "sip:a@h");
        assert_eq!(
            user_and_host("sip:1001@192.0.2.1:5060"),
            ("1001", "192.0.2.1:5060")
        );
        assert_eq!(tag_of("<sip:a@h>;tag=abc"), Some("abc"));
        assert_eq!(tag_of("<sip:a@h>"), None);
    }
}
