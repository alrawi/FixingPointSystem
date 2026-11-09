// SPDX-License-Identifier: GPL-3.0-only

package pki

import (
	"bytes"
	"crypto/sha256"
	"crypto/tls"
	"encoding/hex"
	"net"
	"testing"
	"time"
)

// presentedChain dials a TLS listener and returns the certificates that the
// server presented, in the order that it sent them.
func presentedChain(t *testing.T, s *server, a *Authority, cert *tls.Certificate) tls.ConnectionState {
	t.Helper()
	d := &net.Dialer{Timeout: 5 * time.Second}
	c, err := tls.DialWithDialer(d, "tcp", s.addr, clientCfg(t, a, cert))
	if err != nil {
		t.Fatalf("dial: %v", err)
	}
	defer func() { _ = c.Close() }()
	return c.ConnectionState()
}

// TestServerPresentsCAAfterLeaf shows that both listeners send the leaf first
// and the CA second, so a client that pins the CA fingerprint finds the CA in
// the handshake before it sends anything.
func TestServerPresentsCAAfterLeaf(t *testing.T) {
	a := newAuthority(t, newClock())
	cert, _ := enrollAgent(t, a, "agent-a")
	grpcCfg, err := a.ServerTLSConfig("127.0.0.1")
	if err != nil {
		t.Fatal(err)
	}
	enrollCfg, err := a.EnrollTLSConfig("127.0.0.1")
	if err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		name   string
		cfg    *tls.Config
		client *tls.Certificate
	}{
		{"gRPC listener", grpcCfg, cert},
		{"enrollment listener", enrollCfg, nil},
	} {
		t.Run(tc.name, func(t *testing.T) {
			st := presentedChain(t, startServer(t, tc.cfg), a, tc.client)
			chain := st.PeerCertificates
			if len(chain) != 2 {
				t.Fatalf("server sent %d certificates, want the leaf and the CA", len(chain))
			}
			if chain[0].IsCA || chain[0].Subject.CommonName != "fps-controller" {
				t.Errorf("first certificate = %q (CA %v), want the leaf", chain[0].Subject.CommonName, chain[0].IsCA)
			}
			if !chain[1].IsCA || !bytes.Equal(chain[1].Raw, a.caCert.Raw) {
				t.Errorf("second certificate is not the CA of the authority")
			}
			sum := sha256.Sum256(chain[1].Raw)
			if got := hex.EncodeToString(sum[:]); got != a.CAFingerprint() {
				t.Errorf("fingerprint of the sent CA = %s, want %s", got, a.CAFingerprint())
			}
		})
	}
}
