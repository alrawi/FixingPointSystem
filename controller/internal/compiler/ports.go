// SPDX-License-Identifier: GPL-3.0-only

package compiler

import (
	"encoding/json"
	"fmt"
	"math"
	"regexp"
	"strconv"
	"strings"

	"go.yaml.in/yaml/v3"
)

// PortClash is a mix entry that listens on a port that an earlier entry already uses.
type PortClash struct {
	// Index is the index of the later entry in profile.mix, where the error belongs.
	Index int
	// First is the index of the earlier entry.
	First int
	// Message names both entries and the remedy.
	Message string
}

// acceptPortVar matches an accept port that is a single {{ vars.<name> }}.
var acceptPortVar = regexp.MustCompile(`^\s*\{\{\s*vars\.([A-Za-z_][A-Za-z0-9_]*)\s*\}\}\s*$`)

// scenarioListen is the part of a scenario file that decides where its first flow listens.
type scenarioListen struct {
	Vars  map[string]any `yaml:"vars"`
	Flows []struct {
		L4     string           `yaml:"l4"`
		Server []map[string]any `yaml:"server"`
	} `yaml:"flows"`
}

// ServerPortClashes finds the mix entries that listen on a transport and port
// that an earlier entry listens on in the same server pool. A plan listens once
// for each port of each server host, and an agent refuses a plan in which two
// entries accept on one transport and port of the same hosts (bug 704).
//
// The check is narrow on purpose, like the agent. It reads only the first flow
// of a scenario, and only an accept port that is a literal or a single
// {{ vars.<name> }}, with the value of the param of the entry of that name or
// else the default of the scenario var. A client_sim test starts no servers.
// An entry whose scenario the catalog lacks, or whose port is not constant, is
// skipped, because other checks report the first and the agent the second.
// The result lists at most one clash for each entry, in mix order.
func ServerPortClashes(cfg Config, catalog []Scenario) []PortClash {
	if cfg.Topology == "client_sim" {
		return nil
	}
	byRef, byName, err := indexCatalog(catalog)
	if err != nil {
		return nil
	}
	defaultServer := defaultPool(cfg.Network.HostPools, "server")
	type key struct {
		pool, l4 string
		port     int
	}
	type owner struct {
		index    int
		scenario string
	}
	seen := map[key]owner{}
	var out []PortClash
	for i, m := range cfg.Profile.Mix {
		sc, err := resolve("", m.Scenario, byRef, byName)
		if err != nil {
			continue
		}
		var src scenarioListen
		if yaml.Unmarshal([]byte(sc.Source), &src) != nil || len(src.Flows) == 0 {
			continue
		}
		flow := src.Flows[0]
		var accept map[string]any
		for _, action := range flow.Server {
			if a, ok := action["accept"].(map[string]any); ok {
				accept = a
				break
			}
		}
		if accept == nil {
			continue
		}
		port, ok := constantAcceptPort(accept["port"], m, src.Vars)
		if !ok {
			continue
		}
		l4 := flow.L4
		if l4 == "" {
			l4 = "tcp"
		}
		pool := m.ServerPool
		if pool == "" {
			pool = defaultServer
		}
		k := key{pool: pool, l4: l4, port: port}
		first, dup := seen[k]
		if !dup {
			seen[k] = owner{index: i, scenario: m.Scenario}
			continue
		}
		out = append(out, PortClash{
			Index: i,
			First: first.index,
			Message: fmt.Sprintf("mix entries %d (%s) and %d (%s) both listen on %s port %d of server pool %q; "+
				"give one entry its own server_port in its params or its own server_pool",
				first.index, first.scenario, i, m.Scenario, l4, port, pool),
		})
	}
	return out
}

// checkServerPorts is the compile time form of ServerPortClashes. It returns
// the first clash as an ErrInvalidConfig error at profile.mix[<i>].
func checkServerPorts(cfg Config, catalog []Scenario) error {
	clashes := ServerPortClashes(cfg, catalog)
	if len(clashes) == 0 {
		return nil
	}
	return invalid(fmt.Sprintf("profile.mix[%d]", clashes[0].Index), "%s", clashes[0].Message)
}

// constantAcceptPort returns the port that an accept action names when it is
// constant: an integer or a digit string, or a single {{ vars.<name> }} that
// the entry params or the scenario vars give as an integer.
func constantAcceptPort(raw any, m MixConfig, vars map[string]any) (int, bool) {
	if s, ok := raw.(string); ok {
		if match := acceptPortVar.FindStringSubmatch(s); match != nil {
			value, set := m.Params[match[1]]
			if !set {
				value = vars[match[1]]
			}
			return integerValue(value)
		}
		digits := strings.TrimSpace(s)
		if digits == "" || strings.Trim(digits, "0123456789") != "" {
			return 0, false
		}
		n, err := strconv.Atoi(digits)
		return n, err == nil
	}
	return integerValue(raw)
}

// integerValue converts a decoded YAML or JSON number that holds an integer.
func integerValue(v any) (int, bool) {
	switch n := v.(type) {
	case int:
		return n, true
	case int64:
		return int(n), true
	case uint64:
		if n > math.MaxInt32 {
			return 0, false
		}
		return int(n), true
	case float64:
		if n != math.Trunc(n) || math.Abs(n) > math.MaxInt32 {
			return 0, false
		}
		return int(n), true
	case json.Number:
		i, err := n.Int64()
		return int(i), err == nil
	}
	return 0, false
}
