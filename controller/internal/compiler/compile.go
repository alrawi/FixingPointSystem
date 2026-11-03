// SPDX-License-Identifier: GPL-3.0-only

// Package compiler compiles a test configuration and an agent inventory into
// one plan per agent. Compile is a pure function. It reads no clock, draws no
// random numbers, and does no I/O, so the same request always yields the same
// plans.
//
// The compiler normalizes the mix by sessions, splits every host pool and
// every rate across the workers of all agents in proportion to their cores,
// and derives the load phase fractions. Byte based mix correction arrives in
// CTL-12.
package compiler

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"math"
	"net/netip"
	"regexp"
	"sort"
	"strings"
)

// DefaultEngineAPI is the engine contract version that plans target when the
// request names none.
const DefaultEngineAPI = "0.2.0"

// Sentinel errors that callers test with errors.Is.
var (
	// ErrInvalidConfig reports a test configuration that breaks a rule the compiler needs.
	ErrInvalidConfig = errors.New("invalid test configuration")
	// ErrInvalidInventory reports an agent inventory that cannot run any plan.
	ErrInvalidInventory = errors.New("invalid agent inventory")
	// ErrInvalidCatalog reports a scenario catalog with duplicate or inconsistent entries.
	ErrInvalidCatalog = errors.New("invalid scenario catalog")
	// ErrUnknownScenario reports a mix entry whose scenario the catalog lacks.
	ErrUnknownScenario = errors.New("unknown scenario")
	// ErrAmbiguousScenario reports a bare scenario name that several modules export.
	ErrAmbiguousScenario = errors.New("ambiguous scenario")
	// ErrPoolTooSmall reports a host pool that cannot give every agent at least one host.
	ErrPoolTooSmall = errors.New("host pool too small for the agents")
	// ErrNoByteEstimate reports a throughput objective with a scenario that has no bytes per session estimate.
	ErrNoByteEstimate = errors.New("no bytes per session estimate")
)

// Agent is one entry of the agent inventory.
type Agent struct {
	// ID is the agent identifier that the controller assigned.
	ID string
	// Cores is the number of workers that the agent can run. Worker i pins to core i.
	Cores uint32
	// Backend is kernel, af_xdp, or dpdk. Empty means kernel.
	Backend string
	// Interfaces lists the traffic interfaces of the agent. When it is not
	// empty, every port interface of the test must appear in it.
	Interfaces []string
}

// Scenario is one scenario that an installed module exports.
type Scenario struct {
	// Module pins the module that exports the scenario.
	Module ModuleRef
	// Name is the scenario name, the part of the ref after the slash.
	Name string
	// Source is the scenario file in YAML.
	Source string
	// BytesPerSession estimates the bytes that one session moves in both
	// directions. Only the throughput_bps objective needs it.
	BytesPerSession float64
}

// Request is the input of Compile.
type Request struct {
	// RunID names the run. Plan ids are <RunID>/<agent id>.
	RunID string
	// EngineAPI is the engine contract version. Empty means DefaultEngineAPI.
	EngineAPI string
	// Seed is the run seed when the configuration sets no timeline.seed. The
	// caller picks it at the process edge.
	Seed uint64
	// Config is the test configuration.
	Config Config
	// Agents is the inventory of agents that run the test, in any order.
	Agents []Agent
	// Scenarios is the catalog of installed scenarios, in any order.
	Scenarios []Scenario
}

var (
	namePattern     = regexp.MustCompile(`^[A-Za-z][A-Za-z0-9_.-]{0,62}$`)
	locationPattern = regexp.MustCompile(`^[A-Za-z0-9_.-]+/[A-Za-z0-9_.:-]+$`)
	scenarioPattern = regexp.MustCompile(`^[a-z][a-z0-9_]*(/[a-z][a-z0-9_]*)?$`)
	macPattern      = regexp.MustCompile(`^([0-9a-f]{2}:){5}[0-9a-f]{2}$`)
	metricPattern   = regexp.MustCompile(`^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$`)
)

var (
	topologies = map[string]bool{"app_sim": true, "client_sim": true, "server_sim": true}
	backends   = map[string]bool{"kernel": true, "af_xdp": true, "dpdk": true}
	objectives = map[string]bool{
		"concurrent_sessions": true, "sessions_per_second": true,
		"throughput_bps": true, "simulated_users": true,
	}
	criterionOps = map[string]string{
		"<": "CRITERION_OP_LT", "<=": "CRITERION_OP_LE", ">": "CRITERION_OP_GT",
		">=": "CRITERION_OP_GE", "==": "CRITERION_OP_EQ", "!=": "CRITERION_OP_NE",
	}
)

