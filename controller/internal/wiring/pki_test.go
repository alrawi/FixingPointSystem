// SPDX-License-Identifier: GPL-3.0-only

package wiring

import (
	"bytes"
	"context"
	"crypto/tls"
	"encoding/json"
	"encoding/pem"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/status"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/app"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/pki"
)

// testClock is an agents.Clock that moves only when a test advances it. It
// starts at the wall clock, so the client side of a handshake, which reads the
// real time, accepts the certificates of the controller.
type testClock struct {
	mu sync.Mutex
	t  time.Time
}

func newTestClock() *testClock { return &testClock{t: time.Now()} }

func (c *testClock) Now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.t
}

func (c *testClock) Advance(d time.Duration) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.t = c.t.Add(d)
}

// tlsController is one controller that serves mutual TLS on loopback ports.
type tlsController struct {
	*Controller
	clock      *testClock
	grpcAddr   string
	enrollAddr string
	socket     string
	caPEM      []byte
}

// startTLS starts a controller in the default secure mode with the gRPC
// service, the enrollment service, and the admin socket on loopback.
func startTLS(t *testing.T, mutate func(*app.Config)) *tlsController {
	t.Helper()
	cfg := testConfig(t, t.TempDir())
	cfg.InsecurePlainGRPC = false
	cfg.EnrollListen = "127.0.0.1:0"
	// A short path keeps the socket below the limit of sockaddr_un.
	dir, err := os.MkdirTemp("", "fps")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.RemoveAll(dir) })
	cfg.AdminSocket = filepath.Join(dir, "a.sock")
	if mutate != nil {
		mutate(&cfg)
	}
	if err := cfg.Validate(); err != nil {
		t.Fatal(err)
	}
	clock := newTestClock()
	ctx, cancel := context.WithCancel(context.Background())
	// A long sweep period and heartbeat timeout keep the background sweep from
	// ending sessions while a test moves the clock. Tests call Sweep.
	c, err := New(ctx, Options{Config: cfg, Clock: clock, HeartbeatInterval: time.Hour, HeartbeatTimeout: 2 * time.Hour})
	if err != nil {
		cancel()
		t.Fatalf("New error = %v", err)
	}
	gln, err := c.ListenGRPC(ctx)
	if err != nil {
		t.Fatal(err)
	}
	eln, err := c.ListenEnroll(ctx)
	if err != nil {
		t.Fatal(err)
	}
	aln, err := c.ListenAdmin(ctx)
	if err != nil {
		t.Fatal(err)
	}
	var wg sync.WaitGroup
	wg.Add(2)
	go func() { defer wg.Done(); _ = c.Run(ctx, gln) }()
	go func() { defer wg.Done(); _ = c.RunPKI(ctx, eln, aln) }()
	t.Cleanup(func() {
		cancel()
		wg.Wait()
		if err := c.Close(); err != nil {
			t.Errorf("Close error = %v", err)
		}
	})
	return &tlsController{
		Controller: c,
		clock:      clock,
		grpcAddr:   gln.Addr().String(),
		enrollAddr: eln.Addr().String(),
		socket:     cfg.AdminSocket,
		caPEM:      c.PKI.CACertPEM(),
	}
}

// dialer dials the gRPC service over TLS and presents cert, if any.
func (h *tlsController) dialer(caPEM []byte, cert *tls.Certificate) fakeagent.Dialer {
	return func(context.Context) (*grpc.ClientConn, error) {
		cfg, err := pki.ClientTLSConfig(caPEM, cert, "localhost")
		if err != nil {
			return nil, err
		}
		return grpc.NewClient(h.grpcAddr, grpc.WithTransportCredentials(credentials.NewTLS(cfg)))
	}
}

