// SPDX-License-Identifier: GPL-3.0-only
//! The LDAPv3 messages of the module (RFC 4511): bind, search, and unbind.

use crate::ber::{self, bad, Malformed, Reader};
use crate::filter::Filter;

// Application tags of the protocol operations (RFC 4511 section 4.2 and following).
const BIND_REQUEST: u8 = 0x60;
const BIND_RESPONSE: u8 = 0x61;
const UNBIND_REQUEST: u8 = 0x42;
const SEARCH_REQUEST: u8 = 0x63;
const SEARCH_ENTRY: u8 = 0x64;
const SEARCH_DONE: u8 = 0x65;
/// `AuthenticationChoice` simple, `[0] OCTET STRING`.
const AUTH_SIMPLE: u8 = 0x80;
/// `AuthenticationChoice` sasl, `[3] SaslCredentials`.
const AUTH_SASL: u8 = 0xa3;
/// `Controls`, `[0]` after the protocol operation.
const CONTROLS: u8 = 0xa0;

/// The largest message id (RFC 4511 `maxInt`).
pub const MAX_MESSAGE_ID: i64 = 2_147_483_647;

/// Result codes of RFC 4511 appendix A that the module sends.
pub mod code {
    /// `success`.
    pub const SUCCESS: i64 = 0;
    /// `protocolError`.
    pub const PROTOCOL_ERROR: i64 = 2;
    /// `sizeLimitExceeded`.
    pub const SIZE_LIMIT_EXCEEDED: i64 = 4;
    /// `authMethodNotSupported`.
    pub const AUTH_METHOD_NOT_SUPPORTED: i64 = 7;
    /// `noSuchObject`.
    pub const NO_SUCH_OBJECT: i64 = 32;
    /// `invalidCredentials`.
    pub const INVALID_CREDENTIALS: i64 = 49;
    /// `unwillingToPerform`.
    pub const UNWILLING_TO_PERFORM: i64 = 53;
}

/// The `LDAPResult` part of a response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LdapResult {
    /// The result code.
    pub code: i64,
    /// The matched DN.
    pub matched_dn: String,
    /// The diagnostic message.
    pub diagnostic: String,
}

/// The authentication of a bind request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    /// A simple bind with a password.
    Simple(Vec<u8>),
    /// A SASL bind with the named mechanism.
    Sasl(String),
}

/// A search request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchRequest {
    /// The base object.
    pub base: String,
    /// The scope: 0 base, 1 one level, 2 subtree.
    pub scope: i64,
    /// The alias dereferencing: 0 to 3.
    pub deref: i64,
    /// The size limit, 0 for none.
    pub size_limit: i64,
    /// The time limit in seconds, 0 for none.
    pub time_limit: i64,
    /// Whether only the attribute names are wanted.
    pub types_only: bool,
    /// The filter.
    pub filter: Filter,
    /// The requested attributes, empty for all.
    pub attributes: Vec<String>,
}

/// One attribute of an entry with its values.
pub type Attribute = (String, Vec<Vec<u8>>);

/// A search result entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The DN of the entry.
    pub dn: String,
    /// Its attributes.
    pub attrs: Vec<Attribute>,
}

/// The protocol operation of a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// A bind request.
    BindRequest {
        /// The protocol version.
        version: i64,
        /// The DN to bind as.
        name: String,
        /// The credentials.
        auth: Auth,
    },
    /// A bind response.
    BindResponse(LdapResult),
    /// An unbind request.
    UnbindRequest,
    /// A search request.
    SearchRequest(SearchRequest),
    /// A search result entry.
    SearchEntry(Entry),
    /// A search result done.
    SearchDone(LdapResult),
}

impl Op {
    /// The name that the `recv` and `field` functions use.
    pub fn name(&self) -> &'static str {
        match self {
            Op::BindRequest { .. } => "bind_request",
            Op::BindResponse(_) => "bind_response",
            Op::UnbindRequest => "unbind_request",
            Op::SearchRequest(_) => "search_request",
            Op::SearchEntry(_) => "search_entry",
            Op::SearchDone(_) => "search_done",
        }
    }
}

