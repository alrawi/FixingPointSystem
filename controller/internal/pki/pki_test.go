// SPDX-License-Identifier: GPL-3.0-only

package pki

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/rsa"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"errors"
	"net"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

// clock is a settable clock shared by an authority and its tests.
type clock struct {
	mu sync.Mutex
	t  time.Time
}

func newClock() *clock { return &clock{t: time.Now().Truncate(time.Second)} }

func (c *clock) Now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.t
}

func (c *clock) Advance(d time.Duration) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.t = c.t.Add(d)
}

func newAuthority(t *testing.T, clk *clock) *Authority {
	t.Helper()
	a, err := Open(Config{Now: clk.Now})
	if err != nil {
		t.Fatal(err)
	}
	return a
}

// enrollAgent runs the whole enrollment for id and returns the client key
// pair.
func enrollAgent(t *testing.T, a *Authority, id string) (*tls.Certificate, *Issued) {
	t.Helper()
	tok, _, err := a.NewToken(id, time.Minute)
	if err != nil {
		t.Fatal(err)
	}
	keyPEM, csr, err := NewCSR(id)
	if err != nil {
		t.Fatal(err)
	}
	iss, err := a.Enroll(tok, csr)
	if err != nil {
		t.Fatalf("Enroll: %v", err)
	}
	kp, err := KeyPair(iss.CertPEM, keyPEM)
	if err != nil {
		t.Fatal(err)
	}
	return kp, iss
}

// server is a TLS echo server on loopback. It reports the agent identity of
// every connection that completes the handshake.
type server struct {
	addr string
	ids  chan string
}

func startServer(t *testing.T, cfg *tls.Config) *server {
	t.Helper()
	ln, err := tls.Listen("tcp", "127.0.0.1:0", cfg)
	if err != nil {
		t.Fatal(err)
	}
	s := &server{addr: ln.Addr().String(), ids: make(chan string, 16)}
	t.Cleanup(func() { _ = ln.Close() })
	go func() {
		for {
			c, err := ln.Accept()
			if err != nil {
				return
			}
			go func() {
				defer func() { _ = c.Close() }()
				tc, ok := c.(*tls.Conn)
				if !ok {
					return
				}
				_ = tc.SetDeadline(time.Now().Add(5 * time.Second))
				if err := tc.Handshake(); err != nil {
					return
				}
				st := tc.ConnectionState()
				if len(st.VerifiedChains) > 0 {
					s.ids <- AgentID(st.VerifiedChains[0][0])
				}
				var b [1]byte
				if _, err := tc.Read(b[:]); err == nil {
					_, _ = tc.Write(b[:])
				}
			}()
		}
	}()
	return s
}

// probe dials the server and exchanges one byte. TLS 1.3 reports a rejected
// client certificate on the first read, so the exchange is part of the test.
func probe(s *server, cfg *tls.Config) error {
	d := &net.Dialer{Timeout: 5 * time.Second}
	c, err := tls.DialWithDialer(d, "tcp", s.addr, cfg)
	if err != nil {
		return err
	}
	defer func() { _ = c.Close() }()
	_ = c.SetDeadline(time.Now().Add(5 * time.Second))
	if _, err := c.Write([]byte{1}); err != nil {
		return err
	}
	var b [1]byte
	_, err = c.Read(b[:])
	return err
}

func clientCfg(t *testing.T, a *Authority, cert *tls.Certificate) *tls.Config {
	t.Helper()
	cfg, err := ClientTLSConfig(a.CACertPEM(), cert, "127.0.0.1")
	if err != nil {
		t.Fatal(err)
	}
	return cfg
}

