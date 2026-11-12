// SPDX-License-Identifier: GPL-3.0-only

package wiring

import (
	"context"
	"errors"
	"io/fs"
	"math"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/registry"
)

// enterpriseMixPath is the example test with the basis bytes, a throughput
// objective of 10 Gbit/s, and one scenario of dns and two of http1.
const enterpriseMixPath = "../../../contracts/examples/tests/enterprise_mix_10g.yaml"

// expectedBytesKey matches the expected_bytes key of a manifest with its
// entries, which a test removes to build a module without estimates.
var expectedBytesKey = regexp.MustCompile(`(?m)^expected_bytes:\n(?:[ \t]+.*\n)+`)

// copyModule copies the module tree in src into a new directory with the same
// base name and applies edit to the manifest on the way. It returns the new
// directory.
func copyModule(t *testing.T, src string, edit func(manifest string) string) string {
	t.Helper()
	dst := filepath.Join(t.TempDir(), filepath.Base(src))
	err := filepath.WalkDir(src, func(p string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		rel, err := filepath.Rel(src, p)
		if err != nil {
			return err
		}
		target := filepath.Join(dst, rel)
		if d.IsDir() {
			return os.MkdirAll(target, 0o750)
		}
		data, err := os.ReadFile(p) //nolint:gosec // G304: walks a fixed test directory
		if err != nil {
			return err
		}
		if rel == registry.ManifestFile {
			data = []byte(edit(string(data)))
		}
		return os.WriteFile(target, data, 0o600)
	})
	if err != nil {
		t.Fatalf("copy module %s: %v", src, err)
	}
	return dst
}

// catalogOf installs the module trees in dirs into a new registry and returns
// the scenario catalog of the controller over it.
func catalogOf(t *testing.T, dirs ...string) []compiler.Scenario {
	t.Helper()
	ctx := context.Background()
	reg, err := registry.Open(t.TempDir(), registry.Limits{})
	if err != nil {
		t.Fatal(err)
	}
	for _, d := range dirs {
		if _, err := reg.InstallDir(ctx, d); err != nil {
			t.Fatalf("InstallDir(%s) = %v", d, err)
		}
	}
	got, err := Catalog{Registry: reg}.Scenarios(ctx)
	if err != nil {
		t.Fatalf("Scenarios error = %v", err)
	}
	return got
}

// compileEnterprise compiles the enterprise example for one agent against the
// catalog, after mutate changed the configuration.
func compileEnterprise(t *testing.T, catalog []compiler.Scenario, mutate func(*compiler.Config)) (compiler.Plan, error) {
	t.Helper()
	data, err := os.ReadFile(enterpriseMixPath) //nolint:gosec // G304: fixed test path
	if err != nil {
		t.Fatal(err)
	}
	cfg, err := compiler.ParseConfig(data)
	if err != nil {
		t.Fatal(err)
	}
	if mutate != nil {
		mutate(&cfg)
	}
	plans, err := compiler.Compile(compiler.Request{
		RunID:     "run-0001",
		Config:    cfg,
		Agents:    []compiler.Agent{{ID: "agent-a", Cores: 8, Backend: "dpdk"}},
		Scenarios: catalog,
	})
	if err != nil {
		return compiler.Plan{}, err
	}
	if len(plans) != 1 {
		t.Fatalf("%d plans, want 1", len(plans))
	}
	return plans[0], nil
}

// bitsPerSecond returns the throughput that the worker rates of the plan carry
// when every session moves the bytes of the catalog estimate of its scenario.
func bitsPerSecond(t *testing.T, plan compiler.Plan, estimate map[string]float64) float64 {
	t.Helper()
	scenario := map[string]string{}
	for _, m := range plan.Mix {
		scenario[m.ID] = m.Scenario
	}
	var bps float64
	for _, w := range plan.Workers {
		for _, r := range w.Rates {
			if r.SessionsPerSecond == nil {
				t.Fatalf("rate of %s has no sessions per second", r.MixEntry)
			}
			bps += 8 * *r.SessionsPerSecond * estimate[scenario[r.MixEntry]]
		}
	}
	return bps
}

// estimates maps module/scenario to the bytes per session of the catalog.
func estimates(catalog []compiler.Scenario) map[string]float64 {
	out := map[string]float64{}
	for _, s := range catalog {
		out[s.Module.Name+"/"+s.Name] = s.BytesPerSession
	}
	return out
}

func TestCatalogSetsBytesPerSessionFromManifests(t *testing.T) {
	catalog := catalogOf(t, filepath.Join(examplesDir, "dns"), filepath.Join(examplesDir, "http1"))
	manifests := map[string]registry.Manifest{}
	for _, name := range []string{"dns", "http1"} {
		data, err := os.ReadFile(filepath.Join(examplesDir, name, registry.ManifestFile)) //nolint:gosec // G304: fixed test path
		if err != nil {
			t.Fatal(err)
		}
		m, err := registry.ParseManifest(data)
		if err != nil {
			t.Fatalf("ParseManifest(%s) = %v", name, err)
		}
		manifests[name] = m
	}
	if len(catalog) != 5 {
		t.Fatalf("catalog has %d scenarios, want 5", len(catalog))
	}
	for _, s := range catalog {
		want := manifests[s.Module.Name].ExpectedBytes[s.Name]
		if want < 1 {
			t.Fatalf("%s/%s: the example manifest has no estimate", s.Module.Name, s.Name)
		}
		if s.BytesPerSession != float64(want) {
			t.Errorf("%s/%s: BytesPerSession = %v, want the manifest estimate %d", s.Module.Name, s.Name, s.BytesPerSession, want)
		}
	}
}

