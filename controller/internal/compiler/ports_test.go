// SPDX-License-Identifier: GPL-3.0-only

package compiler

import (
	"errors"
	"strings"
	"testing"
)

// portConfig returns a test configuration with one client pool and two server
// pools, whose mix is the given YAML entries.
func portConfig(t *testing.T, topology, mix string) Config {
	t.Helper()
	doc := `apiVersion: fixingpoint.dev/v1alpha1
kind: Test
metadata:
  name: port_clash
topology: ` + topology + `
network:
  ports:
    - { name: client0, location: local/lo }
    - { name: server0, location: local/lo }
  host_pools:
    - { name: clients, port: client0, role: client, ipv4: 16.0.0.0/24, count: 100 }
    - { name: servers, port: server0, role: server, ipv4: 48.0.0.0/24, count: 4 }
    - { name: servers2, port: server0, role: server, ipv4: 49.0.0.0/24, count: 4 }
  external_servers:
    - { name: ext, address: 192.0.2.1, port: 80 }
profile:
  basis: sessions
  mix:
` + mix + `
load:
  objective: { type: sessions_per_second, value: 100 }
  phases:
    - { name: steady, duration_s: 10, shape: hold }
timeline: { seed: 1 }
`
	cfg, err := ParseConfig([]byte(doc))
	if err != nil {
		t.Fatal(err)
	}
	return cfg
}

func TestServerPortClashNamesBothEntriesAndTheRemedy(t *testing.T) {
	cfg := portConfig(t, "app_sim", `    - { scenario: http1/web_get, weight: 1 }
    - { scenario: dns/lookup, weight: 1 }
    - { scenario: http1/web_browse, weight: 1 }`)
	clashes := ServerPortClashes(cfg, exampleCatalog(t))
	if len(clashes) != 1 || clashes[0].Index != 2 || clashes[0].First != 0 {
		t.Fatalf("clashes %+v, want one at entry 2 against entry 0", clashes)
	}
	for _, want := range []string{"http1/web_get", "http1/web_browse", "tcp port 8080", `"servers"`, "server_port", "server_pool"} {
		if !strings.Contains(clashes[0].Message, want) {
			t.Errorf("message %q lacks %q", clashes[0].Message, want)
		}
	}
	_, err := Compile(Request{
		RunID: "run-0001", Config: cfg, Scenarios: exampleCatalog(t),
		Agents: []Agent{{ID: "local", Cores: 1, Interfaces: []string{"lo"}}},
	})
	if !errors.Is(err, ErrInvalidConfig) || !strings.Contains(err.Error(), "profile.mix[2]: ") || !strings.Contains(err.Error(), clashes[0].Message) {
		t.Fatalf("Compile error %v, want ErrInvalidConfig at profile.mix[2] with the clash message", err)
	}
}

func TestServerPortParamClearsClash(t *testing.T) {
	cfg := portConfig(t, "app_sim", `    - { scenario: http1/web_get, weight: 1 }
    - { scenario: http1/web_browse, weight: 1, params: { server_port: 8081 } }`)
	if got := ServerPortClashes(cfg, exampleCatalog(t)); len(got) != 0 {
		t.Fatalf("clashes %+v, want none when a param moves the port", got)
	}
	if _, err := Compile(Request{
		RunID: "run-0001", Config: cfg, Scenarios: exampleCatalog(t),
		Agents: []Agent{{ID: "local", Cores: 1, Interfaces: []string{"lo"}}},
	}); err != nil {
		t.Fatalf("Compile: %v", err)
	}
	// A param that moves one entry onto the port of another is a clash again.
	cfg = portConfig(t, "app_sim", `    - { scenario: http1/web_get, weight: 1, params: { server_port: 8081 } }
    - { scenario: http1/web_browse, weight: 1, params: { server_port: 8081 } }`)
	if got := ServerPortClashes(cfg, exampleCatalog(t)); len(got) != 1 || !strings.Contains(got[0].Message, "tcp port 8081") {
		t.Fatalf("clashes %+v, want one on tcp port 8081", got)
	}
}

func TestServerPortOwnPoolOrTransportClearsClash(t *testing.T) {
	cfg := portConfig(t, "app_sim", `    - { scenario: http1/web_get, weight: 1 }
    - { scenario: http1/web_browse, weight: 1, server_pool: servers2 }
    - { scenario: dns/lookup, weight: 1 }
    - { scenario: dns/resolve, weight: 1, server_pool: servers2 }`)
	// Entry 1 has its own pool, and the dns scenarios are udp on port 53, so
	// only entries 2 and 3 would share a port, and they use different pools.
	if got := ServerPortClashes(cfg, exampleCatalog(t)); len(got) != 0 {
		t.Fatalf("clashes %+v, want none", got)
	}
	// An entry without a pool listens in the first server pool.
	cfg = portConfig(t, "app_sim", `    - { scenario: http1/web_get, weight: 1 }
    - { scenario: http1/web_browse, weight: 1, server_pool: servers }`)
	if got := ServerPortClashes(cfg, exampleCatalog(t)); len(got) != 1 || got[0].Index != 1 {
		t.Fatalf("clashes %+v, want one at entry 1 because servers is the default pool", got)
	}
}

func TestServerPortClientSimHasNoFinding(t *testing.T) {
	cfg := portConfig(t, "client_sim", `    - { scenario: http1/web_get, weight: 1 }
    - { scenario: http1/web_browse, weight: 1 }`)
	if got := ServerPortClashes(cfg, exampleCatalog(t)); len(got) != 0 {
		t.Fatalf("clashes %+v, want none because client_sim starts no servers", got)
	}
	if err := checkServerPorts(cfg, exampleCatalog(t)); err != nil {
		t.Fatalf("checkServerPorts: %v", err)
	}
}

func TestServerPortLiteralAcceptPort(t *testing.T) {
	const source = `scenario: echo
flows:
  - id: main
    client:
      - connect: { port: 9000 }
    server:
      - accept: { port: 9000 }
`
	mod := ModuleRef{Name: "lit", Version: "1.0.0", Digest: "sha256:00"}
	catalog := []Scenario{
		{Module: mod, Name: "echo", Source: source},
		{Module: mod, Name: "echo_again", Source: strings.Replace(source, "scenario: echo", "scenario: echo_again", 1)},
	}
	cfg := portConfig(t, "server_sim", `    - { scenario: lit/echo, weight: 1 }
    - { scenario: lit/echo_again, weight: 1 }`)
	got := ServerPortClashes(cfg, catalog)
	if len(got) != 1 || got[0].Index != 1 || !strings.Contains(got[0].Message, "tcp port 9000") {
		t.Fatalf("clashes %+v, want one on tcp port 9000 at entry 1", got)
	}
	// A server_port param cannot move a literal port, so only a pool clears it.
	cfg = portConfig(t, "server_sim", `    - { scenario: lit/echo, weight: 1 }
    - { scenario: lit/echo_again, weight: 1, server_pool: servers2 }`)
	if got := ServerPortClashes(cfg, catalog); len(got) != 0 {
		t.Fatalf("clashes %+v, want none with its own server pool", got)
	}
}