// enroll exchanges token for a certificate at the enrollment service and
// returns the certificate and the HTTP status.
func (h *tlsController) enroll(t *testing.T, token, id string) (*tls.Certificate, int) {
	t.Helper()
	keyPEM, csrDER, err := pki.NewCSR(id)
	if err != nil {
		t.Fatal(err)
	}
	body, err := json.Marshal(pki.EnrollRequest{
		Token:  token,
		CSRPEM: string(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE REQUEST", Bytes: csrDER})),
	})
	if err != nil {
		t.Fatal(err)
	}
	tlsCfg, err := pki.ClientTLSConfig(h.caPEM, nil, "localhost")
	if err != nil {
		t.Fatal(err)
	}
	hc := &http.Client{Timeout: 10 * time.Second, Transport: &http.Transport{TLSClientConfig: tlsCfg}}
	defer hc.CloseIdleConnections()
	resp, err := hc.Post("https://"+h.enrollAddr+"/enroll", "application/json", bytes.NewReader(body))
	if err != nil {
		t.Fatalf("POST /enroll: %v", err)
	}
	defer func() { _ = resp.Body.Close() }()
	if resp.StatusCode != http.StatusOK {
		return nil, resp.StatusCode
	}
	var issued pki.IssuedResponse
	if err := json.NewDecoder(resp.Body).Decode(&issued); err != nil {
		t.Fatal(err)
	}
	cert, err := pki.KeyPair([]byte(issued.CertificatePEM), keyPEM)
	if err != nil {
		t.Fatal(err)
	}
	return cert, http.StatusOK
}

// enrolled mints a token for id, enrolls over HTTPS, and returns the certificate.
func (h *tlsController) enrolled(t *testing.T, id string) *tls.Certificate {
	t.Helper()
	token, _, err := h.MintToken(id, 0)
	if err != nil {
		t.Fatal(err)
	}
	cert, code := h.enroll(t, token, id)
	if cert == nil {
		t.Fatalf("enrollment of %s answered %d", id, code)
	}
	return cert
}

func startAgentTLS(t *testing.T, h *tlsController, id string, cert *tls.Certificate) (*fakeagent.Agent, error) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	ag, err := fakeagent.Start(ctx, fakeagent.Options{ID: id, Dial: h.dialer(h.caPEM, cert)})
	if err == nil {
		t.Cleanup(ag.Close)
	}
	return ag, err
}

func waitFor(t *testing.T, what string, ok func() bool) {
	t.Helper()
	deadline := time.Now().Add(5 * time.Second)
	for !ok() {
		if time.Now().After(deadline) {
			t.Fatalf("timed out waiting for %s", what)
		}
		time.Sleep(5 * time.Millisecond)
	}
}