func TestMutualTLSAdmitsOnlyValidCertificates(t *testing.T) {
	clk := newClock()
	a := newAuthority(t, clk)
	scfg, err := a.ServerTLSConfig("127.0.0.1", "controller.test")
	if err != nil {
		t.Fatal(err)
	}
	srv := startServer(t, scfg)

	t.Run("fresh certificate is accepted and names the agent", func(t *testing.T) {
		cert, _ := enrollAgent(t, a, "agent-a")
		if err := probe(srv, clientCfg(t, a, cert)); err != nil {
			t.Fatalf("a freshly enrolled certificate must be accepted: %v", err)
		}
		if got := <-srv.ids; got != "agent-a" {
			t.Fatalf("identity from the certificate subject = %q, want agent-a", got)
		}
	})

	t.Run("no client certificate", func(t *testing.T) {
		if err := probe(srv, clientCfg(t, a, nil)); err == nil {
			t.Fatal("a client without a certificate was accepted")
		}
	})

	t.Run("certificate from another CA", func(t *testing.T) {
		other := newAuthority(t, clk)
		cert, _ := enrollAgent(t, other, "agent-a")
		if err := probe(srv, clientCfg(t, a, cert)); err == nil {
			t.Fatal("a certificate from another CA was accepted")
		}
	})

	t.Run("certificate with the right name that the CA never issued", func(t *testing.T) {
		// A copy of the CA key would be needed to forge this for real. This
		// stands in for state loss: the CA signed it, the controller has no
		// record, so it fails closed.
		b := newAuthority(t, clk)
		b.caCert, b.caKey, b.pool = a.caCert, a.caKey, a.pool
		cert, _ := enrollAgent(t, b, "agent-ghost")
		if err := probe(srv, clientCfg(t, a, cert)); err == nil {
			t.Fatal("a certificate without a record was accepted")
		}
	})

	t.Run("revoked identity", func(t *testing.T) {
		cert, _ := enrollAgent(t, a, "agent-r")
		if err := probe(srv, clientCfg(t, a, cert)); err != nil {
			t.Fatalf("before revocation: %v", err)
		}
		<-srv.ids
		if n, err := a.Revoke("agent-r"); err != nil || n != 1 {
			t.Fatalf("Revoke = %d, %v; want 1, nil", n, err)
		}
		if err := probe(srv, clientCfg(t, a, cert)); err == nil {
			t.Fatal("a revoked identity was accepted")
		}
	})

	t.Run("expired certificate", func(t *testing.T) {
		cert, iss := enrollAgent(t, a, "agent-e")
		if err := probe(srv, clientCfg(t, a, cert)); err != nil {
			t.Fatalf("before expiry: %v", err)
		}
		<-srv.ids
		clk.Advance(iss.NotAfter.Sub(clk.Now()) + time.Minute)
		if err := probe(srv, clientCfg(t, a, cert)); err == nil {
			t.Fatal("an expired certificate was accepted")
		}
	})
}

func TestServerCertificateReissuedBeforeExpiry(t *testing.T) {
	clk := newClock()
	a := newAuthority(t, clk)
	first, err := a.serverCertificate([]string{"127.0.0.1"})
	if err != nil {
		t.Fatal(err)
	}
	same, _ := a.serverCertificate([]string{"127.0.0.1"})
	if first != same {
		t.Fatal("the cached server certificate was not reused")
	}
	clk.Advance(21 * 24 * time.Hour) // past two thirds of 30 days
	next, err := a.serverCertificate([]string{"127.0.0.1"})
	if err != nil {
		t.Fatal(err)
	}
	if next == first {
		t.Fatal("the server certificate was not reissued")
	}
	if _, err := a.ServerTLSConfig(); err == nil {
		t.Fatal("a server config without names must fail")
	}
}