// worker is one worker of one agent in the global worker order.
type worker struct {
	agent int
	id    uint32
}

// compiled holds the parts of the plans that every agent shares.
type compiled struct {
	modules   []ModuleRef
	scenarios []ScenarioSource
	mix       []MixEntry
	ports     []PortBinding
	pools     []HostPoolSlice
	externals []ExternalServer
	phases    []LoadPhase
	warmup    Duration
	criteria  []Criterion
}

// Compile turns a test configuration and an agent inventory into one plan per
// agent, ordered by agent id. The result depends only on the request.
func Compile(req Request) ([]Plan, error) {
	name := req.Config.Metadata.Name
	plans, err := compile(req)
	if err != nil {
		return nil, fmt.Errorf("compile test %q: %w", name, err)
	}
	return plans, nil
}

func compile(req Request) ([]Plan, error) {
	cfg := req.Config
	if req.RunID == "" {
		return nil, errors.New("run id is empty")
	}
	agents, err := sortedAgents(req.Agents)
	if err != nil {
		return nil, err
	}
	if err := checkHeader(cfg); err != nil {
		return nil, err
	}
	var c compiled
	poolIndex, err := c.network(cfg, agents)
	if err != nil {
		return nil, err
	}
	estimates, err := c.profile(cfg, poolIndex, req.Scenarios)
	if err != nil {
		return nil, err
	}
	if err := c.loadAndCriteria(cfg); err != nil {
		return nil, err
	}

	workers := make([]worker, 0)
	cores := make([]uint32, len(agents))
	for a, ag := range agents {
		cores[a] = ag.Cores
		for i := uint32(0); i < ag.Cores; i++ {
			workers = append(workers, worker{agent: a, id: i})
		}
	}
	hosts, slices, err := splitPools(cfg.Network.HostPools, workers, len(agents))
	if err != nil {
		return nil, err
	}
	rates, agentValue, err := splitRates(cfg.Load.Objective, c.mix, estimates, workers, cores)
	if err != nil {
		return nil, err
	}

	engineAPI := req.EngineAPI
	if engineAPI == "" {
		engineAPI = DefaultEngineAPI
	}
	seed := req.Seed
	if cfg.Timeline.Seed != nil {
		seed = *cfg.Timeline.Seed
	}
	plans := make([]Plan, len(agents))
	for a, ag := range agents {
		backend := ag.Backend
		if backend == "" {
			backend = "kernel"
		}
		p := Plan{
			PlanID:          req.RunID + "/" + ag.ID,
			RunID:           req.RunID,
			AgentID:         ag.ID,
			TestName:        cfg.Metadata.Name,
			EngineAPI:       engineAPI,
			Seed:            seed,
			Topology:        enumName("TOPOLOGY_", cfg.Topology),
			Backend:         enumName("BACKEND_", backend),
			Modules:         c.modules,
			Scenarios:       c.scenarios,
			Mix:             c.mix,
			Ports:           c.ports,
			ExternalServers: c.externals,
			Load: &LoadPlan{
				Objective:      enumName("LOAD_OBJECTIVE_", cfg.Load.Objective.Type),
				ObjectiveValue: agentValue[a],
				Phases:         c.phases,
			},
			Warmup:   c.warmup,
			Criteria: c.criteria,
		}
		for i, pool := range c.pools {
			s := pool
			s.Offset, s.Count = slices[i][a].offset, slices[i][a].count
			p.HostPools = append(p.HostPools, s)
		}
		for g, w := range workers {
			if w.agent != a {
				continue
			}
			p.Workers = append(p.Workers, WorkerPlan{
				WorkerID: w.id,
				Core:     w.id,
				Hosts:    hosts[g],
				Rates:    rates[g],
			})
		}
		plans[a] = p
	}
	return plans, nil
}

func invalid(path, format string, args ...any) error {
	return fmt.Errorf("%w: %s: %s", ErrInvalidConfig, path, fmt.Sprintf(format, args...))
}

