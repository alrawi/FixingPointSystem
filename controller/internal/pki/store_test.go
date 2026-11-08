// SPDX-License-Identifier: GPL-3.0-only

package pki

import (
	"bytes"
	"crypto/tls"
	"encoding/json"
	"encoding/pem"
	"errors"
	"io"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestCAIsGeneratedOnceWithPrivateModes(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "pki")
	a, err := Open(Config{Dir: dir})
	if err != nil {
		t.Fatal(err)
	}
	for name, want := range map[string]os.FileMode{CAKeyFile: 0o600, CACertFile: 0o644} {
		info, err := os.Stat(filepath.Join(dir, name))
		if err != nil {
			t.Fatal(err)
		}
		if info.Mode().Perm() != want {
			t.Errorf("%s mode = %04o, want %04o", name, info.Mode().Perm(), want)
		}
	}
	if info, _ := os.Stat(dir); info.Mode().Perm() != 0o700 {
		t.Errorf("directory mode = %04o, want 0700", info.Mode().Perm())
	}
	b, err := Open(Config{Dir: dir})
	if err != nil {
		t.Fatal(err)
	}
	if a.CAFingerprint() != b.CAFingerprint() || len(a.CAFingerprint()) != 64 {
		t.Fatalf("second start changed the CA: %s vs %s", a.CAFingerprint(), b.CAFingerprint())
	}
	if !bytes.Equal(a.CACertPEM(), b.CACertPEM()) {
		t.Fatal("CA certificate differs after restart")
	}
}

func TestOpenRefusesAnUntrustworthyDirectory(t *testing.T) {
	fresh := func() string {
		dir := filepath.Join(t.TempDir(), "pki")
		if _, err := Open(Config{Dir: dir}); err != nil {
			t.Fatal(err)
		}
		return dir
	}
	t.Run("key readable by others", func(t *testing.T) {
		dir := fresh()
		if err := os.Chmod(filepath.Join(dir, CAKeyFile), 0o640); err != nil { //nolint:gosec // G302: the test makes the key readable on purpose
			t.Fatal(err)
		}
		if _, err := Open(Config{Dir: dir}); err == nil || !strings.Contains(err.Error(), "chmod 600") {
			t.Fatalf("err = %v", err)
		}
	})
	t.Run("only the key", func(t *testing.T) {
		dir := fresh()
		_ = os.Remove(filepath.Join(dir, CACertFile))
		if _, err := Open(Config{Dir: dir}); err == nil {
			t.Fatal("half a CA was accepted")
		}
	})
	t.Run("certificate and key of different CAs", func(t *testing.T) {
		dir := fresh()
		other, err := Open(Config{}) // a second, in-memory CA
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Clean(filepath.Join(dir, CACertFile)), other.CACertPEM(), 0o600); err != nil {
			t.Fatal(err)
		}
		if _, err := Open(Config{Dir: dir}); err == nil || !strings.Contains(err.Error(), "does not match") {
			t.Fatalf("err = %v", err)
		}
	})
	t.Run("damaged state", func(t *testing.T) {
		dir := fresh()
		if err := os.WriteFile(filepath.Clean(filepath.Join(dir, StateFile)), []byte("{"), 0o600); err != nil {
			t.Fatal(err)
		}
		if _, err := Open(Config{Dir: dir}); err == nil {
			t.Fatal("a damaged state file was accepted")
		}
	})
	t.Run("expired CA", func(t *testing.T) {
		dir := fresh()
		late := func() time.Time { return time.Now().Add(11 * 365 * 24 * time.Hour) }
		if _, err := Open(Config{Dir: dir, Now: late}); err == nil || !strings.Contains(err.Error(), "expired") {
			t.Fatalf("err = %v", err)
		}
	})
}

func TestStatePersistsAndNeverHoldsTheTokenSecret(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "pki")
	a, err := Open(Config{Dir: dir})
	if err != nil {
		t.Fatal(err)
	}
	tok, _, _ := a.NewToken("agent-a", time.Hour)
	_, secret, _ := splitToken(tok)
	_, csr, _ := NewCSR("agent-a")
	if _, err := a.Enroll(tok, csr); err != nil {
		t.Fatal(err)
	}
	live, _, _ := a.NewToken("agent-c", time.Hour)
	cert, _ := enrollAgent(t, a, "agent-b")
	if _, err := a.Revoke("agent-b"); err != nil {
		t.Fatal(err)
	}

	raw, err := os.ReadFile(filepath.Clean(filepath.Join(dir, StateFile)))
	if err != nil {
		t.Fatal(err)
	}
	_, liveSecret, _ := splitToken(live)
	if bytes.Contains(raw, []byte(secret)) || bytes.Contains(raw, []byte(liveSecret)) {
		t.Fatal("state.json holds a token secret")
	}
	if info, _ := os.Stat(filepath.Join(dir, StateFile)); info.Mode().Perm() != 0o600 {
		t.Fatalf("state mode = %04o", info.Mode().Perm())
	}

	b, err := Open(Config{Dir: dir})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := b.Enroll(tok, csr); !errors.Is(err, ErrTokenUsed) {
		t.Fatalf("a used token after restart = %v, want ErrTokenUsed", err)
	}
	if err := b.Authorize(leafOf(t, cert)); !errors.Is(err, ErrRevoked) {
		t.Fatalf("a revoked identity after restart = %v, want ErrRevoked", err)
	}
	_, csrC, _ := NewCSR("agent-c")
	if _, err := b.Enroll(live, csrC); err != nil {
		t.Fatalf("an unused token after restart: %v", err)
	}
}

