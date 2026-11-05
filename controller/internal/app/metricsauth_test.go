// SPDX-License-Identifier: GPL-3.0-only

package app

import (
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

const testMetricsToken = "scrape-token-0123456789"

func metricsBody() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write([]byte("fps_up 1\n"))
	})
}

func TestRequireBearer(t *testing.T) {
	tests := []struct {
		name       string
		token      string
		header     string
		wantStatus int
	}{
		{"open endpoint without header", "", "", http.StatusOK},
		{"open endpoint ignores a header", "", "Bearer anything", http.StatusOK},
		{"required token accepted", testMetricsToken, "Bearer " + testMetricsToken, http.StatusOK},
		{"scheme is case insensitive", testMetricsToken, "bearer " + testMetricsToken, http.StatusOK},
		{"required token missing", testMetricsToken, "", http.StatusUnauthorized},
		{"wrong token", testMetricsToken, "Bearer wrong-token-0123456789", http.StatusUnauthorized},
		{"prefix of the token", testMetricsToken, "Bearer " + testMetricsToken[:len(testMetricsToken)-1], http.StatusUnauthorized},
		{"token with a suffix", testMetricsToken, "Bearer " + testMetricsToken + "x", http.StatusUnauthorized},
		{"other scheme", testMetricsToken, "Basic " + testMetricsToken, http.StatusUnauthorized},
		{"scheme without credentials", testMetricsToken, "Bearer ", http.StatusUnauthorized},
		{"bare token", testMetricsToken, testMetricsToken, http.StatusUnauthorized},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			h := RequireBearer(tc.token, metricsBody())
			req := httptest.NewRequest(http.MethodGet, "/metrics", nil)
			if tc.header != "" {
				req.Header.Set("Authorization", tc.header)
			}
			rec := httptest.NewRecorder()
			h.ServeHTTP(rec, req)

			if rec.Code != tc.wantStatus {
				t.Fatalf("status = %d, want %d", rec.Code, tc.wantStatus)
			}
			switch tc.wantStatus {
			case http.StatusOK:
				if rec.Body.String() != "fps_up 1\n" {
					t.Errorf("body = %q, want the metrics", rec.Body.String())
				}
			default:
				if got := rec.Header().Get("WWW-Authenticate"); !strings.HasPrefix(got, "Bearer") {
					t.Errorf("WWW-Authenticate = %q, want a Bearer challenge", got)
				}
				if strings.Contains(rec.Body.String(), "fps_up") {
					t.Errorf("a rejected request received the metrics: %q", rec.Body.String())
				}
			}
		})
	}
}

func TestLoadMetricsToken(t *testing.T) {
	dir := t.TempDir()
	write := func(name, content string) string {
		t.Helper()
		p := filepath.Join(dir, name)
		if err := os.WriteFile(p, []byte(content), 0o600); err != nil {
			t.Fatal(err)
		}
		return p
	}

	t.Run("unset leaves the endpoint open", func(t *testing.T) {
		got, err := DefaultConfig().LoadMetricsToken()
		if err != nil || got != "" {
			t.Fatalf("LoadMetricsToken = %q, %v, want empty and no error", got, err)
		}
	})
	t.Run("trims white space", func(t *testing.T) {
		cfg := Config{MetricsTokenFile: write("ok", "  "+testMetricsToken+"\n")}
		got, err := cfg.LoadMetricsToken()
		if err != nil || got != testMetricsToken {
			t.Fatalf("LoadMetricsToken = %q, %v, want %q", got, err, testMetricsToken)
		}
	})
	t.Run("short token", func(t *testing.T) {
		cfg := Config{MetricsTokenFile: write("short", "tiny\n")}
		if _, err := cfg.LoadMetricsToken(); !errors.Is(err, ErrInvalidConfig) {
			t.Fatalf("error = %v, want ErrInvalidConfig", err)
		}
	})
	t.Run("missing file", func(t *testing.T) {
		cfg := Config{MetricsTokenFile: filepath.Join(dir, "absent")}
		if _, err := cfg.LoadMetricsToken(); !errors.Is(err, ErrInvalidConfig) {
			t.Fatalf("error = %v, want ErrInvalidConfig", err)
		}
	})
}

func TestParseConfigReadsMetricsTokenFile(t *testing.T) {
	cfg, err := ParseConfig([]byte("metrics_token_file: /etc/fps/metrics.token\n"))
	if err != nil {
		t.Fatal(err)
	}
	if cfg.MetricsTokenFile != "/etc/fps/metrics.token" {
		t.Errorf("MetricsTokenFile = %q", cfg.MetricsTokenFile)
	}
}
