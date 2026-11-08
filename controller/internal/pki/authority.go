// SPDX-License-Identifier: GPL-3.0-only

package pki

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/hex"
	"encoding/pem"
	"errors"
	"fmt"
	"io/fs"
	"math/big"
	"os"
	"path/filepath"
	"regexp"
	"sync"
	"time"
)

// File names inside Config.Dir.
const (
	CACertFile = "ca.crt"
	CAKeyFile  = "ca.key"
	StateFile  = "state.json"
)

// Defaults of Config.
const (
	DefaultClientLifetime = 24 * time.Hour
	DefaultServerLifetime = 30 * 24 * time.Hour
	caLifetime            = 10 * 365 * 24 * time.Hour
	clockSkew             = time.Minute
)

// Errors of the enrollment, rotation, and verification paths. Callers compare
// them with errors.Is.
var (
	// ErrInvalidToken means that the token is malformed, unknown, or wrong.
	// It does not say which, so a guesser learns nothing.
	ErrInvalidToken = errors.New("pki: invalid enrollment token")
	// ErrTokenUsed means that a valid token was already exchanged.
	ErrTokenUsed = errors.New("pki: enrollment token already used")
	// ErrTokenExpired means that a valid token is past its expiry.
	ErrTokenExpired = errors.New("pki: enrollment token expired")
	// ErrInvalidCSR means that the certificate signing request is unusable.
	ErrInvalidCSR = errors.New("pki: invalid certificate signing request")
	// ErrInvalidID means that an agent identity is outside the Id pattern.
	ErrInvalidID = errors.New("pki: invalid agent identity")
	// ErrCertificateInvalid means that a certificate does not chain to the
	// CA, is expired, or lacks the client authentication usage.
	ErrCertificateInvalid = errors.New("pki: certificate not valid")
	// ErrRevoked means that the CA issued the certificate but no longer
	// accepts it, because the identity was revoked.
	ErrRevoked = errors.New("pki: certificate revoked or unknown")
)

// idPattern is the Id parameter pattern of the REST contract. The agents
// package holds the same rule, and this package copies it to stay free of
// other controller packages.
var idPattern = regexp.MustCompile(`^[A-Za-z0-9_-]{1,64}$`)

// ValidID reports whether id is a valid agent identity.
func ValidID(id string) bool { return idPattern.MatchString(id) }

// Config configures Open.
type Config struct {
	// Dir holds ca.crt, ca.key, and state.json. Open creates it with mode
	// 0700. An empty Dir keeps everything in memory, which serves tests.
	Dir string
	// ClientLifetime is the validity of an agent certificate. The default
	// is 24 hours.
	ClientLifetime time.Duration
	// ServerLifetime is the validity of the controller server certificate.
	// The default is 30 days.
	ServerLifetime time.Duration
	// Now tells the time. The default is time.Now.
	Now func() time.Time
	// OnRevoke runs after Revoke committed a revocation, outside every
	// lock. The agent manager uses it to end the open session of the agent,
	// because the TLS handshake checks revocation only for new connections.
	OnRevoke func(agentID string)
}

// Authority is the controller CA with its enrollment state. It is safe for
// concurrent use.
type Authority struct {
	cfg    Config
	caCert *x509.Certificate
	caKey  *ecdsa.PrivateKey
	caPEM  []byte
	pool   *x509.CertPool

	mu     sync.Mutex
	tokens map[string]*tokenRec
	certs  map[string]certRec // issued and still accepted, by serial in hex

	srvMu sync.Mutex
	srv   *serverCert
}

// Open loads the CA from cfg.Dir, or generates it when the directory holds
// none. The private key file has mode 0600 and the directory mode 0700. Open
// refuses a key file that is readable by group or others, a certificate that
// does not match the key, and an expired CA, because each means that the trust
// anchor is not what the controller wrote.
func Open(cfg Config) (*Authority, error) {
	if cfg.ClientLifetime <= 0 {
		cfg.ClientLifetime = DefaultClientLifetime
	}
	if cfg.ServerLifetime <= 0 {
		cfg.ServerLifetime = DefaultServerLifetime
	}
	if cfg.Now == nil {
		cfg.Now = time.Now
	}
	a := &Authority{cfg: cfg, tokens: map[string]*tokenRec{}, certs: map[string]certRec{}}
	if cfg.Dir == "" {
		if err := a.generateCA(); err != nil {
			return nil, err
		}
	} else {
		if err := os.MkdirAll(cfg.Dir, 0o700); err != nil {
			return nil, fmt.Errorf("pki: create %s: %w", cfg.Dir, err)
		}
		if err := a.loadOrCreateCA(); err != nil {
			return nil, err
		}
		if err := a.loadState(); err != nil {
			return nil, err
		}
	}
	a.pool = x509.NewCertPool()
	a.pool.AddCert(a.caCert)
	return a, nil
}

func (a *Authority) now() time.Time { return a.cfg.Now() }

// CACertPEM returns the CA certificate in PEM form. Agents trust it as the
// root of the controller server certificate.
func (a *Authority) CACertPEM() []byte { return append([]byte(nil), a.caPEM...) }

// CAFingerprint returns the SHA-256 digest of the CA certificate in hex, so an
// operator can pin the CA out of band on the agent.
func (a *Authority) CAFingerprint() string {
	sum := sha256.Sum256(a.caCert.Raw)
	return hex.EncodeToString(sum[:])
}

