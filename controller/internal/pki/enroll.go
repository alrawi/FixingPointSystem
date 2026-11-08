// SPDX-License-Identifier: GPL-3.0-only

package pki

import (
	"crypto"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"encoding/pem"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// Limits of enrollment tokens.
const (
	// DefaultTokenTTL is the lifetime of a token when NewToken gets 0.
	DefaultTokenTTL = time.Hour
	// MaxTokenTTL caps the lifetime of a token.
	MaxTokenTTL = 7 * 24 * time.Hour
	// tokenKeep is how long an expired token stays in the state, so that a
	// late reuse reports expiry and not an unknown token.
	tokenKeep = 24 * time.Hour
)

// tokenRec is one enrollment token. The state keeps the SHA-256 digest of the
// secret and never the secret.
type tokenRec struct {
	ID      string    `json:"id"`
	Hash    string    `json:"hash"`
	AgentID string    `json:"agent_id"`
	Expires time.Time `json:"expires"`
	Used    time.Time `json:"used,omitzero"`
}

// certRec is a certificate that the controller still accepts.
type certRec struct {
	Serial   string    `json:"serial"`
	AgentID  string    `json:"agent_id"`
	NotAfter time.Time `json:"not_after"`
}

type stateFile struct {
	Version int        `json:"version"`
	Tokens  []tokenRec `json:"tokens"`
	Certs   []certRec  `json:"certs"`
}

// Issued is the result of Enroll and Renew.
type Issued struct {
	// AgentID is the identity in the certificate subject.
	AgentID string `json:"agent_id"`
	// CertPEM is the client certificate.
	CertPEM []byte `json:"-"`
	// CAPEM is the CA certificate that the agent trusts for the controller.
	CAPEM []byte `json:"-"`
	// NotBefore and NotAfter bound the validity of the certificate.
	NotBefore time.Time `json:"not_before"`
	NotAfter  time.Time `json:"not_after"`
	// Serial is the certificate serial number in hex.
	Serial string `json:"serial"`
}

// RenewAt returns the time from which an agent should renew, which is when a
// third of the lifetime remains.
func (i *Issued) RenewAt() time.Time { return RenewAt(i.NotBefore, i.NotAfter) }

// RenewAt returns the time from which an agent should renew a certificate
// with the given validity, which is when a third of the lifetime remains.
func RenewAt(notBefore, notAfter time.Time) time.Time {
	return notAfter.Add(-notAfter.Sub(notBefore) / 3)
}

// NewToken creates a single use enrollment token for agentID that expires after
// ttl, which defaults to DefaultTokenTTL and has the cap MaxTokenTTL. The
// token has the form id.secret. The controller stores only a digest of the
// secret, so this return value is the only copy; the operator hands it to the
// agent once. A token for a revoked identity is allowed, and it re-enrolls the
// agent.
func (a *Authority) NewToken(agentID string, ttl time.Duration) (string, time.Time, error) {
	if !ValidID(agentID) {
		return "", time.Time{}, ErrInvalidID
	}
	if ttl <= 0 {
		ttl = DefaultTokenTTL
	}
	if ttl > MaxTokenTTL {
		return "", time.Time{}, fmt.Errorf("pki: token lifetime %s exceeds the maximum %s", ttl, MaxTokenTTL)
	}
	var idb [8]byte
	var sec [32]byte
	if _, err := rand.Read(idb[:]); err != nil {
		return "", time.Time{}, fmt.Errorf("pki: token: %w", err)
	}
	if _, err := rand.Read(sec[:]); err != nil {
		return "", time.Time{}, fmt.Errorf("pki: token: %w", err)
	}
	id := hex.EncodeToString(idb[:])
	secret := base64.RawURLEncoding.EncodeToString(sec[:])
	digest := sha256.Sum256([]byte(secret))

	a.mu.Lock()
	defer a.mu.Unlock()
	now := a.now()
	rec := &tokenRec{ID: id, Hash: hex.EncodeToString(digest[:]), AgentID: agentID, Expires: now.Add(ttl)}
	a.tokens[id] = rec
	if err := a.saveLocked(); err != nil {
		delete(a.tokens, id)
		return "", time.Time{}, err
	}
	return id + "." + secret, rec.Expires, nil
}

// Enroll exchanges a token for a client certificate. It checks the token, the
// CSR, and the expiry, marks the token used, and issues a certificate for the
// agent identity that the token names. The CSR must hold a P-256 or P-384
// ECDSA key and a valid signature, and its subject must be empty or name the
// same identity. The CSR fields other than the public key are never copied.
//
// A token fails with ErrInvalidToken when it is unknown or wrong, with
// ErrTokenUsed or ErrTokenExpired when its secret matched but it cannot be
// used, and a token that fails never reveals which identity it was for.
func (a *Authority) Enroll(token string, csrDER []byte) (*Issued, error) {
	id, secret, wellFormed := splitToken(token)
	a.mu.Lock()
	defer a.mu.Unlock()
	now := a.now()

	rec := a.tokens[id]
	// Compare digests of equal length in constant time. An unknown id is
	// compared against a zero digest, so the work does not depend on whether
	// the id exists.
	var want [sha256.Size]byte
	known := rec != nil
	if known {
		if b, err := hex.DecodeString(rec.Hash); err == nil && len(b) == sha256.Size {
			copy(want[:], b)
		} else {
			known = false
		}
	}
	got := sha256.Sum256([]byte(secret))
	match := subtle.ConstantTimeCompare(got[:], want[:]) == 1
	if !wellFormed || !known || !match {
		return nil, ErrInvalidToken
	}
	if !rec.Used.IsZero() {
		return nil, ErrTokenUsed
	}
	if !now.Before(rec.Expires) {
		return nil, ErrTokenExpired
	}
	pub, err := checkCSR(csrDER, rec.AgentID)
	if err != nil {
		return nil, err // the token stays unused
	}
	rec.Used = now
	issued, cert, err := a.issueLocked(rec.AgentID, pub)
	if err == nil {
		err = a.saveLocked()
	}
	if err != nil {
		rec.Used = time.Time{}
		if cert != nil {
			delete(a.certs, serialHex(cert))
		}
		return nil, err
	}
	return issued, nil
}

// Renew issues a new certificate for the identity of a certificate that the
// agent presented on a TLS connection. It verifies the presented certificate
// again against the CA, so it does not rely on the listener. An expired,
// foreign, or revoked certificate fails; the agent then enrolls again with a
// new token. The old certificate stays valid until it expires or Revoke ends
// the identity, which lets an agent that lost the reply retry.
func (a *Authority) Renew(presented *x509.Certificate, csrDER []byte) (*Issued, error) {
	if presented == nil {
		return nil, ErrCertificateInvalid
	}
	if err := a.Authorize(presented); err != nil {
		return nil, err
	}
	pub, err := checkCSR(csrDER, AgentID(presented))
	if err != nil {
		return nil, err
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	issued, cert, err := a.issueLocked(AgentID(presented), pub)
	if err == nil {
		err = a.saveLocked()
	}
	if err != nil {
		if cert != nil {
			delete(a.certs, serialHex(cert))
		}
		return nil, err
	}
	return issued, nil
}

// Authorize reports whether the controller accepts a client certificate. The
// certificate must chain to the CA with the client authentication usage and be
// inside its validity, and the CA must have issued it and not revoked its
// identity. The TLS configs call it for every handshake, and the agent manager
// may call it again for long lived streams.
func (a *Authority) Authorize(c *x509.Certificate) error {
	if c == nil {
		return ErrCertificateInvalid
	}
	_, err := c.Verify(x509.VerifyOptions{
		Roots:       a.pool,
		CurrentTime: a.now(),
		KeyUsages:   []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth},
	})
	if err != nil {
		return fmt.Errorf("%w: %w", ErrCertificateInvalid, err)
	}
	a.mu.Lock()
	defer a.mu.Unlock()
	rec, ok := a.certs[serialHex(c)]
	if !ok || rec.AgentID != AgentID(c) {
		return ErrRevoked
	}
	return nil
}

// Revoke ends an agent identity. Every certificate of the identity stops being
// accepted at once, its unused tokens are deleted, and it cannot renew. The
// agent can enroll again only with a token that NewToken creates afterwards.
// Revoke returns the number of certificates that it ended and runs
// Config.OnRevoke, so the caller can close the open session, because a
// certificate is checked at the handshake only.
func (a *Authority) Revoke(agentID string) (int, error) {
	a.mu.Lock()
	n := 0
	for s, r := range a.certs {
		if r.AgentID == agentID {
			delete(a.certs, s)
			n++
		}
	}
	for id, t := range a.tokens {
		if t.AgentID == agentID && t.Used.IsZero() {
			delete(a.tokens, id)
		}
	}
	err := a.saveLocked()
	a.mu.Unlock()
	if err != nil {
		return n, err
	}
	if a.cfg.OnRevoke != nil {
		a.cfg.OnRevoke(agentID)
	}
	return n, nil
}

// issueLocked signs a client certificate and records its serial. The caller
// holds a.mu and saves the state.
func (a *Authority) issueLocked(agentID string, pub crypto.PublicKey) (*Issued, *x509.Certificate, error) {
	serial, err := newSerial()
	if err != nil {
		return nil, nil, err
	}
	now := a.now()
	tmpl := &x509.Certificate{
		SerialNumber:          serial,
		Subject:               pkix.Name{CommonName: agentID},
		NotBefore:             now.Add(-clockSkew),
		NotAfter:              now.Add(a.cfg.ClientLifetime),
		KeyUsage:              x509.KeyUsageDigitalSignature,
		ExtKeyUsage:           []x509.ExtKeyUsage{x509.ExtKeyUsageClientAuth},
		BasicConstraintsValid: true,
	}
	// The leaf may not outlive the CA.
	if tmpl.NotAfter.After(a.caCert.NotAfter) {
		tmpl.NotAfter = a.caCert.NotAfter
	}
	der, err := x509.CreateCertificate(rand.Reader, tmpl, a.caCert, pub, a.caKey)
	if err != nil {
		return nil, nil, fmt.Errorf("pki: sign certificate: %w", err)
	}
	cert, err := x509.ParseCertificate(der)
	if err != nil {
		return nil, nil, err
	}
	a.certs[serialHex(cert)] = certRec{Serial: serialHex(cert), AgentID: agentID, NotAfter: cert.NotAfter}
	return &Issued{
		AgentID:   agentID,
		CertPEM:   pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}),
		CAPEM:     a.CACertPEM(),
		NotBefore: cert.NotBefore,
		NotAfter:  cert.NotAfter,
		Serial:    serialHex(cert),
	}, cert, nil
}

