// SPDX-License-Identifier: GPL-3.0-only

package compiler

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"
	"io/fs"
	"math"
	"net/netip"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	"go.yaml.in/yaml/v3"

	golden "github.com/alrawi/FixingPointSystem/tests/golden/go"
)

// contracts is the contracts directory relative to this package.
var contracts = filepath.Join("..", "..", "..", "contracts")

// bytesPerSession holds test estimates for the example scenarios. Module
// metadata supplies real estimates once CTL-12 lands.
var bytesPerSession = map[string]float64{
	"http1/web_get":    20000,
	"http1/web_browse": 250000,
	"http1/web_page":   60000,
	"dns/lookup":       150,
	"dns/resolve":      150,
}

// exampleCatalog reads the example modules of the contracts into a catalog,
// with module digests computed by the rule in contracts/README.md.
func exampleCatalog(t *testing.T) []Scenario {
	t.Helper()
	root := filepath.Join(contracts, "examples", "modules")
	dirs, err := os.ReadDir(root)
	if err != nil {
		t.Fatal(err)
	}
	var out []Scenario
	for _, d := range dirs {
		if !d.IsDir() {
			continue
		}
		dir := filepath.Join(root, d.Name())
		var manifest struct {
			Name     string         `yaml:"name"`
			Version  string         `yaml:"version"`
			Settings map[string]any `yaml:"settings"`
		}
		data, err := os.ReadFile(filepath.Join(dir, "manifest.yaml")) //nolint:gosec // G304: fixed test path
		if err != nil {
			t.Fatal(err)
		}
		if err := yaml.Unmarshal(data, &manifest); err != nil {
			t.Fatal(err)
		}
		mod := ModuleRef{Name: manifest.Name, Version: manifest.Version, Digest: treeDigest(t, dir)}
		var schema json.RawMessage
		if manifest.Settings != nil {
			if schema, err = json.Marshal(manifest.Settings); err != nil {
				t.Fatal(err)
			}
		}
		files, err := filepath.Glob(filepath.Join(dir, "scenarios", "*.yaml"))
		if err != nil {
			t.Fatal(err)
		}
		for _, f := range files {
			src, err := os.ReadFile(f) //nolint:gosec // G304: fixed test path
			if err != nil {
				t.Fatal(err)
			}
			name := strings.TrimSuffix(filepath.Base(f), ".yaml")
			out = append(out, Scenario{
				Module: mod, Name: name, Source: string(src),
				BytesPerSession: bytesPerSession[mod.Name+"/"+name], SettingsSchema: schema,
			})
		}
	}
	return out
}