// TestMutualTLSAdmitsOnlyAgentsWithAValidCertificate is the acceptance test of
// CTL-27 at the level of the controller: a token enrolls an agent, the agent
// registers over mutual TLS, and an agent without a valid certificate cannot,
// whether it has none, one of another CA, one for another identity, or one that
// an operator revoked.
func TestMutualTLSAdmitsOnlyAgentsWithAValidCertificate(t *testing.T) {
	h := startTLS(t, nil)

	// The token mint admits the identity to the inventory.
	cert := h.enrolled(t, "agent-a")
	if a, err := h.Agents.Get("agent-a"); err != nil || !a.Enrolled {
		t.Fatalf("inventory after minting = %+v, %v, want agent-a enrolled", a, err)
	}
	// Enrollment tokens are single use.
	token, _, err := h.MintToken("agent-z", 0)
	if err != nil {
		t.Fatal(err)
	}
	if _, st := h.enroll(t, token, "agent-z"); st != http.StatusOK {
		t.Fatalf("first use of a token answered %d, want 200", st)
	}
	if _, st := h.enroll(t, token, "agent-z"); st != http.StatusUnauthorized {
		t.Errorf("second use of a token answered %d, want 401", st)
	}

	ag, err := startAgentTLS(t, h, "agent-a", cert)
	if err != nil {
		t.Fatalf("agent with a valid certificate: %v", err)
	}
	if a, err := h.Agents.Get("agent-a"); err != nil || a.State != agents.StateOnline {
		t.Fatalf("agent-a = %+v, %v, want online", a, err)
	}

	// An agent without a certificate never completes a call.
	if _, err := startAgentTLS(t, h, "agent-b", nil); err == nil {
		t.Error("an agent without a client certificate registered")
	}
	// A certificate of another CA, even with the right name.
	other, err := pki.Open(pki.Config{})
	if err != nil {
		t.Fatal(err)
	}
	otherToken, _, err := other.NewToken("agent-a", 0)
	if err != nil {
		t.Fatal(err)
	}
	keyPEM, csr, err := pki.NewCSR("agent-a")
	if err != nil {
		t.Fatal(err)
	}
	issued, err := other.Enroll(otherToken, csr)
	if err != nil {
		t.Fatal(err)
	}
	foreign, err := pki.KeyPair(issued.CertPEM, keyPEM)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := startAgentTLS(t, h, "agent-a", foreign); err == nil {
		t.Error("an agent with a certificate of another CA registered")
	}
	// A valid certificate cannot speak for another identity.
	if _, err := startAgentTLS(t, h, "agent-c", cert); status.Code(err) != codes.Unauthenticated {
		t.Errorf("agent-a certificate registering as agent-c = %v, want Unauthenticated", err)
	}
	// A valid certificate of an identity that no operator admitted. Without
	// agent_auto_enroll the manager refuses it.
	token, _, err = h.PKI.NewToken("agent-d", 0)
	if err != nil {
		t.Fatal(err)
	}
	certD, st := h.enroll(t, token, "agent-d")
	if certD == nil {
		t.Fatalf("enrollment of agent-d answered %d", st)
	}
	if _, err := startAgentTLS(t, h, "agent-d", certD); err == nil || !strings.Contains(err.Error(), "not enrolled") {
		t.Errorf("registration of an identity outside the inventory = %v, want not enrolled", err)
	}

	// Revocation ends the open session at once and keeps the agent out.
	if n, err := h.RevokeIdentity("agent-a"); err != nil || n != 1 {
		t.Fatalf("RevokeIdentity = %d, %v, want 1 certificate", n, err)
	}
	select {
	case <-ag.Done():
	case <-time.After(5 * time.Second):
		t.Fatal("the session of the revoked agent stayed open")
	}
	if got := status.Code(ag.Err()); got != codes.Unauthenticated {
		t.Errorf("session of the revoked agent ended with %v (%v), want Unauthenticated", got, ag.Err())
	}
	waitFor(t, "agent-a offline", func() bool {
		a, err := h.Agents.Get("agent-a")
		return err == nil && a.State == agents.StateOffline
	})
	if _, err := startAgentTLS(t, h, "agent-a", cert); err == nil {
		t.Error("a revoked agent registered again")
	}
	// A new token enrolls the identity again.
	if _, err := startAgentTLS(t, h, "agent-a", h.enrolled(t, "agent-a")); err != nil {
		t.Errorf("agent-a after a new enrollment: %v", err)
	}
}

// TestAutoEnrollAdmitsCertificateHolders shows what agent_auto_enroll means
// with certificates: the certificate proves the identity, and the setting
// decides whether the inventory must also list it.
func TestAutoEnrollAdmitsCertificateHolders(t *testing.T) {
	h := startTLS(t, func(c *app.Config) { c.AgentAutoEnroll = true })
	token, _, err := h.PKI.NewToken("agent-e", 0)
	if err != nil {
		t.Fatal(err)
	}
	cert, st := h.enroll(t, token, "agent-e")
	if cert == nil {
		t.Fatalf("enrollment answered %d", st)
	}
	if _, err := startAgentTLS(t, h, "agent-e", cert); err != nil {
		t.Fatalf("a certificate holder outside the inventory with agent_auto_enroll: %v", err)
	}
	// Auto enroll never lets an agent without a certificate in.
	if _, err := startAgentTLS(t, h, "agent-f", nil); err == nil {
		t.Error("agent_auto_enroll admitted an agent without a certificate")
	}
}

// TestExpiredCertificateEndsAnIdleAgentSession moves the clock past the
// lifetime of a client certificate. The open session survives the handshake
// check, so the sweep ends it once the agent is idle, and the agent needs a
// new enrollment because an expired certificate cannot renew.
func TestExpiredCertificateEndsAnIdleAgentSession(t *testing.T) {
	h := startTLS(t, nil)
	cert := h.enrolled(t, "agent-a")
	ag, err := startAgentTLS(t, h, "agent-a", cert)
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	h.clock.Advance(pki.DefaultClientLifetime + time.Hour)
	if err := ag.Heartbeat(ctx); err != nil { // keeps the heartbeat timeout out of the way
		t.Fatal(err)
	}
	if ids := h.Agents.Sweep(); len(ids) != 1 || ids[0] != "agent-a" {
		t.Fatalf("Sweep after the expiry ended %v, want agent-a", ids)
	}
	select {
	case <-ag.Done():
	case <-time.After(5 * time.Second):
		t.Fatal("the session of the expired agent stayed open")
	}
	if got := status.Code(ag.Err()); got != codes.Unauthenticated {
		t.Errorf("session ended with %v (%v), want Unauthenticated", got, ag.Err())
	}
	if _, err := startAgentTLS(t, h, "agent-a", cert); err == nil {
		t.Error("an agent registered with an expired certificate")
	}
	if _, err := startAgentTLS(t, h, "agent-a", h.enrolled(t, "agent-a")); err != nil {
		t.Errorf("agent-a after a new enrollment: %v", err)
	}
}