// checkCSR validates a CSR and returns its public key.
func checkCSR(der []byte, agentID string) (crypto.PublicKey, error) {
	csr, err := x509.ParseCertificateRequest(der)
	if err != nil {
		return nil, fmt.Errorf("%w: %w", ErrInvalidCSR, err)
	}
	if err := csr.CheckSignature(); err != nil {
		return nil, fmt.Errorf("%w: signature: %w", ErrInvalidCSR, err)
	}
	pub, ok := csr.PublicKey.(*ecdsa.PublicKey)
	if !ok || (pub.Curve != elliptic.P256() && pub.Curve != elliptic.P384()) {
		return nil, fmt.Errorf("%w: the key must be ECDSA on P-256 or P-384", ErrInvalidCSR)
	}
	if cn := csr.Subject.CommonName; cn != "" && cn != agentID {
		return nil, fmt.Errorf("%w: the subject names a different identity", ErrInvalidCSR)
	}
	return pub, nil
}

// NewCSR generates a P-256 key and a CSR for agentID. It serves Go clients and
// tests; the agent does the same with its own library. The key is returned as
// PEM (PKCS#8) and the CSR as DER.
func NewCSR(agentID string) (keyPEM, csrDER []byte, err error) {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, nil, err
	}
	csrDER, err = x509.CreateCertificateRequest(rand.Reader, &x509.CertificateRequest{
		Subject: pkix.Name{CommonName: agentID},
	}, key)
	if err != nil {
		return nil, nil, err
	}
	kd, err := x509.MarshalPKCS8PrivateKey(key)
	if err != nil {
		return nil, nil, err
	}
	return pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: kd}), csrDER, nil
}