func treeDigest(t *testing.T, dir string) string {
	t.Helper()
	var lines []string
	err := filepath.WalkDir(dir, func(path string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return err
		}
		data, err := os.ReadFile(path) //nolint:gosec // G304: walks a fixed test directory
		if err != nil {
			return err
		}
		rel, err := filepath.Rel(dir, path)
		if err != nil {
			return err
		}
		sum := sha256.Sum256(data)
		lines = append(lines, hex.EncodeToString(sum[:])+"  "+filepath.ToSlash(rel)+"\n")
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	sort.Slice(lines, func(i, j int) bool {
		return strings.SplitN(lines[i], "  ", 2)[1] < strings.SplitN(lines[j], "  ", 2)[1]
	})
	sum := sha256.Sum256([]byte(strings.Join(lines, "")))
	return "sha256:" + hex.EncodeToString(sum[:])
}

func loadConfig(t *testing.T, path string) Config {
	t.Helper()
	data, err := os.ReadFile(path) //nolint:gosec // G304: fixed test path
	if err != nil {
		t.Fatal(err)
	}
	cfg, err := ParseConfig(data)
	if err != nil {
		t.Fatal(err)
	}
	return cfg
}

type goldenCase struct {
	name   string
	config string
	agents []Agent
}

func goldenCases() []goldenCase {
	return []goldenCase{
		{
			name:   "one_agent",
			config: filepath.Join(contracts, "examples", "tests", "http_smoke.yaml"),
			agents: []Agent{{ID: "local", Cores: 2, Interfaces: []string{"lo"}}},
		},
		{
			name:   "two_agents",
			config: filepath.Join("testdata", "two_agents.yaml"),
			agents: []Agent{{ID: "agent-b", Cores: 2, Backend: "af_xdp"}, {ID: "agent-a", Cores: 4}},
		},
		{
			name:   "uneven_pool",
			config: filepath.Join("testdata", "uneven_pool.yaml"),
			agents: []Agent{{ID: "agent-a", Cores: 3}, {ID: "agent-b", Cores: 2}},
		},
		{
			name:   "enterprise_mix_10g",
			config: filepath.Join(contracts, "examples", "tests", "enterprise_mix_10g.yaml"),
			agents: []Agent{{ID: "agent-a", Cores: 8, Backend: "dpdk"}},
		},
		{
			name:   "pinned_port",
			config: filepath.Join("testdata", "pinned_port.yaml"),
			agents: []Agent{{ID: "agent-a", Cores: 2}, {ID: "agent-b", Cores: 2}},
		},
		{
			name:   "small_pool",
			config: filepath.Join("testdata", "small_pool.yaml"),
			agents: []Agent{{ID: "agent-a", Cores: 2}, {ID: "agent-b", Cores: 2}},
		},
		{
			name:   "module_settings",
			config: filepath.Join("testdata", "module_settings.yaml"),
			agents: []Agent{{ID: "local", Cores: 1, Interfaces: []string{"lo"}}},
		},
	}
}

func compileCase(t *testing.T, c goldenCase, agents []Agent) []Plan {
	t.Helper()
	plans, err := Compile(Request{
		RunID:     "run-0001",
		Config:    loadConfig(t, c.config),
		Agents:    agents,
		Scenarios: exampleCatalog(t),
	})
	if err != nil {
		t.Fatal(err)
	}
	return plans
}

func render(t *testing.T, plans []Plan) [][]byte {
	t.Helper()
	out := make([][]byte, len(plans))
	for i, p := range plans {
		b, err := p.MarshalCanonical()
		if err != nil {
			t.Fatal(err)
		}
		out[i] = b
	}
	return out
}

// TestGoldenPlans compares the plans of every case with testdata/<case>/<agent>.json.
func TestGoldenPlans(t *testing.T) {
	for _, c := range goldenCases() {
		t.Run(c.name, func(t *testing.T) {
			plans := compileCase(t, c, c.agents)
			if len(plans) != len(c.agents) {
				t.Fatalf("got %d plans for %d agents", len(plans), len(c.agents))
			}
			for i, b := range render(t, plans) {
				golden.Assert(t, filepath.Join(c.name, plans[i].AgentID+".json"), b)
			}
			checkInvariants(t, plans)
		})
	}
}

// TestCompileIsDeterministic compiles every case twice, the second time with
// the inventory and the catalog in reverse order, and expects identical bytes.
func TestCompileIsDeterministic(t *testing.T) {
	for _, c := range goldenCases() {
		t.Run(c.name, func(t *testing.T) {
			first := render(t, compileCase(t, c, c.agents))
			reversed := append([]Agent(nil), c.agents...)
			for i, j := 0, len(reversed)-1; i < j; i, j = i+1, j-1 {
				reversed[i], reversed[j] = reversed[j], reversed[i]
			}
			cat := exampleCatalog(t)
			for i, j := 0, len(cat)-1; i < j; i, j = i+1, j-1 {
				cat[i], cat[j] = cat[j], cat[i]
			}
			plans, err := Compile(Request{RunID: "run-0001", Config: loadConfig(t, c.config), Agents: reversed, Scenarios: cat})
			if err != nil {
				t.Fatal(err)
			}
			second := render(t, plans)
			for i := range first {
				if !bytes.Equal(first[i], second[i]) {
					t.Fatalf("plan %d differs between compilations:\n%s", i, golden.Diff("first", first[i], second[i]))
				}
			}
		})
	}
}

// checkInvariants confirms that the agents together cover every pool exactly
// once and carry the whole objective.
func checkInvariants(t *testing.T, plans []Plan) {
	t.Helper()
	covered := map[string]uint32{}
	var objective float64
	for _, p := range plans {
		objective += p.Load.ObjectiveValue
		for _, s := range p.HostPools {
			if s.Offset != covered[s.Name] {
				t.Errorf("%s: pool %s slice starts at %d, want %d", p.AgentID, s.Name, s.Offset, covered[s.Name])
			}
			next := s.Offset
			for _, w := range p.Workers {
				for _, r := range w.Hosts {
					if r.Pool == s.Name {
						if r.Offset != next {
							t.Errorf("%s worker %d: pool %s range starts at %d, want %d", p.AgentID, w.WorkerID, s.Name, r.Offset, next)
						}
						next += r.Count
					}
				}
			}
			if next != s.Offset+s.Count {
				t.Errorf("%s: pool %s ranges end at %d, want %d", p.AgentID, s.Name, next, s.Offset+s.Count)
			}
			covered[s.Name] = next
		}
		var sum float64
		for _, w := range p.Workers {
			for _, r := range w.Rates {
				if r.ConcurrentSessions != nil {
					sum += float64(*r.ConcurrentSessions)
				}
			}
		}
		if p.Load.Objective == "LOAD_OBJECTIVE_CONCURRENT_SESSIONS" && sum != p.Load.ObjectiveValue {
			t.Errorf("%s: worker sessions sum to %v, objective share is %v", p.AgentID, sum, p.Load.ObjectiveValue)
		}
	}
	for _, s := range plans[0].HostPools {
		if covered[s.Name] != s.PoolCount {
			t.Errorf("pool %s: agents cover %d of %d hosts", s.Name, covered[s.Name], s.PoolCount)
		}
	}
	if !(objective > 0) {
		t.Errorf("objective shares sum to %v", objective)
	}
}

func TestObjectiveSharesFollowCores(t *testing.T) {
	c := goldenCases()[1]
	plans := compileCase(t, c, c.agents)
	if got := []float64{plans[0].Load.ObjectiveValue, plans[1].Load.ObjectiveValue}; got[0] != 800 || got[1] != 400 {
		t.Fatalf("objective shares %v, want [800 400]", got)
	}
	for _, p := range plans {
		for _, w := range p.Workers {
			if got := *w.Rates[0].SessionsPerSecond + *w.Rates[1].SessionsPerSecond; got != 200 {
				t.Errorf("%s worker %d: %v sessions per second, want 200", p.AgentID, w.WorkerID, got)
			}
		}
	}
}

// TestPinnedPortRunsOnOneAgent checks that a port named <agent>/<interface>
// and its pools exist on that agent only, and that the flow of the pinned pool
// keeps its whole rate on the agent that runs it.
func TestPinnedPortRunsOnOneAgent(t *testing.T) {
	c := goldenCases()[4]
	plans := compileCase(t, c, c.agents)
	a, b := plans[0], plans[1]
	has := func(p Plan, port string) bool {
		for _, pb := range p.Ports {
			if pb.Name == port {
				return true
			}
		}
		return false
	}
	if !has(a, "lab0") || has(b, "lab0") || !has(a, "users0") || !has(b, "users0") || !has(b, "web0") {
		t.Fatalf("ports of agent-a %+v and agent-b %+v do not follow the locations", a.Ports, b.Ports)
	}
	for _, s := range b.HostPools {
		if s.Name == "lab" {
			t.Fatalf("agent-b emulates hosts of the pinned pool: %+v", s)
		}
	}
	var pinned, spread float64
	for _, p := range plans {
		for _, w := range p.Workers {
			for _, r := range w.Rates {
				switch r.MixEntry {
				case "web_get":
					if p.AgentID != "agent-a" {
						t.Errorf("%s worker %d has a rate for the flow of the pinned pool", p.AgentID, w.WorkerID)
					}
					pinned += *r.SessionsPerSecond
				case "web_browse":
					spread += *r.SessionsPerSecond
				}
			}
		}
	}
	if pinned != 250 || spread != 750 {
		t.Fatalf("rates sum to %v and %v, want 250 and 750", pinned, spread)
	}
	if got := a.Load.ObjectiveValue + b.Load.ObjectiveValue; got != 1000 {
		t.Fatalf("objective shares sum to %v, want 1000", got)
	}
}

// TestSmallPoolLeavesWorkersWithoutRates checks that a pool with fewer hosts
// than workers leaves the last worker without hosts and without rates while
// the objective stays whole.
func TestSmallPoolLeavesWorkersWithoutRates(t *testing.T) {
	c := goldenCases()[5]
	plans := compileCase(t, c, c.agents)
	var total uint32
	for _, p := range plans {
		for _, w := range p.Workers {
			hosts := uint32(0)
			for _, h := range w.Hosts {
				if h.Pool == "clients" {
					hosts += h.Count
				}
			}
			if (hosts == 0) != (len(w.Rates) == 0) {
				t.Errorf("%s worker %d has %d client hosts and %d rates", p.AgentID, w.WorkerID, hosts, len(w.Rates))
			}
			for _, r := range w.Rates {
				total += *r.ConcurrentSessions
			}
		}
	}
	if total != 100 {
		t.Fatalf("sessions sum to %d, want 100", total)
	}
}

func TestModuleSettingsAreCarriedInThePlan(t *testing.T) {
	c := goldenCases()[6]
	plans := compileCase(t, c, c.agents)
	for _, m := range plans[0].Modules {
		want := m.Name == "dns"
		if got := m.Settings["zone_file"] == "zones/example.test.zone"; got != want {
			t.Errorf("module %s carries settings %v", m.Name, m.Settings)
		}
	}
}

func TestSeed(t *testing.T) {
	seedless := func(t *testing.T) Request {
		cfg := loadConfig(t, filepath.Join("testdata", "two_agents.yaml"))
		cfg.Timeline.Seed = nil
		return Request{RunID: "r", Config: cfg, Agents: []Agent{{ID: "a", Cores: 1}}, Scenarios: exampleCatalog(t)}
	}
	tests := []struct {
		name string
		rand []byte
		want uint64
	}{
		{"low bytes", []byte{0, 0, 0, 0, 0, 0, 0, 5}, 6},
		{"zero bytes", make([]byte, 8), 1},
		{"largest value wraps to one", []byte{255, 255, 255, 255, 255, 255, 255, 255}, 1},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			req := seedless(t)
			req.Rand = bytes.NewReader(tt.rand)
			plans, err := Compile(req)
			if err != nil {
				t.Fatal(err)
			}
			if plans[0].Seed != tt.want {
				t.Fatalf("seed %d, want %d", plans[0].Seed, tt.want)
			}
		})
	}
	t.Run("default source", func(t *testing.T) {
		plans, err := Compile(seedless(t))
		if err != nil {
			t.Fatal(err)
		}
		if plans[0].Seed == 0 {
			t.Fatal("the random seed is zero")
		}
	})
	t.Run("failing source", func(t *testing.T) {
		req := seedless(t)
		req.Rand = bytes.NewReader([]byte{1})
		if _, err := Compile(req); !errors.Is(err, io.ErrUnexpectedEOF) {
			t.Fatalf("error %v, want an unexpected EOF", err)
		}
	})
	t.Run("configuration wins over request", func(t *testing.T) {
		req := seedless(t)
		req.Seed = 99
		seed := uint64(7)
		req.Config.Timeline.Seed = &seed
		plans, err := Compile(req)
		if err != nil {
			t.Fatal(err)
		}
		if plans[0].Seed != 7 {
			t.Fatalf("seed %d, want 7", plans[0].Seed)
		}
	})
}