// TestPlainModeIsExplicit shows that the plain mode serves gRPC without a
// certificate authority, and that the PKI operations refuse to run in it.
func TestPlainModeIsExplicit(t *testing.T) {
	cfg := testConfig(t, t.TempDir())
	cfg.AgentAutoEnroll = true
	c := newController(t, cfg)
	if c.PKI != nil {
		t.Fatal("plain mode opened a certificate authority")
	}
	if _, _, err := c.MintToken("agent-a", 0); !errors.Is(err, ErrNoPKI) {
		t.Errorf("MintToken = %v, want ErrNoPKI", err)
	}
	if _, err := c.RevokeIdentity("agent-a"); !errors.Is(err, ErrNoPKI) {
		t.Errorf("RevokeIdentity = %v, want ErrNoPKI", err)
	}
	ctx := context.Background()
	if ln, err := c.ListenEnroll(ctx); ln != nil || err != nil {
		t.Errorf("ListenEnroll = %v, %v, want nil, nil", ln, err)
	}
	if ln, err := c.ListenAdmin(ctx); ln != nil || err != nil {
		t.Errorf("ListenAdmin = %v, %v, want nil, nil", ln, err)
	}
	// Plain mode leaves no PKI state behind.
	if _, err := os.Stat(filepath.Join(cfg.DataDir, app.PKIDirName)); !errors.Is(err, os.ErrNotExist) {
		t.Errorf("plain mode created the PKI directory: %v", err)
	}
}

