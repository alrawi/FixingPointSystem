// SPDX-License-Identifier: GPL-3.0-only

package wiring

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/registry"
)

// testsDir holds the example test configurations, relative to this package
// directory.
const testsDir = "../../../contracts/examples/tests"

// settingsExamples names the examples that use pinned and spread port
// locations, module settings, and criterion filters, which CON-11 moved out of
// tests-pending.
var settingsExamples = []string{"dns_zone", "filtered_criteria", "port_locations"}

func TestCatalogReportsSettingsSchema(t *testing.T) {
	ctx := context.Background()
	reg, err := registry.Open(t.TempDir(), registry.Limits{})
	if err != nil {
		t.Fatal(err)
	}
	installExamples(t, reg)
	got, err := Catalog{Registry: reg}.Scenarios(ctx)
	if err != nil {
		t.Fatalf("Scenarios error = %v", err)
	}
	var dns, http1 int
	for _, s := range got {
		switch s.Module.Name {
		case "dns":
			dns++
			var schema struct {
				Type       string                     `json:"type"`
				Properties map[string]json.RawMessage `json:"properties"`
			}
			if err := json.Unmarshal(s.SettingsSchema, &schema); err != nil {
				t.Fatalf("dns/%s: settings schema %q: %v", s.Name, s.SettingsSchema, err)
			}
			if _, ok := schema.Properties["zone_file"]; !ok || schema.Type != "object" {
				t.Errorf("dns/%s: settings schema %s, want an object schema with zone_file", s.Name, s.SettingsSchema)
			}
		case "http1":
			http1++
			if len(s.SettingsSchema) != 0 {
				t.Errorf("http1/%s: settings schema %s, want none because the manifest has no settings", s.Name, s.SettingsSchema)
			}
		}
	}
	if dns == 0 || http1 == 0 {
		t.Errorf("catalog has %d dns and %d http1 scenarios, want both modules", dns, http1)
	}
}

// startAgent starts a fake agent with the given interfaces.
func startAgent(ctx context.Context, t *testing.T, addr, id string, ifaces ...string) {
	t.Helper()
	caps := fakeagent.DefaultCapabilities()
	caps.Ports = nil
	for i, name := range ifaces {
		caps.Ports = append(caps.Ports, &agentpb.PortInfo{Name: name, Driver: "veth", SpeedMbps: 10000, Mac: fmt.Sprintf("02:00:00:00:00:%02x", i+1), Mtu: 1500})
	}
	ag, err := fakeagent.Start(ctx, fakeagent.Options{ID: id, Capabilities: caps, Dial: func(context.Context) (*grpc.ClientConn, error) {
		return grpc.NewClient(addr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	}})
	if err != nil {
		t.Fatalf("fake agent %s start = %v", id, err)
	}
	t.Cleanup(ag.Close)
}

// TestSettingsExamplesValidateThroughTheAPI stores each of settingsExamples
// through the REST API and validates it against the installed example modules and
// two registered agents, which runs the compiler dry run.
func TestSettingsExamplesValidateThroughTheAPI(t *testing.T) {
	cfg := testConfig(t, t.TempDir())
	cfg.AgentAutoEnroll = true
	c := newController(t, cfg)
	h := newHandler(t, c)
	installExamples(t, c.Registry)

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	ln, err := c.ListenGRPC(ctx)
	if err != nil {
		t.Fatal(err)
	}
	done := make(chan error, 1)
	go func() { done <- c.Run(ctx, ln) }()
	t.Cleanup(func() {
		cancel()
		<-done
	})
	startAgent(ctx, t, ln.Addr().String(), "agent-a", "eth1", "eth2")
	startAgent(ctx, t, ln.Addr().String(), "local", "lo")

	for _, name := range settingsExamples {
		p := filepath.Join(testsDir, name+".yaml")
		body, err := os.ReadFile(p) //nolint:gosec // G304: fixed test directory
		if err != nil {
			t.Fatal(err)
		}
		// dns_zone names the first party dns/lookup_a since CON-15; the
		// example dns module installed here calls that scenario dns/resolve.
		body = bytes.ReplaceAll(body, []byte("dns/lookup_a"), []byte("dns/resolve"))
		id := createConfigYAML(t, h, name, string(body))
		rec := do(t, h, http.MethodPost, "/v1/configs/"+id+":validate", adminToken, "")
		var rep api.ValidationReport
		if rec.Code != http.StatusOK || json.Unmarshal(rec.Body.Bytes(), &rep) != nil {
			t.Fatalf("%s: validate status %d body %s", name, rec.Code, rec.Body)
		}
		if !rep.Valid {
			t.Errorf("%s: report %s, want valid", name, rec.Body)
		}
		for _, f := range rep.Findings {
			if f.Severity == api.FindingSeverityError {
				t.Errorf("%s: error finding %+v", name, f)
			}
		}
	}
}

// createConfigYAML posts a YAML configuration and returns its id.
func createConfigYAML(t *testing.T, h http.Handler, name, body string) string {
	t.Helper()
	req, err := http.NewRequest(http.MethodPost, "/v1/configs", strings.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", "Bearer "+adminToken)
	req.Header.Set("Content-Type", "application/yaml")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	var ref api.ConfigRef
	if rec.Code != http.StatusCreated || json.Unmarshal(rec.Body.Bytes(), &ref) != nil || ref.Id == "" {
		t.Fatalf("%s: createConfig status %d body %s", name, rec.Code, rec.Body)
	}
	return ref.Id
}
