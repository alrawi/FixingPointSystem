// SPDX-License-Identifier: GPL-3.0-only

package store

import (
	"context"
	"fmt"
	"regexp"
	"sort"
	"strings"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
)

// ModuleCatalog lists the scenarios of the installed modules. The module
// registry satisfies it.
type ModuleCatalog interface {
	// Scenarios returns every scenario that an installed module exports, in
	// any order.
	Scenarios(ctx context.Context) ([]compiler.Scenario, error)
}

// AgentInventory lists the agents that can take a run. The agent manager
// satisfies it.
type AgentInventory interface {
	// AvailableAgents returns the registered agents that are online or
	// reserved, in any order.
	AvailableAgents(ctx context.Context) ([]compiler.Agent, error)
}

// dryRunID is the run id of the compiler dry run, which never reaches an agent.
const dryRunID = "validate"

// Validator checks a stored configuration against the schema, its own cross
// references, the module catalog, and the agent inventory.
type Validator struct {
	schema  *Schema
	modules ModuleCatalog
	agents  AgentInventory
}

// NewValidator returns a validator. A nil modules or agents skips that check
// and adds an info finding that says so.
func NewValidator(schema *Schema, modules ModuleCatalog, agents AgentInventory) *Validator {
	return &Validator{schema: schema, modules: modules, agents: agents}
}

// Validate returns the report for the canonical JSON document doc. The error
// is not nil only when a dependency fails, and findings carry every problem
// of the document.
func (v *Validator) Validate(ctx context.Context, doc []byte) (api.ValidationReport, error) {
	findings, err := v.schema.Check(doc)
	if err != nil {
		return api.ValidationReport{}, err
	}
	if len(findings) > 0 {
		return report(findings, nil), nil
	}
	cfg, err := compiler.ParseConfig(doc)
	if err != nil {
		return report([]api.Finding{errorFinding("", err.Error())}, nil), nil
	}
	findings = append(findings, references(cfg)...)

	var catalog []compiler.Scenario
	if v.modules == nil {
		findings = append(findings, infoFinding("/profile/mix", "module catalog unavailable, scenario references not checked"))
	} else {
		if catalog, err = v.modules.Scenarios(ctx); err != nil {
			return api.ValidationReport{}, fmt.Errorf("validate config %s: list scenarios: %w", cfg.Metadata.Name, err)
		}
		findings = append(findings, scenarioRefs(cfg, catalog)...)
	}

	var agents []compiler.Agent
	capacity := &capacityReport{}
	if v.agents == nil {
		findings = append(findings, infoFinding("/network/ports", "agent inventory unavailable, capacity not checked"))
		capacity.required = len(requiredAgents(cfg))
	} else {
		inv, err := v.agents.AvailableAgents(ctx)
		if err != nil {
			return api.ValidationReport{}, fmt.Errorf("validate config %s: list agents: %w", cfg.Metadata.Name, err)
		}
		var fs []api.Finding
		agents, fs, capacity = checkCapacity(cfg, inv)
		findings = append(findings, fs...)
	}

	if v.modules != nil && v.agents != nil && !hasError(findings) {
		_, err := compiler.Compile(compiler.Request{RunID: dryRunID, Config: cfg, Agents: agents, Scenarios: catalog})
		if err != nil {
			findings = append(findings, errorFinding(compilerPath(err), err.Error()))
		}
	}
	return report(sortFindings(findings), capacity), nil
}

// capacityReport holds the capacity part of the report.
type capacityReport struct {
	required, available int
	known               bool
}

func report(findings []api.Finding, c *capacityReport) api.ValidationReport {
	if findings == nil {
		findings = []api.Finding{}
	}
	r := api.ValidationReport{Valid: !hasError(findings), Findings: findings}
	if c != nil {
		required := c.required
		r.Capacity = &struct {
			AgentsAvailable *int `json:"agents_available,omitempty"`
			AgentsRequired  *int `json:"agents_required,omitempty"`
		}{AgentsRequired: &required}
		if c.known {
			available := c.available
			r.Capacity.AgentsAvailable = &available
		}
	}
	return r
}

func hasError(fs []api.Finding) bool {
	for _, f := range fs {
		if f.Severity == api.FindingSeverityError {
			return true
		}
	}
	return false
}

func infoFinding(path, msg string) api.Finding {
	return api.Finding{Severity: api.FindingSeverityInfo, Path: path, Message: msg}
}