func sortedAgents(in []Agent) ([]Agent, error) {
	if len(in) == 0 {
		return nil, fmt.Errorf("%w: no agents", ErrInvalidInventory)
	}
	agents := append([]Agent(nil), in...)
	sort.Slice(agents, func(i, j int) bool { return agents[i].ID < agents[j].ID })
	var total uint64
	for i, ag := range agents {
		switch {
		case ag.ID == "" || strings.Contains(ag.ID, "/"):
			return nil, fmt.Errorf("%w: agent id %q is empty or contains a slash", ErrInvalidInventory, ag.ID)
		case i > 0 && agents[i-1].ID == ag.ID:
			return nil, fmt.Errorf("%w: duplicate agent %s", ErrInvalidInventory, ag.ID)
		case ag.Cores == 0:
			return nil, fmt.Errorf("%w: agent %s has no cores", ErrInvalidInventory, ag.ID)
		case ag.Backend != "" && !backends[ag.Backend]:
			return nil, fmt.Errorf("%w: agent %s has unknown backend %q", ErrInvalidInventory, ag.ID, ag.Backend)
		}
		total += uint64(ag.Cores)
	}
	if total > math.MaxUint32 {
		return nil, fmt.Errorf("%w: %d cores in total exceed the supported count", ErrInvalidInventory, total)
	}
	return agents, nil
}

func checkHeader(cfg Config) error {
	switch {
	case cfg.APIVersion != "fixingpoint.dev/v1alpha1":
		return invalid("apiVersion", "unsupported value %q", cfg.APIVersion)
	case cfg.Kind != "Test":
		return invalid("kind", "unsupported value %q", cfg.Kind)
	case !namePattern.MatchString(cfg.Metadata.Name):
		return invalid("metadata.name", "invalid name %q", cfg.Metadata.Name)
	case !topologies[cfg.Topology]:
		return invalid("topology", "unknown topology %q", cfg.Topology)
	}
	return nil
}

// network checks ports, host pools, and external servers and fills the
// shared parts of the plan. It returns the pool index by name.
func (c *compiled) network(cfg Config, agents []Agent) (map[string]int, error) {
	n := cfg.Network
	if len(n.Ports) == 0 {
		return nil, invalid("network.ports", "a test needs at least one port")
	}
	ports := make(map[string]bool, len(n.Ports))
	for i, p := range n.Ports {
		path := fmt.Sprintf("network.ports[%d]", i)
		switch {
		case !namePattern.MatchString(p.Name):
			return nil, invalid(path, "invalid name %q", p.Name)
		case ports[p.Name]:
			return nil, invalid(path, "duplicate port %s", p.Name)
		case !locationPattern.MatchString(p.Location):
			return nil, invalid(path, "location %q is not <agent>/<interface>", p.Location)
		case p.MTU != 0 && (p.MTU < 576 || p.MTU > 9216):
			return nil, invalid(path, "mtu %d is outside 576 to 9216", p.MTU)
		}
		ports[p.Name] = true
		iface := p.Location[strings.Index(p.Location, "/")+1:]
		for _, ag := range agents {
			if len(ag.Interfaces) > 0 && !contains(ag.Interfaces, iface) {
				return nil, fmt.Errorf("%w: agent %s has no interface %s for port %s", ErrInvalidInventory, ag.ID, iface, p.Name)
			}
		}
		c.ports = append(c.ports, PortBinding{Name: p.Name, InterfaceName: iface, MTU: uint32(p.MTU)}) //nolint:gosec // G115: checked against 9216 above
	}

	if len(n.HostPools) == 0 {
		return nil, invalid("network.host_pools", "a test needs at least one host pool")
	}
	index := make(map[string]int, len(n.HostPools))
	roles := map[string]bool{}
	for i, hp := range n.HostPools {
		path := fmt.Sprintf("network.host_pools[%d]", i)
		slice, err := poolSlice(path, i, hp, ports)
		if err != nil {
			return nil, err
		}
		if _, dup := index[hp.Name]; dup {
			return nil, invalid(path, "duplicate host pool %s", hp.Name)
		}
		index[hp.Name] = i
		roles[hp.Role] = true
		c.pools = append(c.pools, slice)
	}
	if cfg.Topology == "app_sim" && (!roles["client"] || !roles["server"]) {
		return nil, invalid("network.host_pools", "app_sim needs a client pool and a server pool")
	}

	for i, es := range n.ExternalServers {
		path := fmt.Sprintf("network.external_servers[%d]", i)
		switch {
		case !namePattern.MatchString(es.Name):
			return nil, invalid(path, "invalid name %q", es.Name)
		case es.Address == "":
			return nil, invalid(path, "address is empty")
		case es.Port < 0 || es.Port > 65535:
			return nil, invalid(path, "port %d is outside 1 to 65535", es.Port)
		}
		c.externals = append(c.externals, ExternalServer{Name: es.Name, Address: es.Address, Port: uint32(es.Port)}) //nolint:gosec // G115: checked against 65535 above
	}
	if cfg.Topology == "client_sim" && len(c.externals) == 0 {
		return nil, invalid("network.external_servers", "client_sim needs external servers")
	}
	return index, nil
}

