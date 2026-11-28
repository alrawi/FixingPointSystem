// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"strings"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
)

// fallbackRun is a run of three agents. agent-a fell back to the kernel
// backend though its inventory listed AF_XDP, agent-b ran AF_XDP as listed,
// and agent-c reported no backend.
func fallbackRun() runs.Run {
	return runs.Run{
		ID: "run-xdp", ConfigID: "cfg-1", State: runs.Completed,
		Agents: []string{"agent-a", "agent-b", "agent-c"},
		AgentRuns: []runs.AgentRun{
			{Agent: "agent-a", Backend: "kernel", SelectedBackend: "af_xdp", Ports: []runs.AgentRunPort{{Name: "client", InterfaceName: "eth1"}}},
			{Agent: "agent-b", Backend: "af_xdp", SelectedBackend: "af_xdp", Ports: []runs.AgentRunPort{{Name: "client", InterfaceName: "eth1", XdpMode: "native"}}},
		},
	}
}

func TestReportJSONShowsBackendAndMismatch(t *testing.T) {
	rep := Report{Run: fallbackRun()}.API()
	if rep.Run.Agents == nil || len(*rep.Run.Agents) != 2 {
		t.Fatalf("agents of the report: %+v", rep.Run.Agents)
	}
	a := (*rep.Run.Agents)[0]
	if a.Agent != "agent-a" || a.Backend != api.AgentRunBackendKernel || a.SelectedBackend == nil || *a.SelectedBackend != api.AgentRunSelectedBackendAfXdp {
		t.Fatalf("agent-a in the report: %+v", a)
	}
	b := (*rep.Run.Agents)[1]
	if b.Ports == nil || (*b.Ports)[0].XdpMode == nil || *(*b.Ports)[0].XdpMode != api.AgentRunPortsXdpModeNative {
		t.Fatalf("agent-b in the report: %+v", b)
	}
}

func TestReportCSVShowsBackendAndMismatch(t *testing.T) {
	out, err := Report{Run: fallbackRun()}.CSV()
	if err != nil {
		t.Fatal(err)
	}
	csv := string(out)
	for _, want := range []string{
		"run,backend.agent-a,,,kernel,,\n",
		"run,selected_backend.agent-a,,,af_xdp,,\n",
		"run,backend_mismatch.agent-a,,,true,,\n",
		"run,backend.agent-b,,,af_xdp,,\n",
		"run,backend_mismatch.agent-b,,,false,,\n",
		"run,xdp_mode.agent-b.client,,,native,,\n",
		"run,backend.agent-c,,,unknown,,\n",
	} {
		if !strings.Contains(csv, want) {
			t.Errorf("csv lacks %q:\n%s", want, csv)
		}
	}
	if strings.Contains(csv, "xdp_mode.agent-a") || strings.Contains(csv, "backend_mismatch.agent-c") {
		t.Errorf("csv holds a mode for the kernel run or a mismatch for the unknown agent:\n%s", csv)
	}
}

func TestReportCSVWithoutReportedBackendHasNoBackendRows(t *testing.T) {
	run := fallbackRun()
	run.AgentRuns = nil
	out, err := Report{Run: run}.CSV()
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(out), "backend") {
		t.Fatalf("csv names a backend that no agent reported:\n%s", out)
	}
}
