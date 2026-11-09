// SPDX-License-Identifier: GPL-3.0-only

package main

import (
	"bytes"
	"context"
	"crypto/tls"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/credentials/insecure"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/app"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/pki"
)

func TestLoadConfigLayersTLSSettings(t *testing.T) {
	path := writeConfig(t, "enroll_listen: 127.0.0.1:7101\npki_dir: /from/file\ntls_names: [a.example]\nadmin_socket: /from/file.sock\n")
	tests := []struct {
		name         string
		args         []string
		wantEnroll   string
		wantPKI      string
		wantNames    []string
		wantSocket   string
		wantInsecure bool
	}{
		{"defaults", nil, app.DefaultEnrollListen, "", nil, "", false},
		{"file", []string{"-config", path}, "127.0.0.1:7101", "/from/file", []string{"a.example"}, "/from/file.sock", false},
		{"flags over file", []string{"-config", path, "-enroll-listen", "127.0.0.1:7102", "-pki-dir", "/from/flag", "-tls-names", "b.example, 10.0.0.5,,", "-admin-socket", "/from/flag.sock", "-insecure-plain-grpc"},
			"127.0.0.1:7102", "/from/flag", []string{"b.example", "10.0.0.5"}, "/from/flag.sock", true},
		{"data dir sets the defaults", []string{"-data-dir", "/d"}, app.DefaultEnrollListen, "/d/pki", nil, "/d/pki/admin.sock", false},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			o, err := parseFlags(tc.args, io.Discard)
			if err != nil {
				t.Fatal(err)
			}
			cfg, err := loadConfig(o)
			if err != nil {
				t.Fatal(err)
			}
			if cfg.EnrollListen != tc.wantEnroll || cfg.ResolvedPKIDir() != tc.wantPKI || cfg.ResolvedAdminSocket() != tc.wantSocket || cfg.InsecurePlainGRPC != tc.wantInsecure {
				t.Errorf("config = %q %q %q %v, want %q %q %q %v", cfg.EnrollListen, cfg.ResolvedPKIDir(), cfg.ResolvedAdminSocket(), cfg.InsecurePlainGRPC,
					tc.wantEnroll, tc.wantPKI, tc.wantSocket, tc.wantInsecure)
			}
			if fmt.Sprint(cfg.TLSNames) != fmt.Sprint(tc.wantNames) {
				t.Errorf("TLSNames = %v, want %v", cfg.TLSNames, tc.wantNames)
			}
		})
	}
}

// tlsDial dials the gRPC service of the controller over TLS and presents cert,
// if any. The controller certificate names localhost.
func tlsDial(addr string, caPEM []byte, cert *tls.Certificate) fakeagent.Dialer {
	return func(context.Context) (*grpc.ClientConn, error) {
		cfg, err := pki.ClientTLSConfig(caPEM, cert, "localhost")
		if err != nil {
			return nil, err
		}
		return grpc.NewClient(addr, grpc.WithTransportCredentials(credentials.NewTLS(cfg)))
	}
}

