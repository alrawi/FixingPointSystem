// SPDX-License-Identifier: GPL-3.0-only
//! The certificate authority of one run and the server certificates it issues.

use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256, PKCS_ECDSA_P384_SHA384, PKCS_ED25519,
    PKCS_RSA_SHA256,
};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use rustls::RootCertStore;

use crate::error::TlsError;

/// Key type of an issued server certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeafKey {
    /// ECDSA with P-256 and SHA-256, the default of web servers today.
    EcdsaP256,
    /// ECDSA with P-384 and SHA-384.
    EcdsaP384,
    /// Ed25519.
    Ed25519,
    /// An RSA key that the caller generated, as a PKCS#8 PEM string. The ring
    /// provider cannot generate RSA keys, so a caller makes one with
    /// `openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048`, as spike S2 does.
    RsaPem(String),
}

/// A server certificate chain with its private key.
pub struct ServerIdentity {
    chain: Vec<CertificateDer<'static>>,
    key: PrivatePkcs8KeyDer<'static>,
}

impl ServerIdentity {
    /// Returns the chain, leaf first and authority last.
    pub fn chain(&self) -> &[CertificateDer<'static>] {
        &self.chain
    }

    pub(crate) fn key(&self) -> PrivatePkcs8KeyDer<'static> {
        self.key.clone_key()
    }
}

impl Clone for ServerIdentity {
    fn clone(&self) -> Self {
        Self {
            chain: self.chain.clone(),
            key: self.key.clone_key(),
        }
    }
}

impl std::fmt::Debug for ServerIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The private key stays out of debug output.
        f.debug_struct("ServerIdentity")
            .field("certificates", &self.chain.len())
            .finish_non_exhaustive()
    }
}

/// A certificate authority that exists for one run.
///
/// The authority generates an ECDSA P-256 key and a self signed certificate
/// when it is created, and it signs every server certificate of the run.
/// Clients trust only [`RunAuthority::root_store`], so a run never depends on
/// a certificate that lives outside it. Key generation draws from the system
/// random source, which is the one edge of the module that no seed controls.
pub struct RunAuthority {
    ca: Certificate,
    key: KeyPair,
}

fn authority_error(step: &'static str) -> impl FnOnce(rcgen::Error) -> TlsError {
    move |source| TlsError::Authority { step, source }
}

impl RunAuthority {
    /// Generates an authority whose common name is `common_name`.
    pub fn generate(common_name: &str) -> Result<Self, TlsError> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
            .map_err(authority_error("generate the authority key"))?;
        Self::with_key(common_name, key)
    }

    /// Builds an authority around an ECDSA P-256 key that the caller made, as
    /// a PKCS#8 document.
    ///
    /// Two authorities with the same key and the same `common_name` have the
    /// same subject and the same public key, so a client that trusts one
    /// verifies a certificate that the other issued, although their
    /// self signed certificates differ in the bytes of the signature. A run
    /// with several agents uses this to share one trust anchor without
    /// shipping a certificate: every agent derives the key from the run seed
    /// (AGT-14). The key then protects nothing, so only a test network uses
    /// such an authority.
    pub fn from_pkcs8_der(common_name: &str, pkcs8_der: &[u8]) -> Result<Self, TlsError> {
        let der = PrivatePkcs8KeyDer::from(pkcs8_der);
        let key = KeyPair::from_pkcs8_der_and_sign_algo(&der, &PKCS_ECDSA_P256_SHA256)
            .map_err(authority_error("read the authority key"))?;
        Self::with_key(common_name, key)
    }

    fn with_key(common_name: &str, key: KeyPair) -> Result<Self, TlsError> {
        let mut params = CertificateParams::new(Vec::<String>::new())
            .map_err(authority_error("create the authority parameters"))?;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params
            .distinguished_name
            .push(DnType::CommonName, common_name);
        let ca = params
            .self_signed(&key)
            .map_err(authority_error("sign the authority certificate"))?;
        Ok(Self { ca, key })
    }

    /// Issues a server certificate for the DNS names `names`.
    ///
    /// The first name becomes the common name. The chain holds the leaf and
    /// the authority certificate, as the chain of spike S2 does.
    pub fn issue(&self, names: &[&str], key: LeafKey) -> Result<ServerIdentity, TlsError> {
        let Some(first) = names.first() else {
            return Err(TlsError::Config(
                "a server certificate needs at least one name".into(),
            ));
        };
        let leaf_key = match key {
            LeafKey::EcdsaP256 => KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256),
            LeafKey::EcdsaP384 => KeyPair::generate_for(&PKCS_ECDSA_P384_SHA384),
            LeafKey::Ed25519 => KeyPair::generate_for(&PKCS_ED25519),
            LeafKey::RsaPem(pem) => KeyPair::from_pem_and_sign_algo(&pem, &PKCS_RSA_SHA256),
        }
        .map_err(authority_error("generate the server key"))?;
        let mut params =
            CertificateParams::new(names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>())
                .map_err(authority_error("create the server certificate parameters"))?;
        params.distinguished_name.push(DnType::CommonName, *first);
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let leaf = params
            .signed_by(&leaf_key, &self.ca, &self.key)
            .map_err(authority_error("sign the server certificate"))?;
        Ok(ServerIdentity {
            chain: vec![leaf.der().clone(), self.ca.der().clone()],
            key: PrivatePkcs8KeyDer::from(leaf_key.serialize_der()),
        })
    }

    /// Returns a root store that trusts only this authority.
    pub fn root_store(&self) -> Result<RootCertStore, TlsError> {
        let mut roots = RootCertStore::empty();
        roots
            .add(self.ca.der().clone())
            .map_err(|e| TlsError::Config(format!("add the authority to the root store: {e}")))?;
        Ok(roots)
    }

    /// Returns the authority certificate in PEM form, for a capture check or a debugger.
    pub fn ca_pem(&self) -> String {
        self.ca.pem()
    }
}

impl std::fmt::Debug for RunAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunAuthority").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issues_every_generated_key_type_and_rejects_empty_names() {
        let ca = RunAuthority::generate("test authority").unwrap();
        for key in [LeafKey::EcdsaP256, LeafKey::EcdsaP384, LeafKey::Ed25519] {
            let id = ca
                .issue(&["a.example.test", "b.example.test"], key)
                .unwrap();
            assert_eq!(id.chain().len(), 2);
        }
        assert!(matches!(
            ca.issue(&[], LeafKey::EcdsaP256),
            Err(TlsError::Config(_))
        ));
        assert!(ca.ca_pem().starts_with("-----BEGIN CERTIFICATE-----"));
        assert_eq!(ca.root_store().unwrap().len(), 1);
    }

    #[test]
    fn an_authority_from_a_key_keeps_its_name_and_issues() {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let der = key.serialize_der();
        let a = RunAuthority::from_pkcs8_der("shared", &der).unwrap();
        let b = RunAuthority::from_pkcs8_der("shared", &der).unwrap();
        assert_eq!(a.root_store().unwrap().len(), 1);
        assert_eq!(
            b.issue(&["x.example.test"], LeafKey::EcdsaP256)
                .unwrap()
                .chain()
                .len(),
            2
        );
        assert!(matches!(
            RunAuthority::from_pkcs8_der("shared", b"not a key"),
            Err(TlsError::Authority { .. })
        ));
    }

    #[test]
    fn two_runs_have_different_authorities() {
        let a = RunAuthority::generate("run").unwrap();
        let b = RunAuthority::generate("run").unwrap();
        assert_ne!(a.ca_pem(), b.ca_pem());
    }
}
