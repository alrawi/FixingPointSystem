// SPDX-License-Identifier: GPL-3.0-only

package app

import (
	"bytes"
	"encoding/json"
	"errors"
	"io/fs"
	"log/slog"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"
)

func TestParseConfigEmptyDocumentYieldsDefaults(t *testing.T) {
	for _, doc := range []string{"", "# only a comment\n"} {
		got, err := ParseConfig([]byte(doc))
		if err != nil {
			t.Fatalf("ParseConfig(%q) error = %v", doc, err)
		}
		if !reflect.DeepEqual(got, DefaultConfig()) {
			t.Errorf("ParseConfig(%q) = %+v, want defaults %+v", doc, got, DefaultConfig())
		}
	}
}

func TestParseConfigOverridesOnlyGivenKeys(t *testing.T) {
	got, err := ParseConfig([]byte("listen: 127.0.0.1:9000\nshutdown_timeout: 2s\n"))
	if err != nil {
		t.Fatalf("ParseConfig error = %v", err)
	}
	want := DefaultConfig()
	want.Listen = "127.0.0.1:9000"
	want.ShutdownTimeout = Duration(2 * time.Second)
	if !reflect.DeepEqual(got, want) {
		t.Errorf("ParseConfig = %+v, want %+v", got, want)
	}
}

func TestParseConfigRejectsBadInput(t *testing.T) {
	tests := []struct {
		name    string
		doc     string
		wantErr string
		invalid bool
	}{
		{"unknown key", "listn: 127.0.0.1:1\n", "field listn not found", false},
		{"duration without unit", "shutdown_timeout: 15\n", "missing unit", false},
		{"duration as mapping", "shutdown_timeout: {s: 1}\n", "duration must be a string", false},
		{"listen without port", "listen: localhost\n", "listen", true},
		{"unknown log level", "log_level: verbose\n", "log_level", true},
		{"zero shutdown timeout", "shutdown_timeout: 0s\n", "shutdown_timeout", true},
		{"negative read header timeout", "read_header_timeout: -1s\n", "read_header_timeout", true},
		{"list instead of mapping", "- a\n", "cannot unmarshal", false},
		{"grpc listen without port", "grpc_listen: localhost\n", "grpc_listen", true},
		{"token without name", "tokens: [{scope: read, token: " + longToken + "}]\n", "tokens[0]: name is empty", true},
		{"token with unknown scope", "tokens: [{name: a, scope: write, token: " + longToken + "}]\n", `scope "write"`, true},
		{"token without secret", "tokens: [{name: a, scope: read}]\n", "neither token nor token_file", true},
		{"token with two secrets", "tokens: [{name: a, scope: read, token: " + longToken + ", token_file: x}]\n", "both set", true},
		{"short token", "tokens: [{name: a, scope: read, token: short}]\n", "shorter than 16", true},
		{"repeated token name", "tokens: [{name: a, scope: read, token: " + longToken + "}, {name: a, scope: run, token_file: x}]\n", "tokens[1]: name \"a\" repeats", true},
		{"unknown token key", "tokens: [{name: a, scope: read, secret: x}]\n", "field secret not found", false},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			_, err := ParseConfig([]byte(tc.doc))
			if err == nil {
				t.Fatalf("ParseConfig(%q) error = nil, want %q", tc.doc, tc.wantErr)
			}
			if !strings.Contains(err.Error(), tc.wantErr) {
				t.Errorf("error = %q, want it to contain %q", err, tc.wantErr)
			}
			if got := errors.Is(err, ErrInvalidConfig); got != tc.invalid {
				t.Errorf("errors.Is(err, ErrInvalidConfig) = %v, want %v", got, tc.invalid)
			}
		})
	}
}

func TestLoadConfigFileReadsExample(t *testing.T) {
	got, err := LoadConfigFile(filepath.Join("testdata", "controller.yaml"))
	if err != nil {
		t.Fatalf("LoadConfigFile error = %v", err)
	}
	want := Config{
		Listen:            "127.0.0.1:8443",
		LogLevel:          "debug",
		ShutdownTimeout:   Duration(5 * time.Second),
		ReadHeaderTimeout: Duration(3 * time.Second),
		DataDir:           "/var/lib/fps",
		StorePath:         "/var/lib/fps/store/configs.db",
		GRPCListen:        "127.0.0.1:8444",
		AgentAutoEnroll:   true,
		Tokens: []Token{
			{Name: "ci", Scope: "run", Token: "example-run-token-0123456789"},
			{Name: "ops", Scope: "admin", TokenFile: "/etc/fps/admin.token"},
		},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("LoadConfigFile = %+v, want %+v", got, want)
	}
}