// enrollHTTPS exchanges token for a certificate at the enrollment service, the
// way the agent does, and returns the certificate, or nil and the HTTP status.
func enrollHTTPS(t *testing.T, addr string, caPEM []byte, token, id string) (*tls.Certificate, int) {
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
	tlsCfg, err := pki.ClientTLSConfig(caPEM, nil, "localhost")
	if err != nil {
		t.Fatal(err)
	}
	hc := &http.Client{Timeout: 10 * time.Second, Transport: &http.Transport{TLSClientConfig: tlsCfg}}
	defer hc.CloseIdleConnections()
	resp, err := hc.Post("https://"+addr+"/enroll", "application/json", bytes.NewReader(body))
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

// adminCommand runs a subcommand of the binary and returns its standard output.
func adminCommand(t *testing.T, bin string, args ...string) string {
	t.Helper()
	cmd := exec.Command(bin, args...)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	out, err := cmd.Output()
	if err != nil {
		t.Fatalf("fps-controller %s: %v\n%s", strings.Join(args, " "), err, stderr.String())
	}
	return strings.TrimSpace(string(out))
}

// TestEndToEndMutualTLS starts the binary in its secure default mode and runs
// the whole path with a Go fake agent: the operator mints a token with the
// token subcommand, the agent enrolls over HTTPS and registers over mutual
// TLS, an agent without a valid certificate cannot register, and the revoke
// subcommand ends the session of the agent and keeps it out until a new token
// enrolls it again. The real Rust agent has no TLS until AGT-08, so its
// end to end test uses the explicit plain mode.
func TestEndToEndMutualTLS(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("SIGTERM and unix sockets are not available on Windows")
	}
	if testing.Short() {
		t.Skip("builds the binary")
	}
	bin := filepath.Join(t.TempDir(), "fps-controller")
	if out, err := exec.Command("go", "build", "-o", bin, ".").CombinedOutput(); err != nil {
		t.Fatalf("go build: %v\n%s", err, out)
	}
	dir := t.TempDir()
	// A short path keeps the socket below the limit of sockaddr_un.
	sockDir, err := os.MkdirTemp("", "fps")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.RemoveAll(sockDir) })
	socket := filepath.Join(sockDir, "a.sock")
	dataDir := filepath.Join(dir, "data")
	cfgPath := writeConfig(t, fmt.Sprintf(`listen: 127.0.0.1:0
grpc_listen: 127.0.0.1:0
enroll_listen: 127.0.0.1:0
admin_socket: %s
log_level: debug
shutdown_timeout: 10s
data_dir: %s
tokens:
  - { name: admin, scope: admin, token: %s }
`, socket, dataDir, e2eAdminToken))
	p := startController(t, bin, cfgPath)
	enrollAddr := waitForRecord(t, p.logs, "enrollment service listening")
	waitForRecord(t, p.logs, "admin socket listening")

	caPEM, err := os.ReadFile(filepath.Join(dataDir, "pki", pki.CACertFile)) //nolint:gosec // G304: the test's own temporary directory
	if err != nil {
		t.Fatal(err)
	}
	if info, err := os.Stat(filepath.Join(dataDir, "pki", pki.CAKeyFile)); err != nil || info.Mode().Perm() != 0o600 {
		t.Fatalf("CA key = %v, %v, want mode 0600", info, err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	caps := fakeagent.DefaultCapabilities()
	caps.EngineApi = api.EngineAPIVersion
	caps.Ports = []*agentpb.PortInfo{{Name: "lo", Driver: "loopback", SpeedMbps: 10000, Mtu: 65536}}
	start := func(id string, dial fakeagent.Dialer) (*fakeagent.Agent, error) {
		return fakeagent.Start(ctx, fakeagent.Options{ID: id, Capabilities: caps, AutoHeartbeat: true, Dial: dial})
	}
	agentState := func() api.AgentState {
		var list []api.Agent
		p.getJSON(t, "/v1/agents", e2eAdminToken, &list)
		for _, a := range list {
			if a.Id == "local" {
				return a.State
			}
		}
		return ""
	}

	// No certificate, and no admission, whatever the agent claims.
	if _, err := start("local", tlsDial(p.grpcAddr, caPEM, nil)); err == nil {
		t.Fatal("an agent without a client certificate registered")
	}
	// A plain client cannot talk to the TLS port either.
	if _, err := start("local", func(context.Context) (*grpc.ClientConn, error) {
		return grpc.NewClient(p.grpcAddr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	}); err == nil {
		t.Fatal("a plain gRPC client registered with the TLS controller")
	}
	// A certificate of another CA, with the right name.
	other, err := pki.Open(pki.Config{})
	if err != nil {
		t.Fatal(err)
	}
	otherToken, _, err := other.NewToken("local", 0)
	if err != nil {
		t.Fatal(err)
	}
	otherKey, otherCSR, err := pki.NewCSR("local")
	if err != nil {
		t.Fatal(err)
	}
	issued, err := other.Enroll(otherToken, otherCSR)
	if err != nil {
		t.Fatal(err)
	}
	foreign, err := pki.KeyPair(issued.CertPEM, otherKey)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := start("local", tlsDial(p.grpcAddr, caPEM, foreign)); err == nil {
		t.Fatal("an agent with a certificate of another CA registered")
	}
	// The enrollment service refuses a made up token.
	if cert, code := enrollHTTPS(t, enrollAddr, caPEM, "0123456789abcdef.not-a-real-secret", "local"); cert != nil || code != http.StatusUnauthorized {
		t.Fatalf("enrollment with a made up token = %v, %d, want 401", cert, code)
	}

	// The operator mints a token, and the agent enrolls and registers.
	token := adminCommand(t, bin, "token", "-agent-id", "local", "-ttl", "10m", "-config", cfgPath)
	cert, code := enrollHTTPS(t, enrollAddr, caPEM, token, "local")
	if cert == nil {
		t.Fatalf("enrollment with a minted token answered %d; logs:\n%s", code, p.logs.String())
	}
	if _, code := enrollHTTPS(t, enrollAddr, caPEM, token, "local"); code != http.StatusUnauthorized {
		t.Errorf("second use of the token answered %d, want 401", code)
	}
	ag, err := start("local", tlsDial(p.grpcAddr, caPEM, cert))
	if err != nil {
		t.Fatalf("registration over mutual TLS: %v\nlogs:\n%s", err, p.logs.String())
	}
	defer ag.Close()
	if got := agentState(); got != api.AgentStateOnline {
		t.Fatalf("agent state = %q, want online", got)
	}
	// An agent may not borrow another identity with its certificate.
	if _, err := start("other-agent", tlsDial(p.grpcAddr, caPEM, cert)); err == nil {
		t.Fatal("a certificate for local registered as other-agent")
	}

	// Revocation ends the open session and keeps the agent out.
	out := adminCommand(t, bin, "revoke", "-agent-id", "local", "-data-dir", dataDir, "-admin-socket", socket)
	if !strings.Contains(out, "revoked 1 certificate") {
		t.Errorf("revoke output = %q, want it to report one revoked certificate", out)
	}
	select {
	case <-ag.Done():
	case <-time.After(10 * time.Second):
		t.Fatalf("the session of the revoked agent stayed open; logs:\n%s", p.logs.String())
	}
	waitAgentState(t, agentState, api.AgentStateOffline)
	if _, err := start("local", tlsDial(p.grpcAddr, caPEM, cert)); err == nil {
		t.Fatal("a revoked agent registered again")
	}

	// A new token enrolls the identity again.
	token = adminCommand(t, bin, "token", "-agent-id", "local", "-admin-socket", socket)
	cert, code = enrollHTTPS(t, enrollAddr, caPEM, token, "local")
	if cert == nil {
		t.Fatalf("second enrollment answered %d", code)
	}
	ag2, err := start("local", tlsDial(p.grpcAddr, caPEM, cert))
	if err != nil {
		t.Fatalf("registration after a new enrollment: %v", err)
	}
	defer ag2.Close()
	waitAgentState(t, agentState, api.AgentStateOnline)

	// The token never reaches the logs.
	if strings.Contains(p.logs.String(), strings.SplitN(token, ".", 2)[1]) {
		t.Error("the enrollment token secret appears in the controller log")
	}
	p.stop(t)
	if _, err := os.Stat(socket); !os.IsNotExist(err) {
		t.Errorf("the admin socket stays after shutdown: %v", err)
	}
}

func waitAgentState(t *testing.T, get func() api.AgentState, want api.AgentState) {
	t.Helper()
	deadline := time.Now().Add(10 * time.Second)
	for get() != want {
		if time.Now().After(deadline) {
			t.Fatalf("agent state = %q, want %q", get(), want)
		}
		time.Sleep(20 * time.Millisecond)
	}
}
