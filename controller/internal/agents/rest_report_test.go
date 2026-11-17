package agents_test

import (
	"encoding/json"
	"net/http"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
)

// TestRESTAgentCapabilitiesReport checks the kernel release, the selected
// backend, and the XDP mode of each port (CTL-53), and the fallback for an
// agent that reports none of them. Every response is validated against the
// contract.
func TestRESTAgentCapabilitiesReport(t *testing.T) {
	h := newHarness(t, nil)
	rh := newRESTHarness(t, h.mgr)

	// A new agent that selected AF_XDP, with a mode per port.
	afxdp := fakeagent.DefaultCapabilities()
	afxdp.Backends = []agentpb.Backend{agentpb.Backend_BACKEND_KERNEL, agentpb.Backend_BACKEND_AF_XDP}
	afxdp.SelectedBackend = agentpb.Backend_BACKEND_AF_XDP
	afxdp.KernelRelease = "6.8.0-45-generic"
	afxdp.Ports = []*agentpb.PortInfo{
		{Name: "eth1", Driver: "ixgbe", SpeedMbps: 10000, XdpDriverMode: agentpb.XdpMode_XDP_MODE_ZERO_COPY},
		{Name: "eth2", Driver: "i40e", SpeedMbps: 10000, XdpDriverMode: agentpb.XdpMode_XDP_MODE_NATIVE},
		{Name: "eth3", Driver: "veth", SpeedMbps: 10000, XdpDriverMode: agentpb.XdpMode_XDP_MODE_GENERIC},
		{Name: "eth4", Driver: "e1000", SpeedMbps: 1000},
	}

	// A new agent whose selected backend is the kernel, although it lists AF_XDP first.
	kernel := fakeagent.DefaultCapabilities()
	kernel.Backends = []agentpb.Backend{agentpb.Backend_BACKEND_AF_XDP, agentpb.Backend_BACKEND_KERNEL}
	kernel.SelectedBackend = agentpb.Backend_BACKEND_KERNEL
	kernel.KernelRelease = "6.1.0-18-amd64"

	// An old agent sends empty and unspecified values.
	old := fakeagent.DefaultCapabilities()
	old.Backends = []agentpb.Backend{agentpb.Backend_BACKEND_AF_XDP, agentpb.Backend_BACKEND_KERNEL}

	// A new agent that selected no backend because none can run.
	none := fakeagent.DefaultCapabilities()
	none.Backends = nil

	cases := []struct {
		id       string
		caps     *agentpb.Capabilities
		backend  string
		kernel   string
		portMode []string
	}{
		{"agent-afxdp", afxdp, "af_xdp", "6.8.0-45-generic", []string{"zero_copy", "native", "generic", ""}},
		{"agent-kernel", kernel, "kernel", "6.1.0-18-amd64", []string{""}},
		{"agent-old", old, "af_xdp", "", []string{""}},
		{"agent-none", none, "", "", []string{""}},
	}
	for _, c := range cases {
		rec := rh.call(t, http.MethodPost, "/agents", nil, `{"id":"`+c.id+`"}`)
		if rec.Code != http.StatusCreated {
			t.Fatalf("enroll %s: %d %s", c.id, rec.Code, rec.Body)
		}
		h.start(t, fakeagent.Options{ID: c.id, Capabilities: c.caps})
	}

	rec := rh.call(t, http.MethodGet, "/agents", nil, "")
	var list []map[string]any
	if err := json.Unmarshal(rec.Body.Bytes(), &list); err != nil {
		t.Fatalf("decode %s: %v", rec.Body, err)
	}
	byID := map[string]map[string]any{}
	for _, a := range list {
		id, ok := a["id"].(string)
		if !ok {
			t.Fatalf("agent without an id in %s", rec.Body)
		}
		byID[id] = a
	}

	// check wants the key missing when want is "", and the value want
	// otherwise. An absent value is a missing key, never an empty string.
	check := func(t *testing.T, m map[string]any, key, want string) {
		t.Helper()
		v, ok := m[key]
		if want == "" {
			if ok {
				t.Fatalf("%s = %v, want it absent", key, v)
			}
			return
		}
		if v != want {
			t.Fatalf("%s = %v (present %v), want %q", key, v, ok, want)
		}
	}
	for _, c := range cases {
		t.Run(c.id, func(t *testing.T) {
			got := byID[c.id]
			if got == nil {
				t.Fatalf("agent %s missing from %s", c.id, rec.Body)
			}
			check(t, got, "backend", c.backend)
			check(t, got, "kernel_release", c.kernel)
			ports, _ := got["ports"].([]any)
			if len(ports) != len(c.portMode) {
				t.Fatalf("ports %v, want %d", ports, len(c.portMode))
			}
			for i, want := range c.portMode {
				port, ok := ports[i].(map[string]any)
				if !ok {
					t.Fatalf("port %d is %T", i, ports[i])
				}
				check(t, port, "xdp_driver_mode", want)
			}
		})
	}
}