// AgentID returns the agent identity that a client certificate names, which is
// the subject common name. It matches agents.PeerCertIdentity.
func AgentID(c *x509.Certificate) string { return c.Subject.CommonName }

func (a *Authority) generateCA() error {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return fmt.Errorf("pki: generate CA key: %w", err)
	}
	serial, err := newSerial()
	if err != nil {
		return err
	}
	now := a.now()
	tmpl := &x509.Certificate{
		SerialNumber:          serial,
		Subject:               pkix.Name{CommonName: "FPS controller CA", Organization: []string{"FixingPointSystem"}},
		NotBefore:             now.Add(-clockSkew),
		NotAfter:              now.Add(caLifetime),
		KeyUsage:              x509.KeyUsageCertSign | x509.KeyUsageCRLSign,
		BasicConstraintsValid: true,
		IsCA:                  true,
		MaxPathLen:            0,
		MaxPathLenZero:        true,
	}
	der, err := x509.CreateCertificate(rand.Reader, tmpl, tmpl, &key.PublicKey, key)
	if err != nil {
		return fmt.Errorf("pki: create CA certificate: %w", err)
	}
	cert, err := x509.ParseCertificate(der)
	if err != nil {
		return err
	}
	a.caCert, a.caKey = cert, key
	a.caPEM = pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
	return nil
}

func (a *Authority) loadOrCreateCA() error {
	certPath := filepath.Join(a.cfg.Dir, CACertFile)
	keyPath := filepath.Join(a.cfg.Dir, CAKeyFile)
	_, certErr := os.Stat(certPath)
	_, keyErr := os.Stat(keyPath)
	switch {
	case errors.Is(certErr, fs.ErrNotExist) && errors.Is(keyErr, fs.ErrNotExist):
		if err := a.generateCA(); err != nil {
			return err
		}
		keyDER, err := x509.MarshalPKCS8PrivateKey(a.caKey)
		if err != nil {
			return err
		}
		// The key goes first, so a crash never leaves a certificate without
		// its key, which the next start would refuse as half a CA.
		if err := writeNew(keyPath, pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: keyDER}), 0o600); err != nil {
			return err
		}
		return writeNew(certPath, a.caPEM, 0o644)
	case certErr != nil || keyErr != nil:
		return fmt.Errorf("pki: %s holds only one of %s and %s; restore both or remove both", a.cfg.Dir, CACertFile, CAKeyFile)
	}
	info, err := os.Stat(keyPath)
	if err != nil {
		return err
	}
	if info.Mode().Perm()&0o077 != 0 {
		return fmt.Errorf("pki: %s has mode %04o; it must not be accessible by group or others (chmod 600)", keyPath, info.Mode().Perm())
	}
	keyPEM, err := os.ReadFile(keyPath)
	if err != nil {
		return err
	}
	certPEM, err := os.ReadFile(certPath)
	if err != nil {
		return err
	}
	kb, _ := pem.Decode(keyPEM)
	if kb == nil {
		return fmt.Errorf("pki: %s holds no PEM block", keyPath)
	}
	parsed, err := x509.ParsePKCS8PrivateKey(kb.Bytes)
	if err != nil {
		return fmt.Errorf("pki: parse %s: %w", keyPath, err)
	}
	key, ok := parsed.(*ecdsa.PrivateKey)
	if !ok {
		return fmt.Errorf("pki: %s is not an ECDSA key", keyPath)
	}
	cb, _ := pem.Decode(certPEM)
	if cb == nil || cb.Type != "CERTIFICATE" {
		return fmt.Errorf("pki: %s holds no certificate", certPath)
	}
	cert, err := x509.ParseCertificate(cb.Bytes)
	if err != nil {
		return fmt.Errorf("pki: parse %s: %w", certPath, err)
	}
	if !cert.IsCA {
		return fmt.Errorf("pki: %s is not a CA certificate", certPath)
	}
	pub, ok := cert.PublicKey.(*ecdsa.PublicKey)
	if !ok || !pub.Equal(&key.PublicKey) {
		return fmt.Errorf("pki: %s does not match %s", certPath, keyPath)
	}
	if !a.now().Before(cert.NotAfter) {
		return fmt.Errorf("pki: the CA certificate expired at %s; replace the CA and enroll every agent again", cert.NotAfter.UTC().Format(time.RFC3339))
	}
	a.caCert, a.caKey, a.caPEM = cert, key, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: cb.Bytes})
	return nil
}

// writeNew creates path with the given mode and fails when it exists.
func writeNew(path string, data []byte, mode os.FileMode) error {
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, mode)
	if err != nil {
		return fmt.Errorf("pki: create %s: %w", path, err)
	}
	if _, err := f.Write(data); err != nil {
		_ = f.Close()
		return fmt.Errorf("pki: write %s: %w", path, err)
	}
	if err := f.Close(); err != nil {
		return fmt.Errorf("pki: write %s: %w", path, err)
	}
	// The process umask may have cleared bits, so state the mode again.
	return os.Chmod(path, mode)
}

func newSerial() (*big.Int, error) {
	limit := new(big.Int).Lsh(big.NewInt(1), 127)
	n, err := rand.Int(rand.Reader, limit)
	if err != nil {
		return nil, fmt.Errorf("pki: serial number: %w", err)
	}
	return n.Add(n, big.NewInt(1)), nil // positive, below 2^127 + 1
}

func serialHex(c *x509.Certificate) string { return c.SerialNumber.Text(16) }