// enrollServer serves Handler behind EnrollTLSConfig on loopback.
func enrollServer(t *testing.T, a *Authority) string {
	t.Helper()
	cfg, err := a.EnrollTLSConfig("127.0.0.1")
	if err != nil {
		t.Fatal(err)
	}
	ln, err := tls.Listen("tcp", "127.0.0.1:0", cfg)
	if err != nil {
		t.Fatal(err)
	}
	srv := &http.Server{Handler: a.Handler(), ReadHeaderTimeout: 5 * time.Second, ErrorLog: log.New(io.Discard, "", 0)}
	go func() { _ = srv.Serve(ln) }()
	t.Cleanup(func() { _ = srv.Close() })
	return "https://" + ln.Addr().String()
}

func httpClient(cfg *tls.Config) *http.Client {
	return &http.Client{Timeout: 5 * time.Second, Transport: &http.Transport{TLSClientConfig: cfg}}
}

func post(t *testing.T, base, path string, cfg *tls.Config, body any) (int, IssuedResponse) {
	t.Helper()
	data, _ := json.Marshal(body)
	cl := httpClient(cfg)
	defer cl.CloseIdleConnections()
	resp, err := cl.Post(base+path, "application/json", bytes.NewReader(data))
	if err != nil {
		t.Fatalf("POST %s: %v", path, err)
	}
	defer func() { _ = resp.Body.Close() }()
	raw, _ := io.ReadAll(resp.Body)
	var out IssuedResponse
	_ = json.Unmarshal(raw, &out)
	return resp.StatusCode, out
}

func csrPEM(t *testing.T, id string) (key, csr string) {
	t.Helper()
	k, der, err := NewCSR(id)
	if err != nil {
		t.Fatal(err)
	}
	return string(k), string(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE REQUEST", Bytes: der}))
}

func TestHandlerEnrollAndRenewOverTLS(t *testing.T) {
	clk := newClock()
	a := newAuthority(t, clk)
	base := enrollServer(t, a)
	anon := clientCfg(t, a, nil)

	tok, _, _ := a.NewToken("agent-a", time.Minute)
	keyPEM, csr := csrPEM(t, "agent-a")
	code, got := post(t, base, "/enroll", anon, EnrollRequest{Token: tok, CSRPEM: csr})
	if code != 200 || got.AgentID != "agent-a" || got.CAPEM == "" || !got.RenewAt.Before(got.NotAfter) {
		t.Fatalf("enroll = %d %+v", code, got)
	}
	if code, _ := post(t, base, "/enroll", anon, EnrollRequest{Token: tok, CSRPEM: csr}); code != 401 {
		t.Fatalf("a reused token answered %d, want 401", code)
	}
	if code, _ := post(t, base, "/enroll", anon, EnrollRequest{Token: "x.y", CSRPEM: csr}); code != 401 {
		t.Fatalf("a bad token answered %d, want 401", code)
	}
	tok2, _, _ := a.NewToken("agent-a", time.Minute)
	if code, _ := post(t, base, "/enroll", anon, EnrollRequest{Token: tok2, CSRPEM: "junk"}); code != 400 {
		t.Fatalf("a junk CSR answered %d, want 400", code)
	}
	if code, _ := post(t, base, "/enroll", anon, map[string]any{"token": tok2, "extra": 1}); code != 400 {
		t.Fatalf("an unknown field answered %d, want 400", code)
	}

	// Renew with the certificate that enrollment returned.
	kp, err := KeyPair([]byte(got.CertificatePEM), []byte(keyPEM))
	if err != nil {
		t.Fatal(err)
	}
	mtls := clientCfg(t, a, kp)
	newKey, newCSR := csrPEM(t, "agent-a")
	code, renewed := post(t, base, "/renew", mtls, RenewRequest{CSRPEM: newCSR})
	if code != 200 || renewed.CertificatePEM == got.CertificatePEM {
		t.Fatalf("renew = %d", code)
	}
	if _, err := KeyPair([]byte(renewed.CertificatePEM), []byte(newKey)); err != nil {
		t.Fatalf("the renewed certificate does not match the new key: %v", err)
	}
	if code, _ := post(t, base, "/renew", anon, RenewRequest{CSRPEM: newCSR}); code != 401 {
		t.Fatalf("renew without a certificate answered %d, want 401", code)
	}
	_, otherCSR := csrPEM(t, "agent-b")
	if code, _ := post(t, base, "/renew", mtls, RenewRequest{CSRPEM: otherCSR}); code != 400 {
		t.Fatalf("renew for another identity answered %d, want 400", code)
	}

	req, _ := http.NewRequest(http.MethodGet, base+"/enroll", nil)
	cl := httpClient(anon)
	resp, err := cl.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	_ = resp.Body.Close()
	if resp.StatusCode != http.StatusMethodNotAllowed {
		t.Fatalf("GET /enroll answered %d, want 405", resp.StatusCode)
	}

	// After revocation the handshake of the enrollment listener fails.
	if _, err := a.Revoke("agent-a"); err != nil {
		t.Fatal(err)
	}
	body, _ := json.Marshal(RenewRequest{CSRPEM: newCSR})
	rc := httpClient(mtls)
	defer rc.CloseIdleConnections()
	if resp, err := rc.Post(base+"/renew", "application/json", bytes.NewReader(body)); err == nil {
		_ = resp.Body.Close()
		t.Fatalf("a revoked identity renewed with status %d", resp.StatusCode)
	}
}