func contains(list []string, s string) bool {
	for _, v := range list {
		if v == s {
			return true
		}
	}
	return false
}

// poolSlice checks one host pool and returns its shared slice fields.
func poolSlice(path string, index int, hp HostPool, ports map[string]bool) (HostPoolSlice, error) {
	switch {
	case !namePattern.MatchString(hp.Name):
		return HostPoolSlice{}, invalid(path, "invalid name %q", hp.Name)
	case !ports[hp.Port]:
		return HostPoolSlice{}, invalid(path, "unknown port %q", hp.Port)
	case hp.Role != "client" && hp.Role != "server":
		return HostPoolSlice{}, invalid(path, "unknown role %q", hp.Role)
	case hp.IPv4 == "" && hp.IPv6 == "":
		return HostPoolSlice{}, invalid(path, "a host pool needs ipv4 or ipv6")
	case hp.Count < 1 || hp.Count > math.MaxUint32:
		return HostPoolSlice{}, invalid(path, "count %d is outside 1 to %d", hp.Count, uint32(math.MaxUint32))
	case hp.VLAN < 0 || hp.VLAN > 4094:
		return HostPoolSlice{}, invalid(path, "vlan %d is outside 1 to 4094", hp.VLAN)
	}
	count := uint64(hp.Count) //nolint:gosec // G115: checked to lie in 1 to MaxUint32 above
	var gw netip.Addr
	if hp.Gateway != "" {
		var err error
		if gw, err = netip.ParseAddr(hp.Gateway); err != nil {
			return HostPoolSlice{}, invalid(path, "gateway %q is not an address", hp.Gateway)
		}
	}
	for _, prefix := range []string{hp.IPv4, hp.IPv6} {
		if prefix == "" {
			continue
		}
		usable, err := usableHosts(prefix, gw)
		if err != nil {
			return HostPoolSlice{}, invalid(path, "%v", err)
		}
		if count > usable {
			return HostPoolSlice{}, invalid(path, "count %d exceeds %d usable addresses of %s", count, usable, prefix)
		}
	}
	mac := hp.MACBase
	if mac == "" {
		// A locally administered base per pool keeps MAC addresses unique
		// across pools and agents. It leaves 24 bits for the host index.
		if count > 1<<24 || index >= 1<<16 {
			return HostPoolSlice{}, invalid(path, "count %d needs an explicit mac_base", count)
		}
		mac = fmt.Sprintf("02:%02x:%02x:00:00:00", index>>8, index&0xff)
	} else {
		if !macPattern.MatchString(mac) {
			return HostPoolSlice{}, invalid(path, "mac_base %q is not lowercase colon notation", mac)
		}
		if macValue(mac)+count-1 > 1<<48-1 {
			return HostPoolSlice{}, invalid(path, "mac_base %s plus %d hosts overflows", mac, count)
		}
	}
	return HostPoolSlice{
		Name:       hp.Name,
		Port:       hp.Port,
		Role:       enumName("HOST_ROLE_", hp.Role),
		IPv4Prefix: hp.IPv4,
		IPv6Prefix: hp.IPv6,
		PoolCount:  uint32(count), //nolint:gosec // G115: checked to lie in 1 to MaxUint32 above
		Gateway:    hp.Gateway,
		VLAN:       uint32(hp.VLAN), //nolint:gosec // G115: checked against 4094 above
		MACBase:    mac,
		OSProfile:  hp.OSProfile,
	}, nil
}

func macValue(mac string) uint64 {
	var v uint64
	for _, part := range strings.Split(mac, ":") {
		b, _ := hex.DecodeString(part) // the pattern guarantees two hex digits
		v = v<<8 | uint64(b[0])
	}
	return v
}