func TestSeedFallsBackToTheRequest(t *testing.T) {
	cfg := loadConfig(t, filepath.Join("testdata", "two_agents.yaml"))
	cfg.Timeline.Seed = nil
	plans, err := Compile(Request{RunID: "r", Seed: 99, Config: cfg, Agents: []Agent{{ID: "a", Cores: 1}}, Scenarios: exampleCatalog(t)})
	if err != nil {
		t.Fatal(err)
	}
	if plans[0].Seed != 99 {
		t.Fatalf("seed %d, want 99", plans[0].Seed)
	}
}

func TestCompileErrors(t *testing.T) {
	base := func(t *testing.T) Request {
		return Request{
			RunID:     "r",
			Config:    loadConfig(t, filepath.Join("testdata", "two_agents.yaml")),
			Agents:    []Agent{{ID: "a", Cores: 2}},
			Scenarios: exampleCatalog(t),
		}
	}
	tests := []struct {
		name   string
		mutate func(*Request)
		want   error
		text   string
	}{
		{"no agents", func(r *Request) { r.Agents = nil }, ErrInvalidInventory, "no agents"},
		{"duplicate agent", func(r *Request) { r.Agents = append(r.Agents, Agent{ID: "a", Cores: 1}) }, ErrInvalidInventory, "duplicate agent a"},
		{"zero cores", func(r *Request) { r.Agents[0].Cores = 0 }, ErrInvalidInventory, "no cores"},
		{"unknown backend", func(r *Request) { r.Agents[0].Backend = "netmap" }, ErrInvalidInventory, "unknown backend"},
		{"missing interface", func(r *Request) { r.Agents[0].Interfaces = []string{"eth1"} }, ErrInvalidInventory, "no agent has interface eth2"},
		{"pinned to unknown agent", func(r *Request) { r.Config.Network.Ports[0].Location = "agent-z/eth1" }, ErrInvalidInventory, "pinned to agent agent-z"},
		{"pinned agent lacks interface", func(r *Request) {
			r.Config.Network.Ports[0].Location = "a/eth1"
			r.Agents[0].Interfaces = []string{"eth2"}
		}, ErrInvalidInventory, "agent a has no interface eth1"},
		{"bad location", func(r *Request) { r.Config.Network.Ports[0].Location = "a/b/c" }, ErrInvalidConfig, "location"},
		{"seed zero", func(r *Request) {
			zero := uint64(0)
			r.Config.Timeline.Seed = &zero
		}, ErrInvalidConfig, "seed 0 is not allowed"},
		{"settings of the wrong type", func(r *Request) {
			r.Config.Modules = map[string]ModuleConfig{"dns": {Settings: map[string]any{"zone_file": 5}}}
		}, ErrInvalidConfig, "modules.dns.settings"},
		{"unknown setting", func(r *Request) {
			r.Config.Modules = map[string]ModuleConfig{"dns": {Settings: map[string]any{"zone_file": "z", "extra": true}}}
		}, ErrInvalidConfig, "modules.dns.settings"},
		{"module without a settings schema", func(r *Request) {
			r.Config.Modules = map[string]ModuleConfig{"http1": {Settings: map[string]any{"zone_file": "z"}}}
		}, ErrInvalidConfig, "module http1 accepts no settings"},
		{"settings of an unknown module", func(r *Request) {
			r.Config.Modules = map[string]ModuleConfig{"nope": {Settings: map[string]any{"a": 1}}}
		}, ErrUnknownModule, "modules.nope"},
		{"settings of a module outside the mix", func(r *Request) {
			r.Scenarios = append(r.Scenarios, Scenario{Module: ModuleRef{Name: "extra"}, Name: "idle", Source: "x"})
			r.Config.Modules = map[string]ModuleConfig{"extra": {}}
		}, ErrUnknownModule, "no scenario of the profile mix"},
		{"unknown scenario", func(r *Request) { r.Config.Profile.Mix[0].Scenario = "http1/nothing" }, ErrUnknownScenario, "profile.mix[0].scenario"},
		{"ambiguous scenario", func(r *Request) {
			r.Scenarios = append(r.Scenarios, Scenario{Module: ModuleRef{Name: "web2"}, Name: "web_get", Source: "x"})
		}, ErrAmbiguousScenario, "web_get in modules http1, web2"},
		{"duplicate catalog entry", func(r *Request) { r.Scenarios = append(r.Scenarios, r.Scenarios[0]) }, ErrInvalidCatalog, "duplicate scenario"},
		{"pool too small", func(r *Request) {
			r.Config.Network.HostPools[1].Count = 1
			r.Agents = []Agent{{ID: "a", Cores: 1}, {ID: "b", Cores: 1}}
		}, ErrPoolTooSmall, "pool servers has 1 hosts"},
		{"pool beyond prefix", func(r *Request) { r.Config.Network.HostPools[1].Count = 255 }, ErrInvalidConfig, "count 255 exceeds 254 usable addresses"},
		{"gateway outside prefix", func(r *Request) { r.Config.Network.HostPools[0].Gateway = "10.9.0.1" }, ErrInvalidConfig, "outside"},
		{"prefix with host bits", func(r *Request) { r.Config.Network.HostPools[0].IPv4 = "10.0.0.1/22" }, ErrInvalidConfig, "host bits"},
		{"wrong pool role", func(r *Request) { r.Config.Profile.Mix[0].ClientPool = "servers" }, ErrInvalidConfig, "has role server"},
		{"unknown pool", func(r *Request) { r.Config.Profile.Mix[0].ServerPool = "nope" }, ErrInvalidConfig, "unknown host pool"},
		{"unknown port", func(r *Request) { r.Config.Network.HostPools[0].Port = "nope" }, ErrInvalidConfig, "unknown port"},
		{"duplicate pool", func(r *Request) { r.Config.Network.HostPools[1].Name = "clients" }, ErrInvalidConfig, "duplicate host pool"},
		{"app_sim without servers", func(r *Request) {
			r.Config.Network.HostPools[1].Role = "client"
			r.Config.Profile.Mix[0].ServerPool = ""
		}, ErrInvalidConfig, "app_sim needs"},
		{"client_sim without servers", func(r *Request) { r.Config.Topology = "client_sim" }, ErrInvalidConfig, "client_sim needs"},
		{"zero weight", func(r *Request) { r.Config.Profile.Mix[1].Weight = 0 }, ErrInvalidConfig, "not positive"},
		{"bad shape", func(r *Request) { r.Config.Load.Phases[0].Shape = "sine" }, ErrInvalidConfig, "unknown shape"},
		{"duplicate phase", func(r *Request) { r.Config.Load.Phases[1].Name = "ramp_up" }, ErrInvalidConfig, "duplicate name"},
		{"bad op", func(r *Request) { r.Config.Criteria[0].Op = "=~" }, ErrInvalidConfig, "unknown op"},
		{"bad kind", func(r *Request) { r.Config.Kind = "Module" }, ErrInvalidConfig, "kind"},
		{"tiny concurrency", func(r *Request) {
			r.Config.Load.Objective = Objective{Type: "concurrent_sessions", Value: 0.2}
		}, ErrInvalidConfig, "does not round"},
		{"no byte estimate", func(r *Request) {
			r.Config.Load.Objective = Objective{Type: "throughput_bps", Value: 1e9}
			r.Scenarios[0].BytesPerSession = 0
			for i := range r.Scenarios {
				if r.Scenarios[i].Name == "lookup" {
					r.Scenarios[i].BytesPerSession = 0
				}
			}
		}, ErrNoByteEstimate, "dns/lookup"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			req := base(t)
			tt.mutate(&req)
			_, err := Compile(req)
			if !errors.Is(err, tt.want) {
				t.Fatalf("error %v, want %v", err, tt.want)
			}
			if !strings.Contains(err.Error(), tt.text) || !strings.Contains(err.Error(), `compile test "two_agents"`) {
				t.Fatalf("error %q lacks %q or the test name", err, tt.text)
			}
		})
	}
}