func TestTokens(t *testing.T) {
	clk := newClock()
	a := newAuthority(t, clk)
	_, csr, err := NewCSR("agent-a")
	if err != nil {
		t.Fatal(err)
	}

	t.Run("single use", func(t *testing.T) {
		tok, _, _ := a.NewToken("agent-a", time.Minute)
		if _, err := a.Enroll(tok, csr); err != nil {
			t.Fatal(err)
		}
		if _, err := a.Enroll(tok, csr); !errors.Is(err, ErrTokenUsed) {
			t.Fatalf("reuse = %v, want ErrTokenUsed", err)
		}
	})

	t.Run("expiry", func(t *testing.T) {
		tok, exp, _ := a.NewToken("agent-a", time.Minute)
		clk.Advance(exp.Sub(clk.Now())) // exactly at expiry
		if _, err := a.Enroll(tok, csr); !errors.Is(err, ErrTokenExpired) {
			t.Fatalf("expired = %v, want ErrTokenExpired", err)
		}
	})

	t.Run("wrong secret, unknown id, malformed", func(t *testing.T) {
		tok, _, _ := a.NewToken("agent-a", time.Minute)
		id, secret, _ := splitToken(tok)
		// Change the last character to one it is not, so the wrong secret
		// never equals the real one (a fixed "A" did 1 time in 64).
		last := "A"
		if strings.HasSuffix(secret, "A") {
			last = "B"
		}
		for name, bad := range map[string]string{
			"wrong secret": id + "." + secret[:len(secret)-1] + last,
			"unknown id":   "0000000000000000." + secret,
			"no dot":       id + secret,
			"empty":        "",
			"extra dot":    tok + ".x",
			"long":         id + "." + strings.Repeat("a", 300),
		} {
			if _, err := a.Enroll(bad, csr); !errors.Is(err, ErrInvalidToken) {
				t.Errorf("%s = %v, want ErrInvalidToken", name, err)
			}
		}
		if _, err := a.Enroll(tok, csr); err != nil {
			t.Fatalf("the real token still works after guesses: %v", err)
		}
	})

	t.Run("a bad CSR does not burn the token", func(t *testing.T) {
		tok, _, _ := a.NewToken("agent-a", time.Minute)
		_, other, _ := NewCSR("agent-b")
		if _, err := a.Enroll(tok, other); !errors.Is(err, ErrInvalidCSR) {
			t.Fatalf("CSR for another identity = %v, want ErrInvalidCSR", err)
		}
		if _, err := a.Enroll(tok, []byte("junk")); !errors.Is(err, ErrInvalidCSR) {
			t.Fatalf("junk CSR = %v, want ErrInvalidCSR", err)
		}
		if _, err := a.Enroll(tok, csr); err != nil {
			t.Fatalf("token after rejected CSRs: %v", err)
		}
	})

	t.Run("the certificate takes the identity from the token, not the CSR", func(t *testing.T) {
		tok, _, _ := a.NewToken("agent-a", time.Minute)
		key, _ := rsa.GenerateKey(rand.Reader, 2048)
		rsaCSR, _ := x509.CreateCertificateRequest(rand.Reader, &x509.CertificateRequest{Subject: pkix.Name{CommonName: "agent-a"}}, key)
		if _, err := a.Enroll(tok, rsaCSR); !errors.Is(err, ErrInvalidCSR) {
			t.Fatalf("RSA CSR = %v, want ErrInvalidCSR", err)
		}
		bare, _ := x509.CreateCertificateRequest(rand.Reader, &x509.CertificateRequest{DNSNames: []string{"evil.example"}}, mustKey(t))
		iss, err := a.Enroll(tok, bare)
		if err != nil {
			t.Fatal(err)
		}
		c := parseCert(t, iss.CertPEM)
		if AgentID(c) != "agent-a" || len(c.DNSNames) != 0 || c.IsCA {
			t.Fatalf("certificate = CN %q, DNS %v, CA %v", AgentID(c), c.DNSNames, c.IsCA)
		}
	})

	t.Run("limits", func(t *testing.T) {
		if _, _, err := a.NewToken("bad id!", time.Minute); !errors.Is(err, ErrInvalidID) {
			t.Fatalf("invalid id = %v", err)
		}
		if _, _, err := a.NewToken("agent-a", MaxTokenTTL+time.Second); err == nil {
			t.Fatal("a token above the maximum lifetime was accepted")
		}
		_, exp, err := a.NewToken("agent-a", 0)
		if err != nil || exp.Sub(clk.Now()) != DefaultTokenTTL {
			t.Fatalf("default lifetime = %v, %v", exp.Sub(clk.Now()), err)
		}
	})

	t.Run("concurrent exchange of one token succeeds once", func(t *testing.T) {
		tok, _, _ := a.NewToken("agent-a", time.Minute)
		var ok atomic.Int32
		var wg sync.WaitGroup
		for i := 0; i < 16; i++ {
			wg.Add(1)
			go func() {
				defer wg.Done()
				if _, err := a.Enroll(tok, csr); err == nil {
					ok.Add(1)
				}
			}()
		}
		wg.Wait()
		if ok.Load() != 1 {
			t.Fatalf("%d exchanges succeeded, want 1", ok.Load())
		}
	})
}

