// SPDX-License-Identifier: GPL-3.0-only

package agents

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/http"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
)

// OperatorHolder is the reservation holder of POST /v1/agents/{id}:reserve.
const OperatorHolder = "operator"

// maxBody bounds the request body of POST /v1/agents.
const maxBody = 64 << 10

// API serves the /v1/agents operations from a Manager and answers every other
// operation with the 501 problem of api.Unimplemented. A later package
// embeds API, or api.Unimplemented, next to its own operations.
type API struct {
	api.Unimplemented
	m *Manager
}

var _ api.ServerInterface = (*API)(nil)

// NewAPI returns the REST operations of m.
func NewAPI(m *Manager) *API { return &API{m: m} }

// ListAgents answers GET /v1/agents with every agent, ordered by identity.
func (a *API) ListAgents(w http.ResponseWriter, _ *http.Request) {
	list := a.m.List()
	out := make([]api.Agent, 0, len(list))
	for _, ag := range list {
		out = append(out, ToAPI(ag))
	}
	writeJSON(w, http.StatusOK, out)
}

// RegisterAgent answers POST /v1/agents. It enrolls the identity with its
// labels, so the agent may register, and answers 201 with the agent.
func (a *API) RegisterAgent(w http.ResponseWriter, r *http.Request) {
	dec := json.NewDecoder(http.MaxBytesReader(w, r.Body, maxBody))
	dec.DisallowUnknownFields()
	var body api.RegisterAgentJSONRequestBody
	if err := dec.Decode(&body); err != nil {
		api.WriteProblem(w, r, api.Problemf(http.StatusBadRequest, "decode agent enrollment: %v", err))
		return
	}
	if dec.More() {
		api.WriteProblem(w, r, api.NewProblem(http.StatusBadRequest, "agent enrollment holds more than one JSON value"))
		return
	}
	var labels map[string]string
	if body.Labels != nil {
		labels = *body.Labels
	}
	ag, err := a.m.Enroll(body.Id, labels)
	if err != nil {
		writeError(w, r, err)
		return
	}
	writeJSON(w, http.StatusCreated, ToAPI(ag))
}

// ReserveAgent answers POST /v1/agents/{id}:reserve. The operator holds the
// reservation.
func (a *API) ReserveAgent(w http.ResponseWriter, r *http.Request, id api.Id) {
	ag, err := a.m.Reserve(id, OperatorHolder)
	if err != nil {
		writeError(w, r, err)
		return
	}
	writeJSON(w, http.StatusOK, ToAPI(ag))
}

// ReleaseAgent answers POST /v1/agents/{id}:release. It ends any reservation.
func (a *API) ReleaseAgent(w http.ResponseWriter, r *http.Request, id api.Id) {
	ag, err := a.m.Release(id, "")
	if err != nil {
		writeError(w, r, err)
		return
	}
	writeJSON(w, http.StatusOK, ToAPI(ag))
}

// writeError maps the errors of the manager to problems.
func writeError(w http.ResponseWriter, r *http.Request, err error) {
	status := http.StatusInternalServerError
	switch {
	case errors.Is(err, ErrUnknownAgent):
		status = http.StatusNotFound
	case errors.Is(err, ErrInvalidID):
		status = http.StatusUnprocessableEntity
	case errors.Is(err, ErrAgentOffline), errors.Is(err, ErrAgentReserved), errors.Is(err, ErrAgentBusy):
		status = http.StatusConflict
	}
	api.WriteProblem(w, r, api.NewProblem(status, err.Error()))
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	body, err := json.Marshal(v)
	if err != nil {
		api.WriteProblem(w, nil, api.Problemf(http.StatusInternalServerError, "encode response: %v", err))
		return
	}
	h := w.Header()
	h.Set("Content-Type", "application/json")
	h.Set("Cache-Control", "no-store")
	w.WriteHeader(status)
	_, _ = w.Write(append(body, '\n'))
}

// backends maps the proto backends to the REST names.
var backends = map[agentpb.Backend]api.AgentBackend{
	agentpb.Backend_BACKEND_KERNEL: api.AgentBackendKernel,
	agentpb.Backend_BACKEND_AF_XDP: api.AgentBackendAfXdp,
	agentpb.Backend_BACKEND_DPDK:   api.AgentBackendDpdk,
}

// xdpModes maps the proto XDP modes to the REST names. XDP_MODE_UNSPECIFIED
// has no entry, so the property stays absent.
var xdpModes = map[agentpb.XdpMode]api.AgentPortsXdpDriverMode{
	agentpb.XdpMode_XDP_MODE_GENERIC:   api.AgentPortsXdpDriverModeGeneric,
	agentpb.XdpMode_XDP_MODE_NATIVE:    api.AgentPortsXdpDriverModeNative,
	agentpb.XdpMode_XDP_MODE_ZERO_COPY: api.AgentPortsXdpDriverModeZeroCopy,
}

// ToAPI returns the Agent schema of the REST contract for ag. The backend is
// the backend that the agent selected. An agent that reports none (agent
// protocol 0.2.4 or earlier, or no backend that can run) gets the first
// backend that it lists. The kernel release and the XDP mode of a port are
// absent when the agent sent none. The XDP mode is a hint for a driver
// table, and nothing here selects or refuses on it.
func ToAPI(ag Agent) api.Agent {
	out := api.Agent{Id: ag.ID, State: ag.State}
	if ag.Address != "" {
		out.Address = ptr(ag.Address)
	}
	if len(ag.Labels) > 0 {
		out.Labels = ptr(ag.Labels)
	}
	if ag.ReservedBy != "" {
		out.ReservedBy = ptr(ag.ReservedBy)
	}
	c := ag.Capabilities
	if c == nil {
		return out
	}
	if v := c.GetAgentVersion(); v != "" {
		out.Version = ptr(v)
	}
	if n := c.GetCores(); n > 0 {
		out.Cores = ptr(int(n))
	}
	if name, ok := backends[c.GetSelectedBackend()]; ok {
		out.Backend = ptr(name)
	} else {
		for _, b := range c.GetBackends() {
			if name, ok := backends[b]; ok {
				out.Backend = ptr(name)
				break
			}
		}
	}
	if k := c.GetKernelRelease(); k != "" {
		out.KernelRelease = ptr(k)
	}
	if len(c.GetPorts()) > 0 {
		ports := make([]struct {
			Driver        *string                      `json:"driver,omitempty"`
			Name          string                       `json:"name"`
			SpeedMbps     *int                         `json:"speed_mbps,omitempty"`
			XdpDriverMode *api.AgentPortsXdpDriverMode `json:"xdp_driver_mode,omitempty"`
		}, len(c.GetPorts()))
		for i, p := range c.GetPorts() {
			ports[i].Name = p.GetName()
			if d := p.GetDriver(); d != "" {
				ports[i].Driver = ptr(d)
			}
			if s := p.GetSpeedMbps(); s > 0 {
				ports[i].SpeedMbps = ptr(int(s))
			}
			if m, ok := xdpModes[p.GetXdpDriverMode()]; ok {
				ports[i].XdpDriverMode = ptr(m)
			}
		}
		out.Ports = &ports
	}
	return out
}

func ptr[T any](v T) *T { return &v }

// String returns a short description of ag for logs and test failures.
func (ag Agent) String() string {
	return fmt.Sprintf("%s(%s, %s, reserved by %q)", ag.ID, ag.State, ag.AgentState, ag.ReservedBy)
}