// splitToken splits id.secret. A malformed token yields empty parts.
func splitToken(token string) (id, secret string, ok bool) {
	id, secret, ok = strings.Cut(token, ".")
	if !ok || id == "" || secret == "" || strings.Contains(secret, ".") || len(token) > 256 {
		return "", "", false
	}
	return id, secret, true
}

// loadState reads state.json. A damaged file ends Open, because dropping it
// would silently revoke or resurrect identities.
func (a *Authority) loadState() error {
	data, err := os.ReadFile(filepath.Join(a.cfg.Dir, StateFile))
	if errors.Is(err, fs.ErrNotExist) {
		return nil
	}
	if err != nil {
		return fmt.Errorf("pki: read state: %w", err)
	}
	var st stateFile
	if err := json.Unmarshal(data, &st); err != nil {
		return fmt.Errorf("pki: parse %s: %w", StateFile, err)
	}
	if st.Version != 1 {
		return fmt.Errorf("pki: %s has version %d, want 1", StateFile, st.Version)
	}
	for i := range st.Tokens {
		t := st.Tokens[i]
		if b, err := hex.DecodeString(t.Hash); err != nil || len(b) != sha256.Size || t.ID == "" || !ValidID(t.AgentID) {
			return fmt.Errorf("pki: %s holds a damaged token entry", StateFile)
		}
		a.tokens[t.ID] = &t
	}
	for _, c := range st.Certs {
		a.certs[c.Serial] = c
	}
	return nil
}

