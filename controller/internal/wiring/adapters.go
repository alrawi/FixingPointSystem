// SPDX-License-Identifier: GPL-3.0-only

package wiring

import (
	"context"
	"encoding/json"
	"fmt"
	"io/fs"
	"os"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/registry"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
	"github.com/alrawi/FixingPointSystem/controller/internal/store"
)

// Catalog adapts the module registry to the scenario catalog that the
// configuration validator and the run orchestrator read. It lists the
// scenarios of the latest installed version of each module, because test
// configurations name a module without a version.
type Catalog struct {
	// Registry is the module registry.
	Registry *registry.Registry
}

var (
	_ store.ModuleCatalog = Catalog{}
	_ runs.Catalog        = Catalog{}
)

// Scenarios implements store.ModuleCatalog and runs.Catalog. It reads each
// scenario file of the latest version of every installed module.
func (c Catalog) Scenarios(ctx context.Context) ([]compiler.Scenario, error) {
	latest := map[string]registry.Entry{}
	var order []string
	for _, e := range c.Registry.List(ctx, "") {
		if _, seen := latest[e.Name]; !seen {
			order = append(order, e.Name)
		}
		latest[e.Name] = e // entries sort by version precedence, so the last wins
	}
	var out []compiler.Scenario
	for _, name := range order {
		e := latest[name]
		m, err := c.Registry.Manifest(ctx, e.Name, e.Version)
		if err != nil {
			return nil, fmt.Errorf("scenario catalog: %w", err)
		}
		if len(m.Scenarios) != len(e.Scenarios) {
			return nil, fmt.Errorf("scenario catalog: module %s %s: manifest names %d scenario files and the index %d scenarios",
				e.Name, e.Version, len(m.Scenarios), len(e.Scenarios))
		}
		schema, err := settingsSchema(m)
		if err != nil {
			return nil, fmt.Errorf("scenario catalog: module %s %s: %w", e.Name, e.Version, err)
		}
		tree := os.DirFS(c.Registry.Dir(e.Name, e.Version))
		for i, rel := range m.Scenarios {
			// fs.ReadFile rejects a path that leaves the module tree.
			src, err := fs.ReadFile(tree, rel)
			if err != nil {
				return nil, fmt.Errorf("scenario catalog: module %s %s: read scenario %s: %w", e.Name, e.Version, rel, err)
			}
			out = append(out, compiler.Scenario{
				Module:         compiler.ModuleRef{Name: e.Name, Version: e.Version, Digest: e.Digest},
				Name:           e.Scenarios[i],
				Source:         string(src),
				SettingsSchema: schema,
			})
		}
	}
	return out, nil
}

// settingsSchema returns the settings key of the manifest as JSON, or nil
// for a module without one, which accepts no settings.
func settingsSchema(m registry.Manifest) (json.RawMessage, error) {
	var doc struct {
		Settings json.RawMessage `json:"settings"`
	}
	if err := json.Unmarshal(m.Raw, &doc); err != nil {
		return nil, fmt.Errorf("read settings of the manifest: %w", err)
	}
	if len(doc.Settings) == 0 || string(doc.Settings) == "null" {
		return nil, nil
	}
	return doc.Settings, nil
}

// ModuleSummaries implements api.ModuleLister over every installed module
// version, in the order of Registry.List.
func (c Catalog) ModuleSummaries(ctx context.Context) ([]api.ModuleSummary, error) {
	entries := c.Registry.List(ctx, "")
	out := make([]api.ModuleSummary, 0, len(entries))
	for _, e := range entries {
		s := api.ModuleSummary{Name: e.Name, Version: e.Version, Type: api.ModuleType(e.Type), License: e.License}
		if e.Summary != "" {
			s.Summary = &e.Summary
		}
		if len(e.Scenarios) > 0 {
			sc := append([]string(nil), e.Scenarios...)
			s.Scenarios = &sc
		}
		out = append(out, s)
	}
	return out, nil
}

var _ api.ModuleLister = Catalog{}

// AgentLister is the part of the agent manager that the inventory adapter
// reads. *agents.Manager satisfies it.
type AgentLister interface {
	// List returns every known agent.
	List() []agents.Agent
}

// Inventory adapts the agent manager to the agent inventories of the
// configuration validator and the capabilities endpoint.
type Inventory struct {
	// Agents is the agent manager.
	Agents AgentLister
}

var (
	_ store.AgentInventory = Inventory{}
	_ api.AgentInventory   = Inventory{}
)

// compilerBackends maps the backends of the agent contract to the names that
// the compiler and the REST contract use.
var compilerBackends = map[agentpb.Backend]string{
	agentpb.Backend_BACKEND_KERNEL: "kernel",
	agentpb.Backend_BACKEND_AF_XDP: "af_xdp",
	agentpb.Backend_BACKEND_DPDK:   "dpdk",
}

// AvailableAgents implements store.AgentInventory. It returns the agents that
// are online or reserved, with the first backend that each agent reports, as
// the run orchestrator compiles them.
func (inv Inventory) AvailableAgents(context.Context) ([]compiler.Agent, error) {
	var out []compiler.Agent
	for _, ag := range inv.Agents.List() {
		if ag.State != api.AgentStateOnline && ag.State != api.AgentStateReserved {
			continue
		}
		c := ag.Capabilities
		ca := compiler.Agent{ID: ag.ID, Cores: c.GetCores()}
		for _, b := range c.GetBackends() {
			if name, ok := compilerBackends[b]; ok {
				ca.Backend = name
				break
			}
		}
		for _, p := range c.GetPorts() {
			ca.Interfaces = append(ca.Interfaces, p.GetName())
		}
		out = append(out, ca)
	}
	return out, nil
}

// AgentCapabilities implements api.AgentInventory. Each agent carries every
// backend of its latest registration.
func (inv Inventory) AgentCapabilities(context.Context) ([]api.AgentCapability, error) {
	list := inv.Agents.List()
	out := make([]api.AgentCapability, 0, len(list))
	for _, ag := range list {
		ac := api.AgentCapability{Agent: agents.ToAPI(ag)}
		for _, b := range ag.Capabilities.GetBackends() {
			if name, ok := compilerBackends[b]; ok {
				ac.Backends = append(ac.Backends, api.CapabilitiesBackends(name))
			}
		}
		out = append(out, ac)
	}
	return out, nil
}
