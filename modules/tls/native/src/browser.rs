// SPDX-License-Identifier: GPL-3.0-only
//! Browser like client profiles and the JA4 fingerprints that they produce.
//!
//! Each [`BrowserProfile`] pairs a [`ClientProfile`] with two JA4 strings.
//! `expected_ja4` is the fingerprint that this implementation puts on the wire
//! for a TLS 1.3 and 1.2 handshake with the server name `tls.example.test`, and
//! the CI test `modules/tls/tests/ja4.rs` asserts it against the bytes of a real
//! ClientHello. `browser_ja4` is the fingerprint of the real browser as public
//! captures document it. The two differ, and [`BrowserProfile::gaps`] names every
//! reason, because rustls with the ring provider cannot reproduce a browser
//! ClientHello:
//!
//! * It offers only the nine AEAD suites of ECDHE that ring provides. A browser
//!   also offers `ECDHE_*_AES_*_CBC_SHA` and the RSA key exchange suites, and
//!   JA4 hashes the sorted suite list, so the cipher hash of every browser
//!   differs from ours.
//! * It adds no GREASE values, no `signed_certificate_timestamp` (0x0012), no
//!   `application_settings` (0x4469 or 0x44cd), no `renegotiation_info` (0xff01,
//!   rustls signals it with the `TLS_EMPTY_RENEGOTIATION_INFO_SCSV` suite), no
//!   `delegated_credential` (0x0022), no `record_size_limit` (0x001c), and no
//!   `encrypted_client_hello` (0xfe0d, the ring provider has no HPKE). It sends
//!   `compress_certificate` (0x001b) only with a decompressor, which the crate
//!   does not carry.
//! * It encodes the extensions in a random order per connection, which JA4
//!   hides by sorting and the unsorted form JA4_o shows.
//!
//! The fields that a profile does control are the cipher suite order, the group
//! order, the signature scheme order, the ALPN list, the versions, and SNI.

use crate::profile::{CipherSuite, ClientProfile, Group, SigScheme, TlsVersion};

/// A client profile of a browser with its documented and its actual JA4.
#[derive(Debug, Clone, Copy)]
pub struct BrowserProfile {
    /// Name of the profile, which is also the name of the [`ClientProfile`].
    pub name: &'static str,
    /// The browser and version of the public captures that the profile follows.
    pub browser: &'static str,
    /// The JA4 that this implementation produces, which CI asserts.
    pub expected_ja4: &'static str,
    /// The JA4 of the real browser, for comparison. It differs from `expected_ja4`.
    pub browser_ja4: &'static str,
    /// Public references for the field orders, as URLs or document names.
    pub sources: &'static [&'static str],
    /// What the profile cannot reproduce, one precise statement per entry.
    pub gaps: &'static [&'static str],
    build: fn() -> ClientProfile,
}

impl BrowserProfile {
    /// Builds the client profile.
    pub fn client_profile(&self) -> ClientProfile {
        (self.build)()
    }
}

/// The cipher suites of rustls that the browsers also offer, in the order of
/// Chrome and Edge: TLS 1.3 AES-128, AES-256, ChaCha20, then ECDHE with AES-128,
/// ECDSA before RSA, AES-256, then ChaCha20.
const CHROME_SUITES: [CipherSuite; 9] = [
    CipherSuite::Tls13Aes128GcmSha256,
    CipherSuite::Tls13Aes256GcmSha384,
    CipherSuite::Tls13Chacha20Poly1305Sha256,
    CipherSuite::Tls12EcdheEcdsaAes128GcmSha256,
    CipherSuite::Tls12EcdheRsaAes128GcmSha256,
    CipherSuite::Tls12EcdheEcdsaAes256GcmSha384,
    CipherSuite::Tls12EcdheRsaAes256GcmSha384,
    CipherSuite::Tls12EcdheEcdsaChacha20Poly1305Sha256,
    CipherSuite::Tls12EcdheRsaChacha20Poly1305Sha256,
];

/// Firefox lists ChaCha20 before AES-256 in TLS 1.3 and puts the ChaCha20 suites
/// of TLS 1.2 before the AES-256 suites.
const FIREFOX_SUITES: [CipherSuite; 9] = [
    CipherSuite::Tls13Aes128GcmSha256,
    CipherSuite::Tls13Chacha20Poly1305Sha256,
    CipherSuite::Tls13Aes256GcmSha384,
    CipherSuite::Tls12EcdheEcdsaAes128GcmSha256,
    CipherSuite::Tls12EcdheRsaAes128GcmSha256,
    CipherSuite::Tls12EcdheEcdsaChacha20Poly1305Sha256,
    CipherSuite::Tls12EcdheRsaChacha20Poly1305Sha256,
    CipherSuite::Tls12EcdheEcdsaAes256GcmSha384,
    CipherSuite::Tls12EcdheRsaAes256GcmSha384,
];

