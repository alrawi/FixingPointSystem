// SPDX-License-Identifier: GPL-3.0-only
//! The mail client profile and the server profiles with an explicit ALPN policy.
//!
//! The browser profiles of [`crate::browser`] offer the ALPN ids `h2` and
//! `http/1.1`, which a mail client never sends, and the server profiles of the
//! agent select `http/1.1` only. This module adds the missing three:
//!
//! | Name | Side | ALPN |
//! |---|---|---|
//! | [`MAIL_CLIENT`] | client | none, the ClientHello has no `application_layer_protocol_negotiation` extension |
//! | [`SERVER_TLS13_H2`] | server | prefers `h2` and falls back to `http/1.1` |
//! | [`SERVER_TLS13_NOALPN`] | server | none, the server accepts any offer and selects nothing |
//!
//! Both servers accept TLS 1.3 only. Every profile here lists the cipher
//! suites, groups, and signature schemes of Firefox 139, and Thunderbird shares
//! its TLS library, so the mail client differs from `firefox-139` in the ALPN
//! list alone. The same gaps as in [`crate::browser`] apply to its JA4, and the
//! ALPN extension is the only difference between the two ClientHellos.
//!
//! The server profiles issue no tickets, as `server-tls13` of the agent does,
//! so a connector that several sessions share never resumes and keeps the
//! JA4 of its profile. A caller that builds one connector per session sets
//! `resumption` to [`ServerResumption::Tickets`] on the value it gets here.

use crate::browser;
use crate::profile::{ClientProfile, ServerProfile, ServerResumption, TlsVersion};

/// Name of the mail client profile.
pub const MAIL_CLIENT: &str = "mail-client";

/// Name of the TLS 1.3 server profile that selects `h2` when the client offers it.
pub const SERVER_TLS13_H2: &str = "server-tls13-h2";

/// Name of the TLS 1.3 server profile that selects no application protocol.
pub const SERVER_TLS13_NOALPN: &str = "server-tls13-noalpn";

/// Returns the client profile of a mail client such as Thunderbird, which offers
/// no ALPN id. It follows `firefox-139` in everything else.
pub fn mail_client() -> ClientProfile {
    let firefox = browser::find("firefox-139").map(|b| b.client_profile());
    ClientProfile {
        name: MAIL_CLIENT.into(),
        alpn: Vec::new(),
        ..firefox.unwrap_or_default()
    }
}

/// Returns the server profile [`SERVER_TLS13_H2`]. A client that offers `h2`
/// gets it, a client that offers only `http/1.1` gets that, and a client
/// without ALPN gets none.
pub fn server_tls13_h2() -> ServerProfile {
    ServerProfile {
        name: SERVER_TLS13_H2.into(),
        versions: vec![TlsVersion::Tls13],
        alpn: vec![b"h2".to_vec(), b"http/1.1".to_vec()],
        resumption: ServerResumption::Disabled,
        ..ServerProfile::default()
    }
}

/// Returns the server profile [`SERVER_TLS13_NOALPN`], which completes the
/// handshake of a client that offers ALPN ids and selects none of them.
pub fn server_tls13_noalpn() -> ServerProfile {
    ServerProfile {
        name: SERVER_TLS13_NOALPN.into(),
        versions: vec![TlsVersion::Tls13],
        alpn: Vec::new(),
        resumption: ServerResumption::Disabled,
        ..ServerProfile::default()
    }
}

/// Finds one of the new server profiles by name.
pub fn server_profile(name: &str) -> Option<ServerProfile> {
    match name {
        SERVER_TLS13_H2 => Some(server_tls13_h2()),
        SERVER_TLS13_NOALPN => Some(server_tls13_noalpn()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mail_client_is_firefox_without_alpn() {
        let mail = mail_client();
        let firefox = browser::find("firefox-139")
            .map(|b| b.client_profile())
            .unwrap_or_default();
        assert_eq!(mail.name, MAIL_CLIENT);
        assert!(mail.alpn.is_empty());
        assert!(!firefox.alpn.is_empty());
        assert_eq!(mail.cipher_suites, firefox.cipher_suites);
        assert_eq!(mail.groups, firefox.groups);
        assert_eq!(mail.sig_schemes, firefox.sig_schemes);
        assert_eq!(mail.versions, firefox.versions);
    }

    #[test]
    fn server_profiles_carry_their_names_and_alpn_policy() {
        for name in [SERVER_TLS13_H2, SERVER_TLS13_NOALPN] {
            let p = server_profile(name).map(|p| (p.name, p.versions, p.alpn));
            let (n, versions, alpn) = p.unwrap_or_default();
            assert_eq!(n, name);
            assert_eq!(versions, [TlsVersion::Tls13]);
            assert_eq!(alpn.is_empty(), name == SERVER_TLS13_NOALPN);
        }
        assert!(server_profile("server-tls13").is_none());
    }
}