func TestRotation(t *testing.T) {
	clk := newClock()
	a := newAuthority(t, clk)
	srv := startServer(t, mustServerCfg(t, a)) // before the clock moves, so clients see a valid server certificate
	_, iss := enrollAgent(t, a, "agent-a")
	old := parseCert(t, iss.CertPEM)

	if got := iss.RenewAt(); !got.After(iss.NotBefore) || !got.Before(iss.NotAfter) {
		t.Fatalf("RenewAt %v is outside the validity", got)
	}
	clk.Advance(20 * time.Hour)
	keyPEM, csr, _ := NewCSR("agent-a")
	next, err := a.Renew(old, csr)
	if err != nil {
		t.Fatalf("Renew: %v", err)
	}
	if next.Serial == iss.Serial || !next.NotAfter.After(iss.NotAfter) || next.AgentID != "agent-a" {
		t.Fatalf("renewed certificate = %+v after %+v", next, iss)
	}
	kp, err := KeyPair(next.CertPEM, keyPEM)
	if err != nil {
		t.Fatal(err)
	}
	if err := probe(srv, clientCfg(t, a, kp)); err != nil {
		t.Fatalf("renewed certificate rejected: %v", err)
	}

	t.Run("an identity cannot renew into another one", func(t *testing.T) {
		_, other, _ := NewCSR("agent-b")
		if _, err := a.Renew(old, other); !errors.Is(err, ErrInvalidCSR) {
			t.Fatalf("= %v, want ErrInvalidCSR", err)
		}
	})
	t.Run("expired certificate cannot renew", func(t *testing.T) {
		clk.Advance(10 * time.Hour)
		if _, err := a.Renew(old, csr); !errors.Is(err, ErrCertificateInvalid) {
			t.Fatalf("= %v, want ErrCertificateInvalid", err)
		}
	})
	t.Run("revoked identity cannot renew", func(t *testing.T) {
		if _, err := a.Revoke("agent-a"); err != nil {
			t.Fatal(err)
		}
		if _, err := a.Renew(parseCert(t, next.CertPEM), csr); !errors.Is(err, ErrRevoked) {
			t.Fatalf("= %v, want ErrRevoked", err)
		}
	})
	t.Run("foreign certificate cannot renew", func(t *testing.T) {
		other := newAuthority(t, clk)
		_, oi := enrollAgent(t, other, "agent-a")
		if _, err := a.Renew(parseCert(t, oi.CertPEM), csr); !errors.Is(err, ErrCertificateInvalid) {
			t.Fatalf("= %v, want ErrCertificateInvalid", err)
		}
		if _, err := a.Renew(nil, csr); !errors.Is(err, ErrCertificateInvalid) {
			t.Fatalf("nil = %v", err)
		}
	})
}

func TestRevokeReenrollAndHook(t *testing.T) {
	clk := newClock()
	var hooked []string
	a, err := Open(Config{Now: clk.Now, OnRevoke: func(id string) { hooked = append(hooked, id) }})
	if err != nil {
		t.Fatal(err)
	}
	_, one := enrollAgent(t, a, "agent-a")
	_, two := enrollAgent(t, a, "agent-a") // two certificates, both end
	pending, _, _ := a.NewToken("agent-a", time.Minute)
	other, _ := enrollAgent(t, a, "agent-b")

	if n, err := a.Revoke("agent-a"); err != nil || n != 2 {
		t.Fatalf("Revoke = %d, %v; want 2", n, err)
	}
	if len(hooked) != 1 || hooked[0] != "agent-a" {
		t.Fatalf("OnRevoke calls = %v", hooked)
	}
	for _, i := range []*Issued{one, two} {
		if err := a.Authorize(parseCert(t, i.CertPEM)); !errors.Is(err, ErrRevoked) {
			t.Fatalf("Authorize after revoke = %v", err)
		}
	}
	_, csr, _ := NewCSR("agent-a")
	if _, err := a.Enroll(pending, csr); !errors.Is(err, ErrInvalidToken) {
		t.Fatalf("a token pending at revocation = %v, want ErrInvalidToken", err)
	}
	if err := a.Authorize(leafOf(t, other)); err != nil {
		t.Fatalf("revoking agent-a ended agent-b: %v", err)
	}
	// An operator re-enrolls the identity with a new token.
	if _, iss := enrollAgent(t, a, "agent-a"); a.Authorize(parseCert(t, iss.CertPEM)) != nil {
		t.Fatal("a re-enrolled identity was refused")
	}
}

// leafOf parses the leaf of a tls.Certificate.
func leafOf(t *testing.T, c *tls.Certificate) *x509.Certificate {
	t.Helper()
	l, err := x509.ParseCertificate(c.Certificate[0])
	if err != nil {
		t.Fatal(err)
	}
	return l
}

func mustKey(t *testing.T) *ecdsa.PrivateKey {
	t.Helper()
	k, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	return k
}

func parseCert(t *testing.T, certPEM []byte) *x509.Certificate {
	t.Helper()
	b, _ := pem.Decode(certPEM)
	if b == nil {
		t.Fatal("no PEM block")
	}
	c, err := x509.ParseCertificate(b.Bytes)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func mustServerCfg(t *testing.T, a *Authority) *tls.Config {
	t.Helper()
	cfg, err := a.ServerTLSConfig("127.0.0.1")
	if err != nil {
		t.Fatal(err)
	}
	return cfg
}