func TestParseConfigRejectsUnknownKeys(t *testing.T) {
	_, err := ParseConfig([]byte("apiVersion: fixingpoint.dev/v1alpha1\nkind: Test\nmetdata: {}\n"))
	if !errors.Is(err, ErrInvalidConfig) || !strings.Contains(err.Error(), "metdata") {
		t.Fatalf("error %v, want an unknown key error", err)
	}
	if _, err := ParseConfig(nil); !errors.Is(err, ErrInvalidConfig) {
		t.Fatalf("error %v for an empty document", err)
	}
}

func TestEvenSplit(t *testing.T) {
	tests := []struct {
		n     uint64
		k     int
		start int
		want  []uint64
	}{
		{254, 5, 0, []uint64{51, 51, 51, 51, 50}},
		{7, 5, 0, []uint64{2, 2, 1, 1, 1}},
		{7, 5, 4, []uint64{2, 1, 1, 1, 2}},
		{1, 3, 0, []uint64{1, 0, 0}},
		{6, 3, 1, []uint64{2, 2, 2}},
	}
	for _, tt := range tests {
		got := evenSplit(tt.n, tt.k, tt.start)
		for i := range got {
			if got[i] != tt.want[i] {
				t.Fatalf("evenSplit(%d, %d, %d) = %v, want %v", tt.n, tt.k, tt.start, got, tt.want)
			}
		}
	}
}