// usableHosts counts the host addresses of prefix with the rules of the plan
// contract. IPv4 loses the network and broadcast addresses, IPv6 loses the
// network address, and a gateway inside the prefix loses one more.
func usableHosts(prefix string, gw netip.Addr) (uint64, error) {
	p, err := netip.ParsePrefix(prefix)
	if err != nil {
		return 0, fmt.Errorf("prefix %q: %w", prefix, err)
	}
	if p.Masked() != p {
		return 0, fmt.Errorf("prefix %s has host bits set", prefix)
	}
	hostBits := p.Addr().BitLen() - p.Bits()
	if hostBits >= 63 {
		return math.MaxInt64, nil
	}
	usable := uint64(1)<<hostBits - 1
	if p.Addr().Is4() && p.Bits() < 31 {
		usable--
	}
	if gw.IsValid() && gw.Is4() == p.Addr().Is4() {
		if !p.Contains(gw) {
			return 0, fmt.Errorf("gateway %s is outside %s", gw, prefix)
		}
		usable--
	}
	return usable, nil
}

// profile resolves the mix against the catalog and normalizes the weights.
// It returns the bytes per session estimate of each mix entry.
func (c *compiled) profile(cfg Config, pools map[string]int, catalog []Scenario) ([]float64, error) {
	prof := cfg.Profile
	if prof.Basis != "sessions" && prof.Basis != "bytes" {
		return nil, invalid("profile.basis", "unknown basis %q", prof.Basis)
	}
	if len(prof.Mix) == 0 {
		return nil, invalid("profile.mix", "a test needs at least one mix entry")
	}
	byRef, byName, err := indexCatalog(catalog)
	if err != nil {
		return nil, err
	}
	var total float64
	for i, m := range prof.Mix {
		if math.IsNaN(m.Weight) || math.IsInf(m.Weight, 0) || m.Weight <= 0 {
			return nil, invalid(fmt.Sprintf("profile.mix[%d]", i), "weight %v is not positive", m.Weight)
		}
		total += m.Weight
	}
	modules := map[string]ModuleRef{}
	used := map[string]ScenarioSource{}
	ids := map[string]bool{}
	estimates := make([]float64, len(prof.Mix))
	for i, m := range prof.Mix {
		path := fmt.Sprintf("profile.mix[%d]", i)
		sc, err := resolve(path, m.Scenario, byRef, byName)
		if err != nil {
			return nil, err
		}
		for _, link := range []struct{ field, pool, role string }{
			{"client_pool", m.ClientPool, "client"}, {"server_pool", m.ServerPool, "server"},
		} {
			if link.pool == "" {
				continue
			}
			pi, ok := pools[link.pool]
			if !ok {
				return nil, invalid(path+"."+link.field, "unknown host pool %q", link.pool)
			}
			if role := cfg.Network.HostPools[pi].Role; role != link.role {
				return nil, invalid(path+"."+link.field, "host pool %s has role %s", link.pool, role)
			}
		}
		ref := sc.Module.Name + "/" + sc.Name
		if prev, ok := modules[sc.Module.Name]; ok && prev != sc.Module {
			return nil, fmt.Errorf("%w: module %s appears with two versions or digests", ErrInvalidCatalog, sc.Module.Name)
		}
		modules[sc.Module.Name] = sc.Module
		sum := sha256.Sum256([]byte(sc.Source))
		used[ref] = ScenarioSource{Ref: ref, Module: sc.Module.Name, Source: sc.Source, Digest: "sha256:" + hex.EncodeToString(sum[:])}
		estimates[i] = sc.BytesPerSession

		id := sc.Name
		for n := 2; ids[id]; n++ {
			id = fmt.Sprintf("%s_%d", sc.Name, n)
		}
		ids[id] = true
		c.mix = append(c.mix, MixEntry{
			ID:         id,
			Scenario:   ref,
			Weight:     m.Weight / total,
			Params:     m.Params,
			ClientPool: m.ClientPool,
			ServerPool: m.ServerPool,
		})
	}
	for _, name := range sortedKeys(modules) {
		c.modules = append(c.modules, modules[name])
	}
	for _, ref := range sortedKeys(used) {
		c.scenarios = append(c.scenarios, used[ref])
	}
	return estimates, nil
}

func indexCatalog(catalog []Scenario) (map[string]Scenario, map[string][]Scenario, error) {
	byRef := make(map[string]Scenario, len(catalog))
	byName := make(map[string][]Scenario, len(catalog))
	for _, sc := range catalog {
		ref := sc.Module.Name + "/" + sc.Name
		if !scenarioPattern.MatchString(ref) || !strings.Contains(ref, "/") {
			return nil, nil, fmt.Errorf("%w: invalid scenario ref %q", ErrInvalidCatalog, ref)
		}
		if _, dup := byRef[ref]; dup {
			return nil, nil, fmt.Errorf("%w: duplicate scenario %s", ErrInvalidCatalog, ref)
		}
		byRef[ref] = sc
		byName[sc.Name] = append(byName[sc.Name], sc)
	}
	return byRef, byName, nil
}

