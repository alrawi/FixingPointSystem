// SPDX-License-Identifier: GPL-3.0-only

package metrics

import (
	"math"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
)

// TestClientSideIsTheDefault checks that every view counts the client side
// only unless the request selects the server side, so that a session that
// both sides report counts once (CON-08). The catalog gives the metrics of
// scenarios, ports, and transports the side label.
func TestClientSideIsTheDefault(t *testing.T) {
	a := newAgg(t, nil)
	web := func(side string, extra ...string) []string {
		return append(append(append([]string{}, webGet...), "side", side), extra...)
	}
	port := func(side string) []string { return []string{"port", "p1", "side", side} }
	a.Add("agent-a", t0, snapshot("r/agent-a", 1, []*agentpb.CounterValue{
		counter("sessions.attempted", 100, web("client")...),
		counter("sessions.attempted", 100, web("server")...),
		counter("sessions.failed", 4, web("client", "reason", "aborted")...),
		counter("sessions.failed", 2, web("server", "reason", "aborted")...),
		counter("tcp.handshakes", 100, port("client")...),
		counter("tcp.handshakes", 90, port("server")...),
	}))
	client := map[string]float64{"sessions.attempted": 100, "sessions.failed": 4, "sessions.failed_ratio": 0.04}
	for _, c := range []struct {
		side       string
		want       map[string]float64
		handshakes float64
	}{
		{"", client, 100},
		{SideClient, client, 100},
		{SideServer, map[string]float64{"sessions.attempted": 100, "sessions.failed": 2, "sessions.failed_ratio": 0.02}, 90},
	} {
		for _, v := range []struct {
			view api.MetricView
			key  string
		}{{"application", "http1"}, {"scenario", "http1/web_get"}} {
			view := v.view
			samples, err := a.Samples("r", view, Window{Side: c.side})
			if err != nil {
				t.Fatal(err)
			}
			for _, key := range []string{RunKey, v.key} {
				s := sampleOf(t, samples, key)
				for k, want := range c.want {
					if math.Abs(s.Values[k]-want) > 1e-12 {
						t.Errorf("view %s, side %q, key %q: %s = %v, want %v", view, c.side, key, k, s.Values[k], want)
					}
				}
			}
		}
		tcp, err := a.Samples("r", "tcp", Window{Side: c.side})
		if err != nil {
			t.Fatal(err)
		}
		if got := sampleOf(t, tcp, "p1").Values["tcp.handshakes"]; got != c.handshakes {
			t.Errorf("side %q: tcp.handshakes = %v, want %v", c.side, got, c.handshakes)
		}
	}
	latest, err := a.Latest("r", "application")
	if err != nil {
		t.Fatal(err)
	}
	if got := sampleOf(t, latest, RunKey).Values["sessions.attempted"]; got != 100 {
		t.Errorf("latest application view counts %v sessions, want the client side 100", got)
	}
	srv, err := a.LatestSide("r", "application", SideServer)
	if err != nil {
		t.Fatal(err)
	}
	if got := sampleOf(t, srv, RunKey).Values["sessions.failed"]; got != 2 {
		t.Errorf("server side latest view counts %v failed sessions, want 2", got)
	}
	if !ValidSide("") || !ValidSide(SideServer) || ValidSide("both") {
		t.Error("ValidSide must accept the empty side, client, and server, and nothing else")
	}
}