func TestLargestRemainder(t *testing.T) {
	tests := []struct {
		n       uint64
		weights []float64
		want    []uint64
	}{
		{1001, []float64{0.5, 0.3, 0.2}, []uint64{501, 300, 200}},
		{10, []float64{1, 1, 1}, []uint64{4, 3, 3}},
		{1, []float64{0.1, 0.9}, []uint64{0, 1}},
		{3, []float64{1.0 / 3, 1.0 / 3, 1.0 / 3}, []uint64{1, 1, 1}},
	}
	for _, tt := range tests {
		got := largestRemainder(tt.n, tt.weights)
		var sum uint64
		for i := range got {
			sum += got[i]
			if got[i] != tt.want[i] {
				t.Fatalf("largestRemainder(%d, %v) = %v, want %v", tt.n, tt.weights, got, tt.want)
			}
		}
		if sum != tt.n {
			t.Fatalf("parts sum to %d, want %d", sum, tt.n)
		}
	}
}

func TestDurationFromSeconds(t *testing.T) {
	tests := []struct {
		in   float64
		want Duration
	}{
		{0, ""}, {30, "30s"}, {2.5, "2.500s"}, {0.000001, "0.000001s"}, {1.0000000005, "1.000000001s"}, {600, "600s"},
	}
	for _, tt := range tests {
		got, err := durationFromSeconds(tt.in)
		if err != nil || got != tt.want {
			t.Fatalf("durationFromSeconds(%v) = %q, %v, want %q", tt.in, got, err, tt.want)
		}
	}
	for _, bad := range []float64{-1, math.Inf(1), math.NaN()} {
		if _, err := durationFromSeconds(bad); err == nil {
			t.Fatalf("durationFromSeconds(%v) accepted", bad)
		}
	}
}