// TestSecureModeRefusesPlainClients shows that a client without TLS cannot
// register in the default mode.
func TestSecureModeRefusesPlainClients(t *testing.T) {
	h := startTLS(t, func(c *app.Config) { c.AgentAutoEnroll = true })
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_, err := fakeagent.Start(ctx, fakeagent.Options{ID: "agent-a", Dial: func(context.Context) (*grpc.ClientConn, error) {
		return grpc.NewClient(h.grpcAddr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	}})
	if err == nil {
		t.Fatal("a plain client registered with a TLS controller")
	}
	if len(h.Agents.List()) != 0 {
		t.Errorf("inventory = %v, want empty", h.Agents.List())
	}
}

// TestAdminSocket covers the operator path: the token and revoke operations
// run in the controller over its socket, the socket accepts only its owner, a
// second controller on the same socket is refused, and a stale socket file of
// a dead controller is replaced.
func TestAdminSocket(t *testing.T) {
	h := startTLS(t, nil)
	info, err := os.Stat(h.socket)
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm() != 0o600 {
		t.Errorf("admin socket mode = %v, want 0600", info.Mode().Perm())
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	client := AdminClient{Socket: h.socket}

	tok, err := client.MintToken(ctx, "agent-a", 30*time.Minute)
	if err != nil {
		t.Fatal(err)
	}
	if tok.AgentID != "agent-a" || tok.Token == "" || time.Until(tok.Expires) > 31*time.Minute {
		t.Errorf("token response = %+v", tok)
	}
	cert, st := h.enroll(t, tok.Token, "agent-a")
	if cert == nil {
		t.Fatalf("enrollment with a minted token answered %d", st)
	}
	ag, err := startAgentTLS(t, h, "agent-a", cert)
	if err != nil {
		t.Fatal(err)
	}
	rev, err := client.Revoke(ctx, "agent-a")
	if err != nil || rev.Certificates != 1 {
		t.Fatalf("Revoke = %+v, %v, want 1 certificate", rev, err)
	}
	select {
	case <-ag.Done():
	case <-time.After(5 * time.Second):
		t.Fatal("revocation through the socket left the session open")
	}
	if _, err := client.MintToken(ctx, "bad id", 0); err == nil || !strings.Contains(err.Error(), "refused") {
		t.Errorf("MintToken of an invalid id = %v, want a refusal", err)
	}
	if _, err := client.MintToken(ctx, "agent-a", 30*24*time.Hour); err == nil {
		t.Error("MintToken beyond the maximum lifetime succeeded")
	}
	if _, err := (AdminClient{Socket: filepath.Join(t.TempDir(), "absent.sock")}).Revoke(ctx, "agent-a"); !errors.Is(err, ErrControllerNotRunning) {
		t.Errorf("Revoke without a controller = %v, want ErrControllerNotRunning", err)
	}

	// A second controller must not take over a live socket.
	cfg := testConfig(t, t.TempDir())
	cfg.InsecurePlainGRPC = false
	cfg.AdminSocket = h.socket
	second := newController(t, cfg)
	if _, err := second.ListenAdmin(ctx); err == nil || !strings.Contains(err.Error(), "another controller") {
		t.Errorf("ListenAdmin on a live socket = %v, want a refusal", err)
	}
}

func TestAdminSocketReplacesAStaleFile(t *testing.T) {
	dir, err := os.MkdirTemp("", "fps")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.RemoveAll(dir) })
	sock := filepath.Join(dir, "a.sock")
	cfg := testConfig(t, t.TempDir())
	cfg.InsecurePlainGRPC = false
	cfg.AdminSocket = sock
	c := newController(t, cfg)

	// A dead controller leaves its socket file behind.
	first, err := c.ListenAdmin(context.Background())
	if err != nil || first == nil {
		t.Fatalf("ListenAdmin = %v, %v", first, err)
	}
	if ul, ok := first.(interface{ SetUnlinkOnClose(bool) }); ok {
		ul.SetUnlinkOnClose(false)
	}
	_ = first.Close()
	if _, err := os.Lstat(sock); err != nil {
		t.Fatalf("no stale socket file to replace: %v", err)
	}
	second, err := c.ListenAdmin(context.Background())
	if err != nil {
		t.Fatalf("ListenAdmin over a stale socket: %v", err)
	}
	_ = second.Close()

	// A regular file at the path is not ours to remove.
	if err := os.WriteFile(sock, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := c.ListenAdmin(context.Background()); err == nil || !strings.Contains(err.Error(), "not a socket") {
		t.Errorf("ListenAdmin over a regular file = %v, want a refusal", err)
	}
}

// TestAdminHandlerValidatesRequests calls the admin handler directly.
func TestAdminHandlerValidatesRequests(t *testing.T) {
	h := startTLS(t, nil)
	handler := h.AdminHandler()
	tests := []struct {
		name, path, body string
		want             int
	}{
		{"token", AdminTokenPath, `{"agent_id":"agent-a","ttl":"5m"}`, http.StatusOK},
		{"token without ttl", AdminTokenPath, `{"agent_id":"agent-a"}`, http.StatusOK},
		{"bad ttl", AdminTokenPath, `{"agent_id":"agent-a","ttl":"soon"}`, http.StatusBadRequest},
		{"negative ttl", AdminTokenPath, `{"agent_id":"agent-a","ttl":"-1m"}`, http.StatusBadRequest},
		{"bad id", AdminTokenPath, `{"agent_id":"a/b"}`, http.StatusBadRequest},
		{"unknown field", AdminTokenPath, `{"agent_id":"agent-a","admin":true}`, http.StatusBadRequest},
		{"not json", AdminTokenPath, `agent-a`, http.StatusBadRequest},
		{"revoke", AdminRevokePath, `{"agent_id":"agent-a"}`, http.StatusOK},
		{"revoke bad id", AdminRevokePath, `{"agent_id":""}`, http.StatusBadRequest},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			rec := httptest.NewRecorder()
			handler.ServeHTTP(rec, httptest.NewRequest(http.MethodPost, tc.path, strings.NewReader(tc.body)))
			if rec.Code != tc.want {
				t.Errorf("status %d, want %d; body %s", rec.Code, tc.want, rec.Body)
			}
		})
	}
	rec := httptest.NewRecorder()
	handler.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, AdminTokenPath, nil))
	if rec.Code != http.StatusMethodNotAllowed {
		t.Errorf("GET %s = %d, want 405", AdminTokenPath, rec.Code)
	}
}
