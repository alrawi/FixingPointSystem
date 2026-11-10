// SPDX-License-Identifier: GPL-3.0-only

package api

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
)

// EngineAPIVersion is the version of the contracts in contracts/ that the
// controller drives, which GET /v1/capabilities reports as engine_api.
const EngineAPIVersion = "0.2.0"

// APIVersion is the version of the REST contract, from info.version of
// contracts/openapi/fps-api-v1.yaml, which GET /v1/capabilities reports as
// api_version.
const APIVersion = "0.9.0"

// ModuleLister lists the installed modules for GET /v1/capabilities. The
// controller binary adapts the module registry to it.
type ModuleLister interface {
	// ModuleSummaries returns every installed module.
	ModuleSummaries(ctx context.Context) ([]ModuleSummary, error)
}

// AgentInventory lists the known agents for GET /v1/capabilities. The
// controller binary adapts the agent manager to it.
type AgentInventory interface {
	// AgentCapabilities returns every known agent with the backends that its
	// latest registration reports.
	AgentCapabilities(ctx context.Context) ([]AgentCapability, error)
}

// AgentCapability is one agent of the inventory. The Agent schema names a
// single backend, so Backends carries every backend that the agent supports.
type AgentCapability struct {
	// Agent is the agent as GET /v1/agents reports it.
	Agent Agent
	// Backends lists the backends that the agent supports.
	Backends []CapabilitiesBackends
}

// backendOrder is the order of the backends in a capabilities response.
var backendOrder = []CapabilitiesBackends{
	CapabilitiesBackendsKernel,
	CapabilitiesBackendsAfXdp,
	CapabilitiesBackendsDpdk,
}

// CapabilitiesAPI serves GET /v1/capabilities and answers every other
// operation with the 501 problem of Unimplemented.
type CapabilitiesAPI struct {
	Unimplemented
	modules ModuleLister
	agents  AgentInventory
	version string
}

// CapabilitiesOption configures a CapabilitiesAPI.
type CapabilitiesOption func(*CapabilitiesAPI)

// WithControllerVersion sets the version of the controller build, which
// GET /v1/capabilities reports as controller_version. An empty version leaves
// the field out, as the contract says for a build without one.
func WithControllerVersion(version string) CapabilitiesOption {
	return func(c *CapabilitiesAPI) { c.version = version }
}

var _ ServerInterface = (*CapabilitiesAPI)(nil)

// NewCapabilitiesAPI returns the capabilities operation over modules and
// agents. A nil source reports an empty list.
func NewCapabilitiesAPI(modules ModuleLister, agents AgentInventory, opts ...CapabilitiesOption) *CapabilitiesAPI {
	c := &CapabilitiesAPI{modules: modules, agents: agents}
	for _, opt := range opts {
		opt(c)
	}
	return c
}

// Capabilities returns the capabilities document. Agents and modules keep the
// order of their sources. Backends is the union of the valid backends of all
// agents, in the order kernel, af_xdp, dpdk.
func (c *CapabilitiesAPI) Capabilities(ctx context.Context) (Capabilities, error) {
	out := Capabilities{
		EngineApi:  EngineAPIVersion,
		ApiVersion: ptr(APIVersion),
		Backends:   []CapabilitiesBackends{},
		Agents:     []Agent{},
		Modules:    []ModuleSummary{},
	}
	if c.version != "" {
		out.ControllerVersion = ptr(c.version)
	}
	if c.agents != nil {
		list, err := c.agents.AgentCapabilities(ctx)
		if err != nil {
			return Capabilities{}, fmt.Errorf("get capabilities: list agents: %w", err)
		}
		seen := make(map[CapabilitiesBackends]bool, len(backendOrder))
		for _, a := range list {
			out.Agents = append(out.Agents, a.Agent)
			for _, b := range a.Backends {
				seen[b] = true
			}
		}
		for _, b := range backendOrder {
			if seen[b] {
				out.Backends = append(out.Backends, b)
			}
		}
	}
	if c.modules != nil {
		list, err := c.modules.ModuleSummaries(ctx)
		if err != nil {
			return Capabilities{}, fmt.Errorf("get capabilities: list modules: %w", err)
		}
		out.Modules = append(out.Modules, list...)
	}
	return out, nil
}

// GetCapabilities answers GET /v1/capabilities. A failing source gives a 500
// problem that names it.
func (c *CapabilitiesAPI) GetCapabilities(w http.ResponseWriter, r *http.Request) {
	caps, err := c.Capabilities(r.Context())
	if err != nil {
		WriteProblem(w, r, NewProblem(http.StatusInternalServerError, err.Error()))
		return
	}
	body, err := json.Marshal(caps)
	if err != nil {
		WriteProblem(w, r, Problemf(http.StatusInternalServerError, "encode capabilities: %v", err))
		return
	}
	h := w.Header()
	h.Set("Content-Type", "application/json")
	h.Set("Cache-Control", "no-store")
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write(append(body, '\n'))
}