func TestPhaseFractions(t *testing.T) {
	c := compiled{}
	cfg := Config{Load: Load{
		Objective: Objective{Type: "sessions_per_second", Value: 1},
		Phases: []PhaseConfig{
			{Name: "up", DurationS: 4, Shape: "linear"},
			{Name: "hold", DurationS: 4},
			{Name: "down", DurationS: 3.2, Shape: "step"},
			{Name: "up_again", DurationS: 1, Shape: "step"},
		},
	}}
	if err := c.loadAndCriteria(cfg); err != nil {
		t.Fatal(err)
	}
	want := [][3]float64{{0, 1, 0}, {1, 1, 0}, {1, 0, 4}, {0, 1, 1}}
	for i, p := range c.phases {
		if got := [3]float64{p.StartFraction, p.EndFraction, float64(p.Steps)}; got != want[i] {
			t.Errorf("phase %s: start, end, steps %v, want %v", p.Name, got, want[i])
		}
	}
}

func TestUsableHosts(t *testing.T) {
	tests := []struct {
		prefix, gateway string
		want            uint64
	}{
		{"10.0.0.0/24", "", 254}, {"10.0.0.0/24", "10.0.0.1", 253}, {"10.0.0.0/31", "", 1},
		{"2001:db8::/120", "", 255}, {"2001:db8::/120", "10.0.0.1", 255}, {"2001:db8::/64", "", math.MaxInt64},
	}
	for _, tt := range tests {
		var gw netip.Addr
		if tt.gateway != "" {
			gw = netip.MustParseAddr(tt.gateway)
		}
		got, err := usableHosts(tt.prefix, gw)
		if err != nil || got != tt.want {
			t.Errorf("usableHosts(%s, %s) = %d, %v, want %d", tt.prefix, tt.gateway, got, err, tt.want)
		}
	}
}