/// Safari lists the AES-256 suites before the AES-128 suites in TLS 1.2 and puts
/// the ECDSA ChaCha20 suite before the RSA AES-256 suite.
const SAFARI_SUITES: [CipherSuite; 9] = [
    CipherSuite::Tls13Aes128GcmSha256,
    CipherSuite::Tls13Aes256GcmSha384,
    CipherSuite::Tls13Chacha20Poly1305Sha256,
    CipherSuite::Tls12EcdheEcdsaAes256GcmSha384,
    CipherSuite::Tls12EcdheEcdsaAes128GcmSha256,
    CipherSuite::Tls12EcdheEcdsaChacha20Poly1305Sha256,
    CipherSuite::Tls12EcdheRsaAes256GcmSha384,
    CipherSuite::Tls12EcdheRsaAes128GcmSha256,
    CipherSuite::Tls12EcdheRsaChacha20Poly1305Sha256,
];

/// The signature algorithms of Chrome and Edge, all of which ring supports.
const CHROME_SCHEMES: [SigScheme; 8] = [
    SigScheme::EcdsaP256Sha256,
    SigScheme::RsaPssSha256,
    SigScheme::RsaPkcs1Sha256,
    SigScheme::EcdsaP384Sha384,
    SigScheme::RsaPssSha384,
    SigScheme::RsaPkcs1Sha384,
    SigScheme::RsaPssSha512,
    SigScheme::RsaPkcs1Sha512,
];

/// The signature algorithms of Firefox that ring supports. Firefox also lists
/// `ecdsa_secp521r1_sha512` (0x0603), `ed448` (0x0808), the RSA PSS schemes with
/// PSS keys (0x0809 to 0x080b), and the SHA-1 schemes 0x0203 and 0x0201.
const FIREFOX_SCHEMES: [SigScheme; 9] = [
    SigScheme::EcdsaP256Sha256,
    SigScheme::EcdsaP384Sha384,
    SigScheme::Ed25519,
    SigScheme::RsaPssSha256,
    SigScheme::RsaPssSha384,
    SigScheme::RsaPssSha512,
    SigScheme::RsaPkcs1Sha256,
    SigScheme::RsaPkcs1Sha384,
    SigScheme::RsaPkcs1Sha512,
];

/// The signature algorithms of Safari that ring supports. Safari also lists the
/// SHA-1 schemes 0x0203 and 0x0201.
const SAFARI_SCHEMES: [SigScheme; 8] = [
    SigScheme::EcdsaP256Sha256,
    SigScheme::RsaPssSha256,
    SigScheme::RsaPkcs1Sha256,
    SigScheme::EcdsaP384Sha384,
    SigScheme::RsaPssSha384,
    SigScheme::RsaPkcs1Sha384,
    SigScheme::RsaPssSha512,
    SigScheme::RsaPkcs1Sha512,
];

/// Every browser lists X25519 first, then P-256 and P-384. Rustls with ring has
/// no X25519MLKEM768, which Chrome, Edge, and Firefox send first, and no P-521
/// or FFDHE groups. JA4 does not read the groups, so the group list affects the
/// wire form and JA4_o only through the order of the supported_groups values.
const GROUPS: [Group; 3] = [Group::X25519, Group::Secp256r1, Group::Secp384r1];

fn alpn() -> Vec<Vec<u8>> {
    vec![b"h2".to_vec(), b"http/1.1".to_vec()]
}

fn chrome() -> ClientProfile {
    ClientProfile {
        name: "chrome-136".into(),
        versions: vec![TlsVersion::Tls13, TlsVersion::Tls12],
        alpn: alpn(),
        groups: GROUPS.to_vec(),
        cipher_suites: CHROME_SUITES.to_vec(),
        sig_schemes: CHROME_SCHEMES.to_vec(),
        ..ClientProfile::default()
    }
}

fn edge() -> ClientProfile {
    ClientProfile {
        name: "edge-134".into(),
        ..chrome()
    }
}

