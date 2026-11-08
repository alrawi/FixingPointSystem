// SPDX-License-Identifier: GPL-3.0-only

package pki

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"errors"
	"fmt"
	"net"
)

// serverCert is the controller server certificate. The authority reissues it
// when a third of its lifetime remains, so a long running controller never
// serves an expired certificate.
type serverCert struct {
	names []string
	cert  *tls.Certificate
	leaf  *x509.Certificate
}

// ServerTLSConfig builds the tls.Config of the gRPC server. It requires a
// client certificate and verifies it against the CA with the client
// authentication usage. The handshake then checks that the CA issued the
// certificate and did not revoke its identity, so a client without a valid
// certificate never reaches a handler. The subject common name of the verified
// certificate is the agent identity, and AgentID and agents.PeerCertIdentity
// read it from the first verified chain.
//
// names are the host names and IP addresses that agents dial, which become the
// subject alternative names of the server certificate. The configuration
// requires TLS 1.3 and offers only h2. Pass it to
// credentials.NewTLS(cfg) for grpc.Creds.
func (a *Authority) ServerTLSConfig(names ...string) (*tls.Config, error) {
	return a.serverConfig(tls.RequireAndVerifyClientCert, []string{"h2"}, names)
}

// EnrollTLSConfig builds the tls.Config of the enrollment listener behind
// Handler. It does not require a client certificate, because an agent that
// enrolls has none. It verifies a certificate that the client does present,
// which the renew path relies on. It offers only HTTP/1.1.
func (a *Authority) EnrollTLSConfig(names ...string) (*tls.Config, error) {
	return a.serverConfig(tls.VerifyClientCertIfGiven, []string{"http/1.1"}, names)
}

func (a *Authority) serverConfig(auth tls.ClientAuthType, protos, names []string) (*tls.Config, error) {
	if len(names) == 0 {
		return nil, errors.New("pki: the server certificate needs at least one host name or address")
	}
	if _, err := a.serverCertificate(names); err != nil {
		return nil, err
	}
	return &tls.Config{
		MinVersion:     tls.VersionTLS13,
		ClientAuth:     auth,
		ClientCAs:      a.pool,
		NextProtos:     protos,
		Time:           a.now,
		GetCertificate: func(*tls.ClientHelloInfo) (*tls.Certificate, error) { return a.serverCertificate(names) },
		// Session tickets stay off, so every connection runs the full
		// handshake and a revocation takes effect at the next dial.
		SessionTicketsDisabled: true,
		// VerifyConnection also runs on a resumed session, where
		// VerifyPeerCertificate would not.
		VerifyConnection: func(cs tls.ConnectionState) error {
			if len(cs.VerifiedChains) == 0 || len(cs.VerifiedChains[0]) == 0 {
				if auth == tls.RequireAndVerifyClientCert {
					return ErrCertificateInvalid
				}
				return nil
			}
			return a.Authorize(cs.VerifiedChains[0][0])
		},
	}, nil
}

// serverCertificate returns the cached server certificate or issues a new one.
func (a *Authority) serverCertificate(names []string) (*tls.Certificate, error) {
	a.srvMu.Lock()
	defer a.srvMu.Unlock()
	now := a.now()
	if s := a.srv; s != nil && sameNames(s.names, names) && now.Before(RenewAt(s.leaf.NotBefore, s.leaf.NotAfter)) {
		return s.cert, nil
	}
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, err
	}
	serial, err := newSerial()
	if err != nil {
		return nil, err
	}
	tmpl := &x509.Certificate{
		SerialNumber:          serial,
		Subject:               pkix.Name{CommonName: "fps-controller"},
		NotBefore:             now.Add(-clockSkew),
		NotAfter:              now.Add(a.cfg.ServerLifetime),
		KeyUsage:              x509.KeyUsageDigitalSignature,
		ExtKeyUsage:           []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		BasicConstraintsValid: true,
	}
	if tmpl.NotAfter.After(a.caCert.NotAfter) {
		tmpl.NotAfter = a.caCert.NotAfter
	}
	for _, n := range names {
		if ip := net.ParseIP(n); ip != nil {
			tmpl.IPAddresses = append(tmpl.IPAddresses, ip)
		} else {
			tmpl.DNSNames = append(tmpl.DNSNames, n)
		}
	}
	der, err := x509.CreateCertificate(rand.Reader, tmpl, a.caCert, &key.PublicKey, a.caKey)
	if err != nil {
		return nil, fmt.Errorf("pki: sign server certificate: %w", err)
	}
	leaf, err := x509.ParseCertificate(der)
	if err != nil {
		return nil, err
	}
	a.srv = &serverCert{
		names: append([]string(nil), names...),
		leaf:  leaf,
		cert:  &tls.Certificate{Certificate: [][]byte{der}, PrivateKey: key, Leaf: leaf},
	}
	return a.srv.cert, nil
}

func sameNames(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

// ClientTLSConfig builds the tls.Config of a Go client of the controller. It
// trusts only caPEM, checks the server name, and presents cert when it is not
// nil. Enrollment uses a nil cert. Go tests and tools use it; the Rust agent
// builds the equivalent with rustls.
func ClientTLSConfig(caPEM []byte, cert *tls.Certificate, serverName string) (*tls.Config, error) {
	pool := x509.NewCertPool()
	if !pool.AppendCertsFromPEM(caPEM) {
		return nil, errors.New("pki: no CA certificate in the PEM data")
	}
	cfg := &tls.Config{MinVersion: tls.VersionTLS13, RootCAs: pool, ServerName: serverName}
	if cert != nil {
		cfg.Certificates = []tls.Certificate{*cert}
	}
	return cfg, nil
}

// KeyPair pairs a certificate PEM with its private key PEM for use in a
// client tls.Config.
func KeyPair(certPEM, keyPEM []byte) (*tls.Certificate, error) {
	c, err := tls.X509KeyPair(certPEM, keyPEM)
	if err != nil {
		return nil, err
	}
	return &c, nil
}