// saveLocked prunes the state and writes it with mode 0600 through a rename,
// so a crash leaves either the old file or the new one. The caller holds a.mu.
func (a *Authority) saveLocked() error {
	if a.cfg.Dir == "" {
		return nil
	}
	now := a.now()
	st := stateFile{Version: 1, Tokens: []tokenRec{}, Certs: []certRec{}}
	for id, t := range a.tokens {
		if now.After(t.Expires.Add(tokenKeep)) {
			delete(a.tokens, id)
			continue
		}
		st.Tokens = append(st.Tokens, *t)
	}
	for s, c := range a.certs {
		if now.After(c.NotAfter) {
			delete(a.certs, s)
			continue
		}
		st.Certs = append(st.Certs, c)
	}
	data, err := json.MarshalIndent(st, "", "  ")
	if err != nil {
		return err
	}
	tmp, err := os.CreateTemp(a.cfg.Dir, StateFile+".tmp-*")
	if err != nil {
		return fmt.Errorf("pki: save state: %w", err)
	}
	defer func() { _ = os.Remove(tmp.Name()) }()
	if err := tmp.Chmod(0o600); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("pki: save state: %w", err)
	}
	if _, err := tmp.Write(data); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("pki: save state: %w", err)
	}
	if err := tmp.Sync(); err != nil {
		_ = tmp.Close()
		return fmt.Errorf("pki: save state: %w", err)
	}
	if err := tmp.Close(); err != nil {
		return fmt.Errorf("pki: save state: %w", err)
	}
	if err := os.Rename(tmp.Name(), filepath.Join(a.cfg.Dir, StateFile)); err != nil {
		return fmt.Errorf("pki: save state: %w", err)
	}
	return nil
}