func TestCatalogCompilesBytesBasisAndThroughputFromManifests(t *testing.T) {
	catalog := catalogOf(t, filepath.Join(examplesDir, "dns"), filepath.Join(examplesDir, "http1"))
	est := estimates(catalog)
	byteWeight := map[string]float64{"http1/web_browse": 0.6, "http1/web_get": 0.399, "dns/lookup": 0.001}

	t.Run("bytes basis with a throughput objective", func(t *testing.T) {
		plan, err := compileEnterprise(t, catalog, nil)
		if err != nil {
			t.Fatalf("Compile error = %v", err)
		}
		if len(plan.Mix) != 3 {
			t.Fatalf("%d mix entries, want 3", len(plan.Mix))
		}
		// The session shares follow weight over bytes per session, so sessions
		// at these shares carry the byte weights again.
		var wantSum, sum, carried float64
		for ref, w := range byteWeight {
			wantSum += w / est[ref]
		}
		for _, m := range plan.Mix {
			want := byteWeight[m.Scenario] / est[m.Scenario] / wantSum
			if math.Abs(m.Weight-want) > 1e-12 {
				t.Errorf("session share of %s = %v, want %v", m.ID, m.Weight, want)
			}
			sum += m.Weight
			carried += m.Weight * est[m.Scenario]
		}
		if math.Abs(sum-1) > 1e-12 {
			t.Errorf("session shares sum to %v, want 1", sum)
		}
		for _, m := range plan.Mix {
			if got := m.Weight * est[m.Scenario] / carried; math.Abs(got-byteWeight[m.Scenario]) > 1e-12 {
				t.Errorf("byte share of %s = %v, want %v", m.ID, got, byteWeight[m.Scenario])
			}
		}
		if got := bitsPerSecond(t, plan, est); math.Abs(got/10e9-1) > 1e-9 {
			t.Errorf("worker rates carry %v bit/s, want 10e9", got)
		}
	})

	t.Run("sessions basis with a throughput objective", func(t *testing.T) {
		plan, err := compileEnterprise(t, catalog, func(c *compiler.Config) { c.Profile.Basis = "sessions" })
		if err != nil {
			t.Fatalf("Compile error = %v", err)
		}
		// The weights are session shares here, so the mean bytes per session
		// is the weighted mean of the manifest estimates.
		var mean float64
		for _, m := range plan.Mix {
			mean += m.Weight * est[m.Scenario]
		}
		wantSessions := 10e9 / (8 * mean)
		var sessions float64
		for _, w := range plan.Workers {
			for _, r := range w.Rates {
				sessions += *r.SessionsPerSecond
			}
		}
		if math.Abs(sessions/wantSessions-1) > 1e-9 {
			t.Errorf("worker rates sum to %v sessions/s, want %v", sessions, wantSessions)
		}
		if got := bitsPerSecond(t, plan, est); math.Abs(got/10e9-1) > 1e-9 {
			t.Errorf("worker rates carry %v bit/s, want 10e9", got)
		}
	})

	t.Run("bytes basis with a session rate objective", func(t *testing.T) {
		_, err := compileEnterprise(t, catalog, func(c *compiler.Config) {
			c.Load.Objective = compiler.Objective{Type: "sessions_per_second", Value: 1000}
		})
		if err != nil {
			t.Fatalf("Compile error = %v", err)
		}
	})
}

func TestCatalogWithoutEstimateFailsWithErrNoByteEstimate(t *testing.T) {
	bare := copyModule(t, filepath.Join(examplesDir, "dns"), func(manifest string) string {
		out := expectedBytesKey.ReplaceAllString(manifest, "")
		if out == manifest {
			t.Fatal("the dns example manifest has no expected_bytes key to remove")
		}
		return out
	})
	catalog := catalogOf(t, bare, filepath.Join(examplesDir, "http1"))
	for _, s := range catalog {
		if s.Module.Name == "dns" && s.BytesPerSession != 0 {
			t.Errorf("dns/%s: BytesPerSession = %v, want 0 without an estimate", s.Name, s.BytesPerSession)
		}
		if s.Module.Name == "http1" && !(s.BytesPerSession > 0) {
			t.Errorf("http1/%s: BytesPerSession = %v, want the manifest estimate", s.Name, s.BytesPerSession)
		}
	}

	for name, mutate := range map[string]func(*compiler.Config){
		"bytes basis with a throughput objective": nil,
		"bytes basis with a session rate objective": func(c *compiler.Config) {
			c.Load.Objective = compiler.Objective{Type: "sessions_per_second", Value: 1000}
		},
		"sessions basis with a throughput objective": func(c *compiler.Config) { c.Profile.Basis = "sessions" },
	} {
		t.Run(name, func(t *testing.T) {
			_, err := compileEnterprise(t, catalog, mutate)
			if !errors.Is(err, compiler.ErrNoByteEstimate) || !strings.Contains(err.Error(), "dns/lookup") {
				t.Fatalf("error = %v, want ErrNoByteEstimate naming dns/lookup", err)
			}
		})
	}

	// The sessions basis with a session rate objective needs no estimate.
	_, err := compileEnterprise(t, catalog, func(c *compiler.Config) {
		c.Profile.Basis = "sessions"
		c.Load.Objective = compiler.Objective{Type: "sessions_per_second", Value: 1000}
	})
	if err != nil {
		t.Fatalf("sessions basis with a session rate objective: %v", err)
	}
}
