// SPDX-License-Identifier: GPL-3.0-only

package metrics

import (
	"bytes"
	"math"
	"net/http"
	"net/http/httptest"
	"os"
	"regexp"
	"strings"
	"testing"

	golden "github.com/alrawi/FixingPointSystem/tests/golden/go"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
)

func TestCatalogMatchesContract(t *testing.T) {
	want, err := os.ReadFile("../../../contracts/metrics/catalog.yaml")
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(catalogYAML, want) {
		t.Fatal("controller/internal/metrics/catalog.yaml differs from contracts/metrics/catalog.yaml; copy the contract file")
	}
	c, err := DefaultCatalog()
	if err != nil {
		t.Fatal(err)
	}
	if m := c.Lookup("sessions.failed_ratio"); m == nil || m.formula == nil || m.formula.left != "sessions.failed" {
		t.Fatalf("sessions.failed_ratio formula %+v", m)
	}
	if len(c.Metrics()) < 40 {
		t.Fatalf("%d metrics", len(c.Metrics()))
	}
}

func TestParseCatalogRejects(t *testing.T) {
	for name, doc := range map[string]string{
		"yaml":      "metrics: [",
		"no name":   "metrics: [{type: counter}]",
		"kind":      "metrics: [{name: a, type: meter}]",
		"duplicate": "metrics: [{name: a, type: counter}, {name: a, type: counter}]",
		"formula":   "metrics: [{name: a, type: gauge, formula: a b}]",
		"reference": "metrics: [{name: a, type: gauge, formula: b / c}]",
		"histogram": "metrics: [{name: a, type: gauge, formula: b / b}, {name: b, type: histogram}]",
	} {
		if _, err := ParseCatalog([]byte(doc)); err == nil {
			t.Errorf("%s: no error", name)
		}
	}
}

func TestPrometheusNames(t *testing.T) {
	c, err := DefaultCatalog()
	if err != nil {
		t.Fatal(err)
	}
	valid := regexp.MustCompile(`^[a-zA-Z_:][a-zA-Z0-9_:]*$`)
	seen := map[string]string{}
	for _, m := range c.Metrics() {
		n := PrometheusName(m)
		if !valid.MatchString(n) {
			t.Errorf("%s maps to the invalid name %s", m.Name, n)
		}
		for _, suffix := range []string{"", "_sum", "_count"} {
			if other, dup := seen[n+suffix]; dup {
				t.Errorf("%s and %s both map to %s", m.Name, other, n+suffix)
			}
		}
		seen[n] = m.Name
		for _, l := range m.Labels {
			if !valid.MatchString(l) || l == RunLabel || l == "quantile" {
				t.Errorf("%s label %s cannot be a Prometheus label", m.Name, l)
			}
		}
	}
	for name, want := range map[string]string{
		"sessions.attempted":    "fps_sessions_attempted_total",
		"tcp.resets.tx":         "fps_tcp_resets_tx_total",
		"sessions.failed_ratio": "fps_sessions_failed_ratio",
		"tcp.rtt":               "fps_tcp_rtt",
	} {
		if got := PrometheusName(c.Lookup(name)); got != want {
			t.Errorf("%s maps to %s, want %s", name, got, want)
		}
	}
}

func TestPrometheusExposition(t *testing.T) {
	a := newAgg(t, nil)
	a.Add("agent-a", t0, snapshot("run-2/agent-a", 1, []*agentpb.CounterValue{
		counter("sessions.attempted", 40, webGet...),
		counter("sessions.failed", 4, append(webGet, "reason", "expect_timeout")...),
		gauge("sessions.concurrent", 3.5, webGet...),
		counter("port.bytes.tx", 1<<40, "port", `eth"1\`),
		gauge("throughput.bps", math.Inf(1), webGet...),
	}, hist("tcp.rtt", record(10, 20, 30, 4000), "port", "p1")))
	a.Add("agent-b", t0, snapshot("run-1/agent-b", 1, []*agentpb.CounterValue{
		counter("sessions.attempted", 2, webGet...),
	}))
	srv := NewPrometheus(a)
	rec := httptest.NewRecorder()
	srv.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/metrics", nil))
	if rec.Code != http.StatusOK || rec.Header().Get("Content-Type") != PrometheusContentType {
		t.Fatalf("status %d, type %s", rec.Code, rec.Header().Get("Content-Type"))
	}
	golden.Assert(t, "prometheus.txt", rec.Body.Bytes())

	// Every family has HELP and TYPE before its samples, and every sample
	// line parses.
	line := regexp.MustCompile(`^[a-z_]+(\{[a-z_]+="([^"\\\n]|\\.)*"(,[a-z_]+="([^"\\\n]|\\.)*")*\})? (NaN|[+-]Inf|-?[0-9.e+]+)$`)
	declared := map[string]bool{}
	for _, l := range strings.Split(strings.TrimSpace(rec.Body.String()), "\n") {
		if strings.HasPrefix(l, "# TYPE ") {
			declared[strings.Fields(l)[2]] = true
			continue
		}
		if strings.HasPrefix(l, "#") {
			continue
		}
		if !line.MatchString(l) {
			t.Errorf("malformed sample line %q", l)
		}
		name := l[:strings.IndexAny(l, "{ ")]
		base := strings.TrimSuffix(strings.TrimSuffix(name, "_sum"), "_count")
		if !declared[name] && !declared[base] {
			t.Errorf("sample %s before its TYPE", name)
		}
	}

	rec = httptest.NewRecorder()
	srv.ServeHTTP(rec, httptest.NewRequest(http.MethodPost, "/metrics", nil))
	if rec.Code != http.StatusMethodNotAllowed {
		t.Errorf("POST status %d", rec.Code)
	}
	rec = httptest.NewRecorder()
	srv.ServeHTTP(rec, httptest.NewRequest(http.MethodHead, "/metrics", nil))
	if rec.Code != http.StatusOK || rec.Body.Len() != 0 {
		t.Errorf("HEAD status %d with %d bytes", rec.Code, rec.Body.Len())
	}
}