// references checks the names that one part of the document uses to point at
// another part, which the schema cannot express.
func references(cfg compiler.Config) []api.Finding {
	var out []api.Finding
	ports := map[string]bool{}
	for i, p := range cfg.Network.Ports {
		if ports[p.Name] {
			out = append(out, errorFinding(fmt.Sprintf("/network/ports/%d/name", i), fmt.Sprintf("duplicate port name %s", p.Name)))
		}
		ports[p.Name] = true
	}
	roles := map[string]string{}
	have := map[string]bool{}
	for i, hp := range cfg.Network.HostPools {
		at := fmt.Sprintf("/network/host_pools/%d", i)
		if _, dup := roles[hp.Name]; dup {
			out = append(out, errorFinding(at+"/name", fmt.Sprintf("duplicate host pool name %s", hp.Name)))
		}
		roles[hp.Name] = hp.Role
		have[hp.Role] = true
		if !ports[hp.Port] {
			out = append(out, errorFinding(at+"/port", fmt.Sprintf("unknown port %s", hp.Port)))
		}
	}
	switch {
	case cfg.Topology == "app_sim" && (!have["client"] || !have["server"]):
		out = append(out, errorFinding("/network/host_pools", "app_sim needs a client pool and a server pool"))
	case cfg.Topology == "client_sim" && len(cfg.Network.ExternalServers) == 0:
		out = append(out, errorFinding("/network/external_servers", "client_sim needs external servers"))
	}
	for i, m := range cfg.Profile.Mix {
		for _, link := range []struct{ field, pool, role string }{
			{"client_pool", m.ClientPool, "client"}, {"server_pool", m.ServerPool, "server"},
		} {
			if link.pool == "" {
				continue
			}
			at := fmt.Sprintf("/profile/mix/%d/%s", i, link.field)
			role, ok := roles[link.pool]
			switch {
			case !ok:
				out = append(out, errorFinding(at, fmt.Sprintf("unknown host pool %s", link.pool)))
			case role != link.role:
				out = append(out, errorFinding(at, fmt.Sprintf("host pool %s has role %s, not %s", link.pool, role, link.role)))
			}
		}
	}
	phases := map[string]bool{}
	for i, ph := range cfg.Load.Phases {
		if phases[ph.Name] {
			out = append(out, errorFinding(fmt.Sprintf("/load/phases/%d/name", i), fmt.Sprintf("duplicate phase name %s", ph.Name)))
		}
		phases[ph.Name] = true
	}
	return out
}

// scenarioRefs resolves every mix entry against the catalog. A reference
// <module>/<scenario> must exist, and a bare name must be unique.
func scenarioRefs(cfg compiler.Config, catalog []compiler.Scenario) []api.Finding {
	byRef := map[string]bool{}
	byName := map[string][]string{}
	for _, sc := range catalog {
		ref := sc.Module.Name + "/" + sc.Name
		if !byRef[ref] {
			byName[sc.Name] = append(byName[sc.Name], sc.Module.Name)
		}
		byRef[ref] = true
	}
	var out []api.Finding
	for i, m := range cfg.Profile.Mix {
		at := fmt.Sprintf("/profile/mix/%d/scenario", i)
		if strings.Contains(m.Scenario, "/") {
			if !byRef[m.Scenario] {
				mod, _, _ := strings.Cut(m.Scenario, "/")
				out = append(out, errorFinding(at, fmt.Sprintf("no installed module %s exports scenario %s", mod, m.Scenario)))
			}
			continue
		}
		switch mods := byName[m.Scenario]; len(mods) {
		case 0:
			out = append(out, errorFinding(at, fmt.Sprintf("no installed module exports scenario %s", m.Scenario)))
		case 1:
		default:
			sort.Strings(mods)
			out = append(out, errorFinding(at, fmt.Sprintf("scenario %s is ambiguous, modules %s export it", m.Scenario, strings.Join(mods, ", "))))
		}
	}
	return out
}

// requiredAgents returns the distinct agent names of the port locations in
// order of first use.
func requiredAgents(cfg compiler.Config) []string {
	var names []string
	seen := map[string]bool{}
	for _, p := range cfg.Network.Ports {
		name, _, _ := strings.Cut(p.Location, "/")
		if !seen[name] {
			seen[name] = true
			names = append(names, name)
		}
	}
	return names
}

// checkCapacity matches the agents that the ports name with the inventory.
// It returns the matching agents, which the compiler dry run uses, the
// findings, and the capacity counts.
func checkCapacity(cfg compiler.Config, inv []compiler.Agent) ([]compiler.Agent, []api.Finding, *capacityReport) {
	byID := make(map[string]compiler.Agent, len(inv))
	for _, ag := range inv {
		byID[ag.ID] = ag
	}
	required := requiredAgents(cfg)
	var used []compiler.Agent
	for _, name := range required {
		if ag, ok := byID[name]; ok {
			used = append(used, ag)
		}
	}
	var out []api.Finding
	for i, p := range cfg.Network.Ports {
		at := fmt.Sprintf("/network/ports/%d/location", i)
		name, iface, _ := strings.Cut(p.Location, "/")
		ag, ok := byID[name]
		switch {
		case !ok:
			out = append(out, errorFinding(at, fmt.Sprintf("agent %s is not available", name)))
		case len(ag.Interfaces) > 0 && !contains(ag.Interfaces, iface):
			out = append(out, errorFinding(at, fmt.Sprintf("agent %s has no interface %s", name, iface)))
		}
	}
	return used, out, &capacityReport{required: len(required), available: len(used), known: true}
}

func contains(list []string, s string) bool {
	for _, v := range list {
		if v == s {
			return true
		}
	}
	return false
}

// compilerPathRE finds the dotted path that a compiler error names, such as
// network.host_pools[1].count.
var compilerPathRE = regexp.MustCompile(`\b((?:apiVersion|kind|metadata|topology|network|profile|load|timeline|criteria)(?:\.[a-z_]+|\[\d+\])*): `)

// compilerPath turns the path in a compiler error into a JSON Pointer, or the
// root pointer when the error names none.
func compilerPath(err error) string {
	m := compilerPathRE.FindStringSubmatch(err.Error())
	if m == nil {
		return ""
	}
	p := strings.NewReplacer("[", "/", "]", "", ".", "/").Replace(m[1])
	return "/" + p
}
