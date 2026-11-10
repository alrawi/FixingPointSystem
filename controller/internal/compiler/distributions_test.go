// SPDX-License-Identifier: GPL-3.0-only

package compiler

import (
	"bytes"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func fptr(v float64) *float64 { return &v }

const distributionsConfig = "testdata/distributions.yaml"

// TestParseConfigAcceptsDistributions checks that the strict decoder accepts
// every arrival and size field of test configuration v0.3 and keeps its value.
func TestParseConfigAcceptsDistributions(t *testing.T) {
	cfg := loadConfig(t, distributionsConfig)
	if want := (&Arrival{Dist: "exponential", MeanMs: fptr(2)}); !reflect.DeepEqual(cfg.Profile.Arrival, want) {
		t.Errorf("profile.arrival = %+v, want %+v", cfg.Profile.Arrival, want)
	}
	size := cfg.Profile.Size
	if size == nil || size.Both != nil || size.Request == nil || size.Response == nil {
		t.Fatalf("profile.size = %+v, want request and response", size)
	}
	if want := (&SizeDist{Dist: "lognormal", Mu: fptr(5.5), Sigma: fptr(0.8)}); !reflect.DeepEqual(size.Request, want) {
		t.Errorf("profile.size.request = %+v, want %+v", size.Request, want)
	}
	if want := (&SizeDist{Dist: "pareto", Alpha: fptr(1.2), Scale: fptr(512)}); !reflect.DeepEqual(size.Response, want) {
		t.Errorf("profile.size.response = %+v, want %+v", size.Response, want)
	}
	entry := cfg.Profile.Mix[0]
	if want := (&Arrival{Dist: "constant", Ms: fptr(4), ToleranceMs: fptr(0.5)}); !reflect.DeepEqual(entry.Arrival, want) {
		t.Errorf("mix[0].arrival = %+v, want %+v", entry.Arrival, want)
	}
	if entry.Size == nil || entry.Size.Request == nil || entry.Size.Both == nil || entry.Size.Response != nil {
		t.Errorf("mix[0].size = %+v, want request and both", entry.Size)
	}
	if got := cfg.Profile.Mix[1]; got.Arrival != nil || got.Size != nil {
		t.Errorf("mix[1] = %+v, want no declarations", got)
	}
}

// TestParseConfigStillRejectsUnknownDistributionKeys checks that the decoder
// stays strict inside the new objects, so a misspelled parameter is an error.
func TestParseConfigStillRejectsUnknownDistributionKeys(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("testdata", "distributions.yaml"))
	if err != nil {
		t.Fatal(err)
	}
	for _, tt := range []struct{ name, old, repl string }{
		{"arrival parameter", "mean_ms: 2", "mean: 2"},
		{"size parameter", "alpha: 1.2, scale: 512", "alpha: 1.2, scale_ms: 512"},
		{"size direction", "response: { dist: pareto", "reply: { dist: pareto"},
	} {
		t.Run(tt.name, func(t *testing.T) {
			text := string(data)
			if !strings.Contains(text, tt.old) {
				t.Fatalf("test data lacks %q", tt.old)
			}
			_, err := ParseConfig([]byte(strings.Replace(text, tt.old, tt.repl, 1)))
			if !errors.Is(err, ErrInvalidConfig) {
				t.Fatalf("ParseConfig = %v, want ErrInvalidConfig", err)
			}
		})
	}
}

// TestDistributionsLeaveThePlanUnchanged checks that a test with arrival and
// size declarations compiles to the same plans as the test without them,
// because the declarations are targets and add no command.
func TestDistributionsLeaveThePlanUnchanged(t *testing.T) {
	agents := []Agent{{ID: "local", Cores: 1, Interfaces: []string{"lo"}}}
	with := loadConfig(t, distributionsConfig)
	without := with
	without.Profile.Arrival, without.Profile.Size = nil, nil
	without.Profile.Mix = append([]MixConfig(nil), with.Profile.Mix...)
	for i := range without.Profile.Mix {
		without.Profile.Mix[i].Arrival, without.Profile.Mix[i].Size = nil, nil
	}
	build := func(cfg Config) [][]byte {
		plans, err := Compile(Request{RunID: "run-0001", Config: cfg, Agents: agents, Scenarios: exampleCatalog(t)})
		if err != nil {
			t.Fatal(err)
		}
		return render(t, plans)
	}
	a, b := build(with), build(without)
	if len(a) != len(b) {
		t.Fatalf("%d plans with declarations, %d without", len(a), len(b))
	}
	for i := range a {
		if !bytes.Equal(a[i], b[i]) {
			t.Errorf("plan %d differs with declarations", i)
		}
	}
}

// TestDeclarations checks what the compiler carries to the report: the
// profile declarations, and for each mix entry its own arrival and its size
// with each direction overriding the profile's.
func TestDeclarations(t *testing.T) {
	cfg := loadConfig(t, distributionsConfig)
	got := cfg.Declarations()
	if got == nil {
		t.Fatal("Declarations() = nil")
	}
	prof := cfg.Profile
	if got.Arrival != prof.Arrival || got.Size != prof.Size {
		t.Errorf("profile level = %+v %+v, want the profile declarations", got.Arrival, got.Size)
	}
	if len(got.Mix) != 2 {
		t.Fatalf("mix = %+v, want 2 entries", got.Mix)
	}
	dns, web := got.Mix[0], got.Mix[1]
	if dns.Scenario != "dns/resolve" || dns.Arrival == nil || dns.Arrival.Dist != "constant" {
		t.Errorf("dns/resolve entry = %+v, want its own constant arrival", dns)
	}
	// request and both come from the entry, and response from the profile.
	if dns.Size == nil || dns.Size.Request.Dist != "constant" || dns.Size.Both.Dist != "uniform" || dns.Size.Response != prof.Size.Response {
		t.Errorf("dns/resolve size = %+v, want constant request, uniform both, and the profile response", dns.Size)
	}
	// An entry without declarations takes the profile size and no arrival.
	if web.Scenario != "http1/web_get" || web.Arrival != nil || web.Size == nil || web.Size.Request != prof.Size.Request || web.Size.Both != nil {
		t.Errorf("http1/web_get entry = %+v, want the profile size and no arrival", web)
	}
}

func TestDeclarationsAreNilWithoutDeclarations(t *testing.T) {
	cfg := loadConfig(t, filepath.Join("testdata", "module_settings.yaml"))
	if d := cfg.Declarations(); d != nil {
		t.Errorf("Declarations() = %+v, want nil", d)
	}
}
