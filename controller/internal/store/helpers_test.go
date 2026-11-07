// SPDX-License-Identifier: GPL-3.0-only

package store

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	"go.yaml.in/yaml/v3"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
)

// contracts is the contracts directory, relative to the package directory.
var contracts = filepath.Join("..", "..", "..", "contracts")

// fixture is one test configuration file and the JSON Pointer that its first
// line names after "# want:", if any.
type fixture struct {
	name string
	data []byte
	want string
}

// examples returns every configuration under contracts/examples/tests/.
func examples(t *testing.T) []fixture {
	t.Helper()
	fs := readFixtures(t, filepath.Join(contracts, "examples", "tests"))
	if len(fs) < 2 {
		t.Fatalf("found %d examples, want at least 2", len(fs))
	}
	return fs
}

func readFixtures(t *testing.T, dir string) []fixture {
	t.Helper()
	paths, err := filepath.Glob(filepath.Join(dir, "*.yaml"))
	if err != nil {
		t.Fatal(err)
	}
	sort.Strings(paths)
	var out []fixture
	for _, p := range paths {
		data, err := os.ReadFile(p) //nolint:gosec // G304: fixed test path
		if err != nil {
			t.Fatal(err)
		}
		f := fixture{name: strings.TrimSuffix(filepath.Base(p), ".yaml"), data: data}
		line, _ := bufio.NewReader(bytes.NewReader(data)).ReadString('\n')
		if w, ok := strings.CutPrefix(strings.TrimSpace(line), "# want:"); ok {
			f.want = strings.TrimSpace(w)
		}
		out = append(out, f)
	}
	if len(out) == 0 {
		t.Fatalf("no fixtures in %s", dir)
	}
	return out
}

// fakeCatalog is the ModuleCatalog fake. It serves a fixed list or an error.
type fakeCatalog struct {
	scenarios []compiler.Scenario
	err       error
}

func (f fakeCatalog) Scenarios(context.Context) ([]compiler.Scenario, error) {
	return f.scenarios, f.err
}

// fakeInventory is the AgentInventory fake.
type fakeInventory struct {
	agents []compiler.Agent
	err    error
}

func (f fakeInventory) AvailableAgents(context.Context) ([]compiler.Agent, error) {
	return f.agents, f.err
}

// exampleCatalog lists the scenarios of the example modules under
// contracts/examples/modules/, each with a bytes per session estimate.
func exampleCatalog(t *testing.T) fakeCatalog {
	t.Helper()
	manifests, err := filepath.Glob(filepath.Join(contracts, "examples", "modules", "*", "manifest.yaml"))
	if err != nil || len(manifests) == 0 {
		t.Fatalf("no example modules: %v", err)
	}
	var out []compiler.Scenario
	for _, m := range manifests {
		data, err := os.ReadFile(m) //nolint:gosec // G304: fixed test path
		if err != nil {
			t.Fatal(err)
		}
		var man struct {
			Name     string `yaml:"name"`
			Version  string `yaml:"version"`
			Settings any    `yaml:"settings"`
		}
		if err := yaml.Unmarshal(data, &man); err != nil {
			t.Fatal(err)
		}
		var schema json.RawMessage
		if man.Settings != nil {
			if schema, err = json.Marshal(man.Settings); err != nil {
				t.Fatal(err)
			}
		}
		files, err := filepath.Glob(filepath.Join(filepath.Dir(m), "scenarios", "*.yaml"))
		if err != nil {
			t.Fatal(err)
		}
		for _, f := range files {
			out = append(out, compiler.Scenario{
				Module:          compiler.ModuleRef{Name: man.Name, Version: man.Version, Digest: "sha256:" + strings.Repeat("0", 64)},
				Name:            strings.TrimSuffix(filepath.Base(f), ".yaml"),
				Source:          "name: " + man.Name,
				BytesPerSession: 4096,
				SettingsSchema:  schema,
			})
		}
	}
	return fakeCatalog{scenarios: out}
}

// exampleInventory holds the agents that the example configurations name.
func exampleInventory() fakeInventory {
	return fakeInventory{agents: []compiler.Agent{
		{ID: "local", Cores: 2, Interfaces: []string{"lo"}},
		{ID: "agent-a", Cores: 4, Interfaces: []string{"eth1", "eth2"}},
		{ID: "spare", Cores: 1},
	}}
}

// hasFinding reports whether fs holds a finding of severity sev at path.
func hasFinding(fs []api.Finding, sev api.FindingSeverity, path string) bool {
	for _, f := range fs {
		if f.Severity == sev && f.Path == path {
			return true
		}
	}
	return false
}

func mustCanonical(t *testing.T, data []byte) []byte {
	t.Helper()
	doc, err := Canonical(data, FormatYAML)
	if err != nil {
		t.Fatal(err)
	}
	return doc
}