fn firefox() -> ClientProfile {
    ClientProfile {
        name: "firefox-139".into(),
        versions: vec![TlsVersion::Tls13, TlsVersion::Tls12],
        alpn: alpn(),
        groups: GROUPS.to_vec(),
        cipher_suites: FIREFOX_SUITES.to_vec(),
        sig_schemes: FIREFOX_SCHEMES.to_vec(),
        ..ClientProfile::default()
    }
}

fn safari() -> ClientProfile {
    ClientProfile {
        name: "safari-18.5".into(),
        versions: vec![TlsVersion::Tls13, TlsVersion::Tls12],
        alpn: alpn(),
        groups: GROUPS.to_vec(),
        cipher_suites: SAFARI_SUITES.to_vec(),
        sig_schemes: SAFARI_SCHEMES.to_vec(),
        ..ClientProfile::default()
    }
}

const FOXIO_JA4: &str = "https://github.com/FoxIO-LLC/ja4/blob/main/technical_details/JA4.md";
const WREQ_CHROME: &str = "https://docs.rs/crate/wreq-util/2.2.6/source/tests/emulation_chrome.rs";
const WREQ_FIREFOX: &str =
    "https://docs.rs/crate/wreq-util/2.2.6/source/tests/emulation_firefox.rs";
const WREQ_SAFARI: &str = "https://docs.rs/crate/wreq-util/2.2.6/source/tests/emulation_safari.rs";
const REALISM_TEST: &str =
    "realism/tests/test_ja4.py, the ClientHello of the FoxIO worked example (Chrome of 2023)";

/// Gaps that every profile shares, in the words of the wiki page.
const COMMON_GAPS: [&str; 4] = [
    "Cipher suites: only the nine ECDHE AEAD suites of ring plus the renegotiation SCSV (0x00ff) are offered, so the browser's CBC (0xc009, 0xc00a, 0xc013, 0xc014, 0xc008, 0xc012, 0x000a) and RSA key exchange suites (0x009c, 0x009d, 0x002f, 0x0035) are missing and the JA4 cipher count and hash differ",
    "GREASE: rustls adds no GREASE cipher, extension, group, or version value, which JA4 ignores, but the wire form and JA3 differ",
    "Extensions: rustls sends 0x0000, 0x0005, 0x000a, 0x000b, 0x000d, 0x0010, 0x0017, 0x0023, 0x002b, 0x002d, and 0x0033, so every extension that a browser sends beyond these changes the extension count and hash of JA4",
    "Extension order: rustls shuffles the extensions per connection and a browser shuffles them too (Chrome since 110) or keeps a fixed order (Firefox, Safari), so JA4_o differs, and JA4 does not",
];