/// One `LDAPMessage`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// The message id.
    pub id: i64,
    /// The operation.
    pub op: Op,
}

fn string(tag: u8, s: &str) -> Vec<u8> {
    ber::octets(tag, s.as_bytes())
}

fn ldap_result(r: &LdapResult) -> Vec<Vec<u8>> {
    vec![
        ber::integer(ber::ENUMERATED, r.code),
        string(ber::OCTET_STRING, &r.matched_dn),
        string(ber::OCTET_STRING, &r.diagnostic),
    ]
}

fn read_result(r: &mut Reader<'_>) -> Result<LdapResult, Malformed> {
    let code = r.next(ber::ENUMERATED, "the result code")?.int()?;
    let matched_dn = r
        .next(ber::OCTET_STRING, "the matched DN")?
        .text()?
        .to_owned();
    let diagnostic = r
        .next(ber::OCTET_STRING, "the diagnostic message")?
        .text()?
        .to_owned();
    Ok(LdapResult {
        code,
        matched_dn,
        diagnostic,
    })
}

impl Message {
    /// Encodes the message as one BER `SEQUENCE`.
    pub fn encode(&self) -> Result<Vec<u8>, Malformed> {
        if !(0..=MAX_MESSAGE_ID).contains(&self.id) {
            return Err(bad(format!(
                "the message id {} lies outside 0 to {MAX_MESSAGE_ID}",
                self.id
            )));
        }
        let op = match &self.op {
            Op::BindRequest {
                version,
                name,
                auth,
            } => {
                let auth = match auth {
                    Auth::Simple(p) => ber::octets(AUTH_SIMPLE, p),
                    Auth::Sasl(m) => ber::constructed(AUTH_SASL, &[string(ber::OCTET_STRING, m)]),
                };
                ber::constructed(
                    BIND_REQUEST,
                    &[
                        ber::integer(ber::INTEGER, *version),
                        string(ber::OCTET_STRING, name),
                        auth,
                    ],
                )
            }
            Op::BindResponse(r) => ber::constructed(BIND_RESPONSE, &ldap_result(r)),
            Op::UnbindRequest => ber::tlv(UNBIND_REQUEST, &[]),
            Op::SearchRequest(s) => ber::constructed(
                SEARCH_REQUEST,
                &[
                    string(ber::OCTET_STRING, &s.base),
                    ber::integer(ber::ENUMERATED, s.scope),
                    ber::integer(ber::ENUMERATED, s.deref),
                    ber::integer(ber::INTEGER, s.size_limit),
                    ber::integer(ber::INTEGER, s.time_limit),
                    ber::boolean(s.types_only),
                    s.filter.encode(),
                    ber::constructed(
                        ber::SEQUENCE,
                        &s.attributes
                            .iter()
                            .map(|a| string(ber::OCTET_STRING, a))
                            .collect::<Vec<_>>(),
                    ),
                ],
            ),
            Op::SearchEntry(e) => {
                let attrs: Vec<Vec<u8>> = e
                    .attrs
                    .iter()
                    .map(|(name, values)| {
                        ber::constructed(
                            ber::SEQUENCE,
                            &[
                                string(ber::OCTET_STRING, name),
                                ber::constructed(
                                    ber::SET,
                                    &values
                                        .iter()
                                        .map(|v| ber::octets(ber::OCTET_STRING, v))
                                        .collect::<Vec<_>>(),
                                ),
                            ],
                        )
                    })
                    .collect();
                ber::constructed(
                    SEARCH_ENTRY,
                    &[
                        string(ber::OCTET_STRING, &e.dn),
                        ber::constructed(ber::SEQUENCE, &attrs),
                    ],
                )
            }
            Op::SearchDone(r) => ber::constructed(SEARCH_DONE, &ldap_result(r)),
        };
        let msg = ber::constructed(ber::SEQUENCE, &[ber::integer(ber::INTEGER, self.id), op]);
        if msg.len() > ber::MAX_LEN + 4 {
            return Err(bad(format!(
                "the message has {} octets and the module sends at most {}",
                msg.len(),
                ber::MAX_LEN + 4
            )));
        }
        Ok(msg)
    }