func resolve(path, ref string, byRef map[string]Scenario, byName map[string][]Scenario) (Scenario, error) {
	if !scenarioPattern.MatchString(ref) {
		return Scenario{}, invalid(path+".scenario", "invalid scenario reference %q", ref)
	}
	if strings.Contains(ref, "/") {
		sc, ok := byRef[ref]
		if !ok {
			return Scenario{}, fmt.Errorf("%s.scenario: %w %s", path, ErrUnknownScenario, ref)
		}
		return sc, nil
	}
	switch found := byName[ref]; len(found) {
	case 0:
		return Scenario{}, fmt.Errorf("%s.scenario: %w %s", path, ErrUnknownScenario, ref)
	case 1:
		return found[0], nil
	default:
		mods := make([]string, len(found))
		for i, sc := range found {
			mods[i] = sc.Module.Name
		}
		sort.Strings(mods)
		return Scenario{}, fmt.Errorf("%s.scenario: %w %s in modules %s", path, ErrAmbiguousScenario, ref, strings.Join(mods, ", "))
	}
}

func sortedKeys[V any](m map[string]V) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}

// loadAndCriteria checks the objective, derives the phase fractions, and
// converts the warm up period and the criteria.
func (c *compiled) loadAndCriteria(cfg Config) error {
	obj := cfg.Load.Objective
	if !objectives[obj.Type] {
		return invalid("load.objective.type", "unknown objective %q", obj.Type)
	}
	if math.IsNaN(obj.Value) || math.IsInf(obj.Value, 0) || obj.Value <= 0 {
		return invalid("load.objective.value", "value %v is not positive", obj.Value)
	}
	if len(cfg.Load.Phases) == 0 {
		return invalid("load.phases", "a test needs at least one load phase")
	}
	names := map[string]bool{}
	fraction := 0.0
	for i, ph := range cfg.Load.Phases {
		path := fmt.Sprintf("load.phases[%d]", i)
		if !namePattern.MatchString(ph.Name) || names[ph.Name] {
			return invalid(path, "invalid or duplicate name %q", ph.Name)
		}
		names[ph.Name] = true
		if !(ph.DurationS > 0) {
			return invalid(path, "duration_s %v is not positive", ph.DurationS)
		}
		d, err := durationFromSeconds(ph.DurationS)
		if err != nil || d == "" {
			return invalid(path, "duration_s %v is out of range", ph.DurationS)
		}
		phase := LoadPhase{Name: ph.Name, Duration: d}
		switch ph.Shape {
		case "", "hold":
			// A hold phase runs at the steady target.
			phase.Shape, phase.StartFraction, phase.EndFraction = "PHASE_SHAPE_HOLD", 1, 1
		case "linear", "step":
			// A ramp moves toward the steady target from below and back to
			// zero from the target.
			phase.Shape = enumName("PHASE_SHAPE_", ph.Shape)
			phase.StartFraction, phase.EndFraction = fraction, 1
			if fraction >= 1 {
				phase.EndFraction = 0
			}
			if ph.Shape == "step" {
				phase.Steps = uint32(math.Min(math.Max(1, math.Ceil(ph.DurationS)), math.MaxUint32))
			}
		default:
			return invalid(path, "unknown shape %q", ph.Shape)
		}
		fraction = phase.EndFraction
		c.phases = append(c.phases, phase)
	}
	warmup, err := durationFromSeconds(cfg.Timeline.WarmupS)
	if err != nil {
		return invalid("timeline.warmup_s", "%v", err)
	}
	c.warmup = warmup
	for i, cr := range cfg.Criteria {
		path := fmt.Sprintf("criteria[%d]", i)
		op, ok := criterionOps[cr.Op]
		switch {
		case !metricPattern.MatchString(cr.Metric):
			return invalid(path, "invalid metric %q", cr.Metric)
		case !ok:
			return invalid(path, "unknown op %q", cr.Op)
		case math.IsNaN(cr.Value) || math.IsInf(cr.Value, 0):
			return invalid(path, "value is not finite")
		}
		c.criteria = append(c.criteria, Criterion{Metric: cr.Metric, Op: op, Value: cr.Value})
	}
	return nil
}