/// The four browser profiles, in the order Chrome, Edge, Firefox, Safari.
pub const BROWSER_PROFILES: [BrowserProfile; 4] = [
    BrowserProfile {
        name: "chrome-136",
        browser: "Chrome 136 on desktop, the newest of Chrome 132 to 137 that share one JA4",
        expected_ja4: "t13d1011h2_61a7ad8aa9b6_6d021c4c45cd",
        browser_ja4: "t13d1516h2_8daaf6152771_d8a2da3f94cd",
        sources: &[
            FOXIO_JA4,
            WREQ_CHROME,
            REALISM_TEST,
            "cipher, extension, and signature algorithm order of Chrome: the FoxIO worked example above",
        ],
        gaps: &[
            COMMON_GAPS[0],
            COMMON_GAPS[1],
            COMMON_GAPS[2],
            COMMON_GAPS[3],
            "Chrome also sends 0x0012 (signed_certificate_timestamp), 0x001b (compress_certificate, brotli), 0x44cd (application_settings), 0xfe0d (ECH GREASE), and 0xff01 (renegotiation_info), which rustls with ring cannot send",
            "Groups: Chrome 136 sends X25519MLKEM768 (0x11ec) first, which the ring provider lacks",
        ],
        build: chrome,
    },
    BrowserProfile {
        name: "edge-134",
        browser: "Edge 134 on desktop, a Chromium build with the extension set of Chrome 131",
        expected_ja4: "t13d1011h2_61a7ad8aa9b6_6d021c4c45cd",
        browser_ja4: "t13d1516h2_8daaf6152771_02713d6af862",
        sources: &[
            FOXIO_JA4,
            WREQ_CHROME,
            "field orders: the same as Chrome, because Edge uses the Chromium TLS stack",
        ],
        gaps: &[
            COMMON_GAPS[0],
            COMMON_GAPS[1],
            COMMON_GAPS[2],
            COMMON_GAPS[3],
            "Edge 134 has another extension hash than Chrome 136 in the sources (02713d6af862 against d8a2da3f94cd), in part c (extensions and signature algorithms), and rustls cannot send either extension set",
            "Edge and Chrome differ in JA4 only by the extension hash, and this implementation cannot produce either, so the two profiles have the same expected JA4",
        ],
        build: edge,
    },
    BrowserProfile {
        name: "firefox-139",
        browser: "Firefox 139 on desktop, the newest of Firefox 135 to 139 that share one JA4",
        expected_ja4: "t13d1011h2_61a7ad8aa9b6_f674693fb0dc",
        browser_ja4: "t13d1717h2_5b57614c22b0_3cbfd9057e0d",
        sources: &[
            FOXIO_JA4,
            WREQ_FIREFOX,
            "cipher and signature algorithm order of Firefox: public ClientHello captures of Firefox 13x (tls.peet.ws and ja4db listings), recalled and not re-fetched in this session",
        ],
        gaps: &[
            COMMON_GAPS[0],
            COMMON_GAPS[1],
            COMMON_GAPS[2],
            COMMON_GAPS[3],
            "Firefox also sends 0x0012, 0x001b, 0x001c (record_size_limit), 0x0022 (delegated_credential), 0xfe0d, and 0xff01, and offers the groups X25519MLKEM768, P-521, and FFDHE, none of which rustls with ring can send",
            "Signature algorithms: 0x0603, 0x0808, 0x0809, 0x080a, 0x080b, 0x0203, and 0x0201 are missing",
        ],
        build: firefox,
    },
    BrowserProfile {
        name: "safari-18.5",
        browser: "Safari 18.5 on macOS, the newest of Safari 18.2 to 18.5 that share one JA4",
        expected_ja4: "t13d1011h2_61a7ad8aa9b6_6d021c4c45cd",
        browser_ja4: "t13d2014h2_a09f3c656075_e42f34c56612",
        sources: &[
            FOXIO_JA4,
            WREQ_SAFARI,
            "cipher and signature algorithm order of Safari: public ClientHello captures of Safari 17 and 18 (tls.peet.ws and ja4db listings), recalled and not re-fetched in this session",
        ],
        gaps: &[
            COMMON_GAPS[0],
            COMMON_GAPS[1],
            COMMON_GAPS[2],
            COMMON_GAPS[3],
            "Safari also sends 0x0012, 0x001b (zlib), and 0xff01, and its list holds 20 suites, so the cipher count of part a is 10 here and 20 in Safari",
            "Signature algorithms: the SHA-1 schemes 0x0203 and 0x0201 are missing, which leaves the same list as Chrome",
        ],
        build: safari,
    },
];

/// Finds a browser profile by name, for example `chrome-136`.
pub fn find(name: &str) -> Option<&'static BrowserProfile> {
    BROWSER_PROFILES.iter().find(|p| p.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_names_match_their_client_profiles() {
        for p in &BROWSER_PROFILES {
            assert_eq!(p.client_profile().name, p.name);
            assert_eq!(find(p.name).map(|f| f.name), Some(p.name));
            assert!(!p.sources.is_empty() && !p.gaps.is_empty());
        }
    }

    #[test]
    fn expected_ja4_has_three_parts_and_differs_from_the_browser() {
        for p in &BROWSER_PROFILES {
            for ja4 in [p.expected_ja4, p.browser_ja4] {
                let parts: Vec<_> = ja4.split('_').collect();
                assert_eq!(parts.len(), 3, "{}: {ja4}", p.name);
                assert_eq!(
                    (parts[0].len(), parts[1].len(), parts[2].len()),
                    (10, 12, 12)
                );
            }
            assert_ne!(
                p.expected_ja4, p.browser_ja4,
                "{}: the gap list says rustls differs",
                p.name
            );
        }
    }

    #[test]
    fn signature_scheme_codes_are_the_iana_values() {
        let codes: Vec<u16> = CHROME_SCHEMES.iter().map(|s| s.code()).collect();
        assert_eq!(
            codes,
            [0x0403, 0x0804, 0x0401, 0x0503, 0x0805, 0x0501, 0x0806, 0x0601]
        );
        assert_eq!(SigScheme::Ed25519.code(), 0x0807);
    }
}