    /// Decodes one complete message. The buffer must hold exactly one.
    pub fn decode(buf: &[u8]) -> Result<Message, Malformed> {
        let mut top = Reader::new(buf);
        let seq = top.next(ber::SEQUENCE, "the message")?;
        top.finish("the buffer")?;
        let mut r = seq.reader();
        let id = r.next(ber::INTEGER, "the message id")?.int()?;
        if !(0..=MAX_MESSAGE_ID).contains(&id) {
            return Err(bad(format!(
                "the message id {id} lies outside 0 to {MAX_MESSAGE_ID}"
            )));
        }
        let op = r.next_any()?;
        let op = match op.tag {
            BIND_REQUEST => {
                let mut b = op.reader();
                let version = b.next(ber::INTEGER, "the version")?.int()?;
                let name = b.next(ber::OCTET_STRING, "the bind name")?.text()?.to_owned();
                let a = b.next_any()?;
                let auth = match a.tag {
                    AUTH_SIMPLE => Auth::Simple(a.content.to_vec()),
                    AUTH_SASL => {
                        let m = a
                            .reader()
                            .next(ber::OCTET_STRING, "the SASL mechanism")?
                            .text()?
                            .to_owned();
                        Auth::Sasl(m)
                    }
                    other => {
                        return Err(bad(format!(
                            "the authentication has the tag 0x{other:02x}"
                        )))
                    }
                };
                b.finish("the bind request")?;
                Op::BindRequest { version, name, auth }
            }
            BIND_RESPONSE => {
                let mut b = op.reader();
                let res = read_result(&mut b)?;
                // Referral [3] and serverSaslCreds [7] follow the result.
                while !b.is_empty() {
                    b.next_any()?;
                }
                Op::BindResponse(res)
            }
            UNBIND_REQUEST => {
                if !op.content.is_empty() {
                    return Err(bad("an unbind request has content"));
                }
                Op::UnbindRequest
            }
            SEARCH_REQUEST => {
                let mut s = op.reader();
                let base = s.next(ber::OCTET_STRING, "the base object")?.text()?.to_owned();
                let scope = s.next(ber::ENUMERATED, "the scope")?.int()?;
                let deref = s.next(ber::ENUMERATED, "the alias handling")?.int()?;
                let size_limit = s.next(ber::INTEGER, "the size limit")?.int()?;
                let time_limit = s.next(ber::INTEGER, "the time limit")?.int()?;
                let types_only = s.next(ber::BOOLEAN, "the types only flag")?.boolean()?;
                if !(0..=2).contains(&scope) || !(0..=3).contains(&deref) {
                    return Err(bad(format!(
                        "the scope {scope} or the alias handling {deref} is out of range"
                    )));
                }
                if size_limit < 0 || time_limit < 0 {
                    return Err(bad("a limit of a search request is negative"));
                }
                let filter = Filter::decode(s.next_any()?)?;
                let mut list = s.next(ber::SEQUENCE, "the attribute list")?.reader();
                let mut attributes = Vec::new();
                while !list.is_empty() {
                    attributes
                        .push(list.next(ber::OCTET_STRING, "an attribute name")?.text()?.to_owned());
                }
                s.finish("the search request")?;
                Op::SearchRequest(SearchRequest {
                    base,
                    scope,
                    deref,
                    size_limit,
                    time_limit,
                    types_only,
                    filter,
                    attributes,
                })
            }
            SEARCH_ENTRY => {
                let mut s = op.reader();
                let dn = s.next(ber::OCTET_STRING, "the object name")?.text()?.to_owned();
                let mut list = s.next(ber::SEQUENCE, "the attribute list")?.reader();
                s.finish("the search result entry")?;
                let mut attrs = Vec::new();
                while !list.is_empty() {
                    let mut a = list.next(ber::SEQUENCE, "an attribute")?.reader();
                    let name = a.next(ber::OCTET_STRING, "an attribute type")?.text()?.to_owned();
                    let mut set = a.next(ber::SET, "the values")?.reader();
                    a.finish("an attribute")?;
                    let mut values = Vec::new();
                    while !set.is_empty() {
                        values.push(set.next(ber::OCTET_STRING, "a value")?.content.to_vec());
                    }
                    attrs.push((name, values));
                }
                Op::SearchEntry(Entry { dn, attrs })
            }
            SEARCH_DONE => {
                let mut b = op.reader();
                let res = read_result(&mut b)?;
                while !b.is_empty() {
                    b.next_any()?;
                }
                Op::SearchDone(res)
            }
            other => {
                return Err(bad(format!(
                    "the protocol operation has the tag 0x{other:02x}, which this module does not handle"
                )))
            }
        };
        if r.peek_tag() == Some(CONTROLS) {
            r.next_any()?;
        }
        r.finish("the message")?;
        Ok(Message { id, op })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search() -> SearchRequest {
        SearchRequest {
            base: "dc=example,dc=test".into(),
            scope: 2,
            deref: 0,
            size_limit: 0,
            time_limit: 0,
            types_only: false,
            filter: Filter::parse("(uid=jdoe)").unwrap(),
            attributes: vec!["cn".into(), "mail".into()],
        }
    }

    #[test]
    fn a_simple_bind_request_is_the_known_byte_string() {
        // RFC 4511 style example: version 3, name "cn=a", password "pw".
        let m = Message {
            id: 1,
            op: Op::BindRequest {
                version: 3,
                name: "cn=a".into(),
                auth: Auth::Simple(b"pw".to_vec()),
            },
        };
        let want = [
            0x30, 0x12, 0x02, 0x01, 0x01, 0x60, 0x0d, 0x02, 0x01, 0x03, 0x04, 0x04, b'c', b'n',
            b'=', b'a', 0x80, 0x02, b'p', b'w',
        ];
        assert_eq!(m.encode().unwrap(), want);
        assert_eq!(Message::decode(&want).unwrap(), m);
    }

    #[test]
    fn a_successful_bind_response_and_an_unbind_are_the_known_byte_strings() {
        let ok = Message {
            id: 1,
            op: Op::BindResponse(LdapResult {
                code: 0,
                matched_dn: String::new(),
                diagnostic: String::new(),
            }),
        };
        assert_eq!(
            ok.encode().unwrap(),
            [0x30, 0x0c, 0x02, 0x01, 0x01, 0x61, 0x07, 0x0a, 0x01, 0x00, 0x04, 0x00, 0x04, 0x00]
        );
        let unbind = Message {
            id: 3,
            op: Op::UnbindRequest,
        };
        assert_eq!(
            unbind.encode().unwrap(),
            [0x30, 0x05, 0x02, 0x01, 0x03, 0x42, 0x00]
        );
    }

    #[test]
    fn every_operation_round_trips() {
        let result = LdapResult {
            code: code::INVALID_CREDENTIALS,
            matched_dn: "dc=example,dc=test".into(),
            diagnostic: "invalid credentials".into(),
        };
        let msgs = [
            Message {
                id: 0,
                op: Op::BindRequest {
                    version: 3,
                    name: String::new(),
                    auth: Auth::Simple(Vec::new()),
                },
            },
            Message {
                id: 7,
                op: Op::BindRequest {
                    version: 3,
                    name: "cn=a".into(),
                    auth: Auth::Sasl("GSSAPI".into()),
                },
            },
            Message {
                id: 2,
                op: Op::BindResponse(result.clone()),
            },
            Message {
                id: 4,
                op: Op::SearchRequest(search()),
            },
            Message {
                id: 4,
                op: Op::SearchEntry(Entry {
                    dn: "uid=jdoe,ou=People,dc=example,dc=test".into(),
                    attrs: vec![
                        ("cn".into(), vec![b"John Doe".to_vec()]),
                        (
                            "mail".into(),
                            vec![b"a@x.test".to_vec(), b"b@x.test".to_vec()],
                        ),
                        ("description".into(), vec![]),
                    ],
                }),
            },
            Message {
                id: 4,
                op: Op::SearchDone(result),
            },
            Message {
                id: MAX_MESSAGE_ID,
                op: Op::UnbindRequest,
            },
        ];
        for m in msgs {
            let enc = m.encode().unwrap();
            assert_eq!(Message::decode(&enc).unwrap(), m, "{}", m.op.name());
        }
    }

    #[test]
    fn a_long_message_uses_the_long_length_form() {
        let e = Message {
            id: 5,
            op: Op::SearchEntry(Entry {
                dn: "uid=x".into(),
                attrs: vec![("description".into(), vec![vec![b'd'; 400]])],
            }),
        };
        let enc = e.encode().unwrap();
        assert_eq!(&enc[..2], &[0x30, 0x82]);
        assert_eq!(Message::decode(&enc).unwrap(), e);
        let too_long = Message {
            id: 5,
            op: Op::SearchEntry(Entry {
                dn: "uid=x".into(),
                attrs: vec![("description".into(), vec![vec![b'd'; 70_000]])],
            }),
        };
        assert!(too_long.encode().is_err());
    }

    #[test]
    fn controls_after_the_operation_are_skipped() {
        let mut enc = Message {
            id: 3,
            op: Op::UnbindRequest,
        }
        .encode()
        .unwrap();
        enc.extend_from_slice(&[0xa0, 0x00]);
        enc[1] += 2;
        assert_eq!(Message::decode(&enc).unwrap().op, Op::UnbindRequest);
    }

    #[test]
    fn the_decoder_rejects_what_it_cannot_trust() {
        let ok = Message {
            id: 3,
            op: Op::UnbindRequest,
        }
        .encode()
        .unwrap();
        let mut trailing = ok.clone();
        trailing.push(0);
        let cases: Vec<(Vec<u8>, &str)> = vec![
            (vec![], "empty"),
            (ok[..ok.len() - 1].to_vec(), "cut off"),
            (trailing, "trailing octet"),
            (
                vec![0x31, 0x05, 0x02, 0x01, 0x03, 0x42, 0x00],
                "wrong outer tag",
            ),
            (
                vec![0x30, 0x05, 0x04, 0x01, 0x03, 0x42, 0x00],
                "id is not an integer",
            ),
            (
                vec![0x30, 0x08, 0x02, 0x04, 0x80, 0, 0, 0, 0x42, 0x00],
                "negative id",
            ),
            (
                vec![0x30, 0x08, 0x02, 0x04, 0x7f, 0xff, 0xff, 0xff],
                "no operation",
            ),
            (
                vec![0x30, 0x06, 0x02, 0x01, 0x03, 0x42, 0x01, 0x00],
                "unbind with content",
            ),
            (
                vec![0x30, 0x05, 0x02, 0x01, 0x03, 0x50, 0x00],
                "unknown operation",
            ),
        ];
        for (bytes, why) in cases {
            assert!(Message::decode(&bytes).is_err(), "{why}");
        }
        let id = Message {
            id: -1,
            op: Op::UnbindRequest,
        };
        assert!(id.encode().is_err());
        let id = Message {
            id: MAX_MESSAGE_ID + 1,
            op: Op::UnbindRequest,
        };
        assert!(id.encode().is_err());
        // A scope outside 0 to 2.
        let mut s = search();
        s.scope = 3;
        let enc = Message {
            id: 1,
            op: Op::SearchRequest(s),
        }
        .encode()
        .unwrap();
        assert!(Message::decode(&enc).unwrap_err().0.contains("scope"));
    }
}
