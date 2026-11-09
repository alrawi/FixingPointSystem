// SPDX-License-Identifier: GPL-3.0-only

package signing

import (
	"crypto/ed25519"
	"crypto/sha256"
	"crypto/x509"
	"encoding/hex"
	"encoding/pem"
	"fmt"
	"io"
	"regexp"
)

// KeyIDPrefix starts every key id.
const KeyIDPrefix = "ed25519:"

// SignatureSize is the length of an Ed25519 signature in bytes.
const SignatureSize = ed25519.SignatureSize

// digestPattern is the form of a module tree digest.
var digestPattern = regexp.MustCompile(`^sha256:[0-9a-f]{64}$`)

// KeyID derives the id of a public key. It is KeyIDPrefix followed by the hex
// of the first 16 bytes of the SHA-256 of the 32 byte public key, so the id is
// 40 characters and needs no registry to assign it.
func KeyID(pub ed25519.PublicKey) string {
	sum := sha256.Sum256(pub)
	return KeyIDPrefix + hex.EncodeToString(sum[:16])
}

// Signature is a detached signature and the id of the key that made it. The
// two fields map to ModuleRef.signature and ModuleRef.signature_key_id.
type Signature struct {
	// KeyID is the id of the signing key, empty for an unsigned module.
	KeyID string
	// Bytes is the 64 byte Ed25519 signature, empty for an unsigned module.
	Bytes []byte
}

// Signer signs digests and indexes with one Ed25519 private key.
type Signer struct {
	priv ed25519.PrivateKey
	id   string
}

// GenerateSigner creates a key pair from the random source r, which callers
// pass crypto/rand.Reader for real keys.
func GenerateSigner(r io.Reader) (*Signer, error) {
	_, priv, err := ed25519.GenerateKey(r)
	if err != nil {
		return nil, fmt.Errorf("generate signing key: %w", err)
	}
	return NewSigner(priv)
}

// NewSigner wraps a private key.
func NewSigner(priv ed25519.PrivateKey) (*Signer, error) {
	if len(priv) != ed25519.PrivateKeySize {
		return nil, fmt.Errorf("%w: private key has %d bytes, want %d", ErrKey, len(priv), ed25519.PrivateKeySize)
	}
	s := &Signer{priv: append(ed25519.PrivateKey(nil), priv...)}
	s.id = KeyID(s.PublicKey())
	return s, nil
}

// KeyID returns the id of the public key.
func (s *Signer) KeyID() string { return s.id }

// PublicKey returns the public key.
func (s *Signer) PublicKey() ed25519.PublicKey {
	pub, _ := s.priv.Public().(ed25519.PublicKey)
	return pub
}

// SignDigest signs the UTF-8 bytes of a module tree digest, as CON-05 defines
// ModuleRef.signature.
func (s *Signer) SignDigest(digest string) (Signature, error) {
	if !digestPattern.MatchString(digest) {
		return Signature{}, fmt.Errorf("sign %q: %w", digest, ErrDigest)
	}
	return Signature{KeyID: s.id, Bytes: ed25519.Sign(s.priv, []byte(digest))}, nil
}

// MarshalPEM encodes the private key as a PKCS #8 PEM block. The result is a
// secret.
func (s *Signer) MarshalPEM() ([]byte, error) {
	der, err := x509.MarshalPKCS8PrivateKey(s.priv)
	if err != nil {
		return nil, fmt.Errorf("%w: marshal private key: %s", ErrKey, err.Error())
	}
	return pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: der}), nil
}

// ParseSignerPEM reads a private key that MarshalPEM wrote.
func ParseSignerPEM(data []byte) (*Signer, error) {
	block, _ := pem.Decode(data)
	if block == nil || block.Type != "PRIVATE KEY" {
		return nil, fmt.Errorf("%w: no PRIVATE KEY block", ErrKey)
	}
	k, err := x509.ParsePKCS8PrivateKey(block.Bytes)
	if err != nil {
		return nil, fmt.Errorf("%w: %s", ErrKey, err.Error())
	}
	priv, ok := k.(ed25519.PrivateKey)
	if !ok {
		return nil, fmt.Errorf("%w: the key is not Ed25519", ErrKey)
	}
	return NewSigner(priv)
}

// MarshalPublicKeyPEM encodes a public key as a PKIX PEM block.
func MarshalPublicKeyPEM(pub ed25519.PublicKey) ([]byte, error) {
	if len(pub) != ed25519.PublicKeySize {
		return nil, fmt.Errorf("%w: public key has %d bytes, want %d", ErrKey, len(pub), ed25519.PublicKeySize)
	}
	der, err := x509.MarshalPKIXPublicKey(pub)
	if err != nil {
		return nil, fmt.Errorf("%w: marshal public key: %s", ErrKey, err.Error())
	}
	return pem.EncodeToMemory(&pem.Block{Type: "PUBLIC KEY", Bytes: der}), nil
}

// ParsePublicKeyPEM reads a public key that MarshalPublicKeyPEM wrote.
func ParsePublicKeyPEM(data []byte) (ed25519.PublicKey, error) {
	block, _ := pem.Decode(data)
	if block == nil || block.Type != "PUBLIC KEY" {
		return nil, fmt.Errorf("%w: no PUBLIC KEY block", ErrKey)
	}
	k, err := x509.ParsePKIXPublicKey(block.Bytes)
	if err != nil {
		return nil, fmt.Errorf("%w: %s", ErrKey, err.Error())
	}
	pub, ok := k.(ed25519.PublicKey)
	if !ok {
		return nil, fmt.Errorf("%w: the key is not Ed25519", ErrKey)
	}
	return pub, nil
}
