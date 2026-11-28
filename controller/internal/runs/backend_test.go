// SPDX-License-Identifier: GPL-3.0-only

package runs_test

import (
	"net/http"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// xdpCapabilities is an agent that lists AF_XDP as its selected backend and
// the kernel backend as a fallback.
func xdpCapabilities() *agentpb.Capabilities {
	c := fakeagent.DefaultCapabilities()
	c.Backends = []agentpb.Backend{agentpb.Backend_BACKEND_KERNEL, agentpb.Backend_BACKEND_AF_XDP}
	c.SelectedBackend = agentpb.Backend_BACKEND_AF_XDP
	return c
}

// backendAgent starts a fake agent that reports the run backend and ports.
func (h *harness) backendAgent(t *testing.T, id string, caps *agentpb.Capabilities, backend agentpb.Backend, ports ...*agentpb.RunPort) *fakeagent.Agent {
	t.Helper()
	a, err := fakeagent.Start(testContext(t), fakeagent.Options{ID: id, Dial: h.dial, Capabilities: caps, RunBackend: backend, RunPorts: ports})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(a.Close)
	return a
}

// reported waits until the controller holds the backend of the agents of a run.
func (h *harness) reported(t *testing.T, id string, n int) runs.Run {
	t.Helper()
	var run runs.Run
	waitFor(t, "the reported backends", func() bool {
		var err error
		run, err = h.orch.Get(id)
		return err == nil && len(run.AgentRuns) == n
	})
	return run
}

// TestRunOnKernelFallbackShowsKernelAndMismatch is the acceptance test of
// CTL-55: an agent whose inventory lists AF_XDP ran the plan on the kernel
// backend, and the run, the REST body, and the report show both.
func TestRunOnKernelFallbackShowsKernelAndMismatch(t *testing.T) {
	h := newHarness(t, nil)
	a := h.backendAgent(t, "agent-a", xdpCapabilities(), agentpb.Backend_BACKEND_KERNEL,
		&agentpb.RunPort{Name: "client", InterfaceName: "eth1"})
	run := h.start(t, a)
	got := h.reported(t, run.ID, 1)

	ar := got.AgentRuns[0]
	if ar.Agent != "agent-a" || ar.Backend != "kernel" || ar.SelectedBackend != "af_xdp" || !ar.Mismatch() {
		t.Fatalf("agent run %+v, want kernel against selected af_xdp and a mismatch", ar)
	}
	if len(ar.Ports) != 1 || ar.Ports[0].Name != "client" || ar.Ports[0].InterfaceName != "eth1" || ar.Ports[0].XdpMode != "" {
		t.Fatalf("ports %+v, want client on eth1 without an XDP mode", ar.Ports)
	}

	rh := newRESTHarness(t, h.orch)
	rec := rh.call(t, http.MethodGet, "/runs/{id}", map[string]string{"id": run.ID}, "", true)
	body := decodeRun(t, rec)
	if body.Agents == nil || len(*body.Agents) != 1 {
		t.Fatalf("agents of the body: %s", rec.Body)
	}
	x := (*body.Agents)[0]
	if x.Agent != "agent-a" || x.Backend != api.AgentRunBackendKernel || x.SelectedBackend == nil || *x.SelectedBackend != api.AgentRunSelectedBackendAfXdp {
		t.Fatalf("agent of the body %s", mustJSON(t, x))
	}
	if x.Ports == nil || len(*x.Ports) != 1 || (*x.Ports)[0].XdpMode != nil {
		t.Fatalf("ports of the body %s", mustJSON(t, x))
	}
}

// TestRunOnAFXDPShowsTheBoundMode checks a run that ran the backend that the
// inventory listed: no mismatch, and the bound mode of each port.
func TestRunOnAFXDPShowsTheBoundMode(t *testing.T) {
	h := newHarness(t, nil)
	a := h.backendAgent(t, "agent-a", xdpCapabilities(), agentpb.Backend_BACKEND_AF_XDP,
		&agentpb.RunPort{Name: "client", InterfaceName: "eth1", XdpMode: agentpb.XdpMode_XDP_MODE_NATIVE})
	run := h.start(t, a)
	ar := h.reported(t, run.ID, 1).AgentRuns[0]
	if ar.Backend != "af_xdp" || ar.SelectedBackend != "af_xdp" || ar.Mismatch() {
		t.Fatalf("agent run %+v, want af_xdp without a mismatch", ar)
	}
	if len(ar.Ports) != 1 || ar.Ports[0].XdpMode != "native" {
		t.Fatalf("ports %+v, want the native mode", ar.Ports)
	}

	body := runs.ToAPI(h.reported(t, run.ID, 1))
	if body.Agents == nil || (*body.Agents)[0].Ports == nil || *(*(*body.Agents)[0].Ports)[0].XdpMode != api.AgentRunPortsXdpModeNative {
		t.Fatalf("body %s", mustJSON(t, body.Agents))
	}
}

// TestAgentThatReportsNoBackendIsUnknown checks an agent of agent protocol
// 0.2.6 or earlier: BACKEND_UNSPECIFIED is unknown and never the kernel
// backend, so the run lists no backend for it.
func TestAgentThatReportsNoBackendIsUnknown(t *testing.T) {
	h := newHarness(t, nil)
	a := h.agent(t, "agent-a", nil)
	run := h.start(t, a)
	if err := a.Heartbeat(testContext(t)); err != nil {
		t.Fatal(err)
	}
	got, err := h.orch.Get(run.ID)
	if err != nil {
		t.Fatal(err)
	}
	if len(got.AgentRuns) != 0 {
		t.Fatalf("agent runs %+v, want none for an agent that reported no backend", got.AgentRuns)
	}
	if body := runs.ToAPI(got); body.Agents != nil {
		t.Fatalf("agents of the body %s, want absent", mustJSON(t, body.Agents))
	}
}

// TestMismatchNeedsBothBackends checks that an unknown inventory backend is
// no mismatch.
func TestMismatchNeedsBothBackends(t *testing.T) {
	for _, tc := range []struct {
		name string
		run  runs.AgentRun
		want bool
	}{
		{"same", runs.AgentRun{Backend: "kernel", SelectedBackend: "kernel"}, false},
		{"differs", runs.AgentRun{Backend: "kernel", SelectedBackend: "af_xdp"}, true},
		{"inventory unknown", runs.AgentRun{Backend: "kernel"}, false},
		{"run unknown", runs.AgentRun{SelectedBackend: "af_xdp"}, false},
	} {
		if got := tc.run.Mismatch(); got != tc.want {
			t.Errorf("%s: Mismatch() = %v, want %v", tc.name, got, tc.want)
		}
	}
}