func TestLoadConfigFileMissingWrapsNotExist(t *testing.T) {
	_, err := LoadConfigFile(filepath.Join(t.TempDir(), "absent.yaml"))
	if !errors.Is(err, fs.ErrNotExist) {
		t.Fatalf("error = %v, want one that wraps fs.ErrNotExist", err)
	}
}

func TestDurationRoundTrip(t *testing.T) {
	v, err := Duration(1500 * time.Millisecond).MarshalYAML()
	if err != nil || v != "1.5s" {
		t.Fatalf("MarshalYAML = %v, %v, want 1.5s", v, err)
	}
}

func TestParseLogLevelIgnoresCase(t *testing.T) {
	tests := map[string]slog.Level{"DEBUG": slog.LevelDebug, "Info": slog.LevelInfo, "warn": slog.LevelWarn, "error": slog.LevelError}
	for in, want := range tests {
		got, err := ParseLogLevel(in)
		if err != nil || got != want {
			t.Errorf("ParseLogLevel(%q) = %v, %v, want %v", in, got, err, want)
		}
	}
}

func TestNewLoggerWritesJSONAtLevel(t *testing.T) {
	var buf bytes.Buffer
	logger, err := NewLogger(&buf, "warn")
	if err != nil {
		t.Fatalf("NewLogger error = %v", err)
	}
	logger.Info("dropped")
	logger.Warn("kept", "run_id", "r1")

	lines := strings.Split(strings.TrimSpace(buf.String()), "\n")
	if len(lines) != 1 {
		t.Fatalf("got %d lines %q, want 1", len(lines), lines)
	}
	var rec map[string]any
	if err := json.Unmarshal([]byte(lines[0]), &rec); err != nil {
		t.Fatalf("log line is not JSON: %v", err)
	}
	if rec["msg"] != "kept" || rec["level"] != "WARN" || rec["run_id"] != "r1" {
		t.Errorf("record = %v, want msg kept, level WARN, run_id r1", rec)
	}
}

func TestNewLoggerRejectsUnknownLevel(t *testing.T) {
	if _, err := NewLogger(&bytes.Buffer{}, "loud"); !errors.Is(err, ErrInvalidConfig) {
		t.Fatalf("error = %v, want ErrInvalidConfig", err)
	}
}

// longToken is a token that meets MinTokenLength.
const longToken = "token-0123456789abcdef"

func TestResolvedPaths(t *testing.T) {
	tests := []struct {
		name      string
		cfg       Config
		wantStore string
		wantReg   string
	}{
		{"no data dir", Config{}, MemoryStorePath, ""},
		{"data dir", Config{DataDir: "/d"}, filepath.Join("/d", StoreFile), filepath.Join("/d", ModulesDir)},
		{"store path over data dir", Config{DataDir: "/d", StorePath: "/s.db"}, "/s.db", filepath.Join("/d", ModulesDir)},
		{"store path alone", Config{StorePath: "/s.db"}, "/s.db", ""},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			if got := tc.cfg.ResolvedStorePath(); got != tc.wantStore {
				t.Errorf("ResolvedStorePath = %q, want %q", got, tc.wantStore)
			}
			if got := tc.cfg.RegistryDir(); got != tc.wantReg {
				t.Errorf("RegistryDir = %q, want %q", got, tc.wantReg)
			}
		})
	}
}

func TestLoadTokensReadsFiles(t *testing.T) {
	dir := t.TempDir()
	file := filepath.Join(dir, "admin.token")
	if err := os.WriteFile(file, []byte("  admin-secret-0123456789\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	short := filepath.Join(dir, "short.token")
	if err := os.WriteFile(short, []byte("abc\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	cfg := Config{Tokens: []Token{
		{Name: "ci", Scope: "run", Token: longToken},
		{Name: "ops", Scope: "admin", TokenFile: file},
	}}
	got, err := cfg.LoadTokens()
	if err != nil {
		t.Fatalf("LoadTokens error = %v", err)
	}
	want := map[string]string{longToken: "run", "admin-secret-0123456789": "admin"}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("LoadTokens = %v, want %v", got, want)
	}

	for name, tok := range map[string]Token{
		"missing file": {Name: "x", Scope: "read", TokenFile: filepath.Join(dir, "absent")},
		"short file":   {Name: "x", Scope: "read", TokenFile: short},
		"same secret":  {Name: "x", Scope: "read", Token: "admin-secret-0123456789"},
	} {
		t.Run(name, func(t *testing.T) {
			c := Config{Tokens: []Token{{Name: "ops", Scope: "admin", TokenFile: file}, tok}}
			if _, err := c.LoadTokens(); err == nil {
				t.Fatal("LoadTokens error = nil, want an error")
			}
		})
	}
}
