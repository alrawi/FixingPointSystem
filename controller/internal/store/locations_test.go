// SPDX-License-Identifier: GPL-3.0-only

package store

import (
	"context"
	"fmt"
	"strings"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
)

func TestSplitLocation(t *testing.T) {
	tests := []struct{ location, agent, iface string }{
		{"agent-a/eth1", "agent-a", "eth1"},
		{"*/eth1", "", "eth1"},
		{"eth1", "", "eth1"},
		{"local/lo", "local", "lo"},
		{"a.b-c/0000:01:00.0", "a.b-c", "0000:01:00.0"},
	}
	for _, tc := range tests {
		agent, iface := splitLocation(tc.location)
		if agent != tc.agent || iface != tc.iface {
			t.Errorf("splitLocation(%q) = %q, %q, want %q, %q", tc.location, agent, iface, tc.agent, tc.iface)
		}
	}
}

// locationConfig returns a configuration whose ports have the given locations.
func locationConfig(locations ...string) compiler.Config {
	var cfg compiler.Config
	for i, l := range locations {
		cfg.Network.Ports = append(cfg.Network.Ports, compiler.Port{Name: fmt.Sprintf("p%d", i), Location: l})
	}
	return cfg
}

func TestCheckCapacityLocations(t *testing.T) {
	inv := exampleInventory().agents
	tests := []struct {
		name      string
		locations []string
		wantErrAt []string
		required  int
		usedIDs   string // in inventory order
	}{
		{"pinned", []string{"agent-a/eth1"}, nil, 1, "[agent-a]"},
		{"bare spreads over agents with the interface", []string{"eth1"}, nil, 1, "[agent-a spare]"},
		{"star spreads like bare", []string{"*/eth2"}, nil, 1, "[agent-a spare]"},
		{"bare lo", []string{"lo"}, nil, 1, "[local spare]"},
		{"pinned and spread", []string{"local/lo", "eth1"}, nil, 1, "[local agent-a spare]"},
		{"two pinned", []string{"local/lo", "agent-a/eth1"}, nil, 2, "[local agent-a]"},
		{"bare is never an agent name", []string{"eth1", "eth2"}, nil, 1, "[agent-a spare]"},
		{"pinned agent missing", []string{"gone/eth1"}, []string{"/network/ports/0/location"}, 1, "[]"},
		{"pinned interface missing", []string{"agent-a/lo"}, []string{"/network/ports/0/location"}, 1, "[]"},
		// The agent spare lists no interfaces, so it has every interface.
		{"spread over spare only", []string{"eth9"}, nil, 1, "[spare]"},
	}
	for _, tc := range tests {
		used, fs, c := checkCapacity(locationConfig(tc.locations...), inv)
		ids := []string{}
		for _, ag := range used {
			ids = append(ids, ag.ID)
		}
		if got := fmt.Sprint(ids); got != tc.usedIDs {
			t.Errorf("%s: used agents %s, want %s", tc.name, got, tc.usedIDs)
		}
		if len(fs) != len(tc.wantErrAt) {
			t.Errorf("%s: findings %+v, want errors at %v", tc.name, fs, tc.wantErrAt)
		}
		for _, at := range tc.wantErrAt {
			if !hasFinding(fs, api.FindingSeverityError, at) {
				t.Errorf("%s: findings %+v lack an error at %s", tc.name, fs, at)
			}
		}
		if c.required != tc.required || c.available != len(used) || !c.known {
			t.Errorf("%s: capacity %+v, want %d required and %d available", tc.name, c, tc.required, len(used))
		}
	}
}

func TestCheckCapacitySpreadWithoutInterface(t *testing.T) {
	inv := []compiler.Agent{{ID: "a", Cores: 1, Interfaces: []string{"eth1"}}}
	_, fs, _ := checkCapacity(locationConfig("*/eth7", "eth8"), inv)
	if !hasFinding(fs, api.FindingSeverityError, "/network/ports/0/location") ||
		!hasFinding(fs, api.FindingSeverityError, "/network/ports/1/location") {
		t.Errorf("findings %+v, want an error for each spread port that no agent can run", fs)
	}
}

func TestRequiredAgents(t *testing.T) {
	for _, tc := range []struct {
		locations []string
		want      int
	}{
		{[]string{"eth1"}, 1},
		{[]string{"*/eth1", "eth2"}, 1},
		{[]string{"a/eth1", "a/eth2", "eth3"}, 1},
		{[]string{"a/eth1", "b/eth1"}, 2},
	} {
		if got := requiredAgents(locationConfig(tc.locations...)); got != tc.want {
			t.Errorf("requiredAgents(%v) = %d, want %d", tc.locations, got, tc.want)
		}
	}
}

// TestValidateSettingsExamples validates the examples that CON-11 moved out of
// tests-pending with the example modules, so bare and */ locations and module
// settings pass the store.
func TestValidateSettingsExamples(t *testing.T) {
	v := NewValidator(newSchema(t), exampleCatalog(t), exampleInventory())
	for _, name := range []string{"dns_zone", "filtered_criteria", "port_locations"} {
		ex := exampleNamed(t, name)
		rep, err := v.Validate(context.Background(), mustCanonical(t, ex.data))
		if err != nil {
			t.Fatalf("%s: %v", ex.name, err)
		}
		if !rep.Valid || len(rep.Findings) != 0 {
			t.Errorf("%s: report %+v, want valid without findings", ex.name, rep)
		}
	}
}

func TestValidateModuleSettingsPath(t *testing.T) {
	v := NewValidator(newSchema(t), exampleCatalog(t), exampleInventory())
	ex := exampleNamed(t, "dns_zone")
	data := strings.Replace(string(ex.data), "zone_file: zones/example.test.zone", "zone_file: 5", 1)
	rep, err := v.Validate(context.Background(), mustCanonical(t, []byte(data)))
	if err != nil {
		t.Fatal(err)
	}
	if rep.Valid || !hasFinding(rep.Findings, api.FindingSeverityError, "/modules/dns/settings") {
		t.Errorf("report %+v, want an error at /modules/dns/settings", rep)
	}
}

func TestCompilerPathCoversModules(t *testing.T) {
	tests := []struct {
		err  error
		want string
	}{
		{fmt.Errorf("compile test \"x\": %w: modules.dns.settings: bad", compiler.ErrInvalidConfig), "/modules/dns/settings"},
		{fmt.Errorf("compile test \"x\": modules.http1: %w: unused", compiler.ErrUnknownModule), "/modules/http1"},
	}
	for _, tc := range tests {
		if got := compilerPath(tc.err); got != tc.want {
			t.Errorf("compilerPath(%v) = %q, want %q", tc.err, got, tc.want)
		}
	}
}
