// SPDX-License-Identifier: GPL-3.0-only

package multiagent_test

import (
	"context"
	"errors"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/fakeagent"
	"github.com/alrawi/FixingPointSystem/controller/internal/multiagent"
)

// skewedFakes starts one fake agent per skew on the harness. The agents read
// the same fixed clock as the manager, so the measured offset is exactly the
// skew and not the skew minus the heartbeat delay.
func (h *harness) skewedFakes(t *testing.T, skews map[string]time.Duration) []string {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	t.Cleanup(cancel)
	var ids []string
	for id, skew := range skews {
		a, err := fakeagent.Start(ctx, fakeagent.Options{ID: id, Dial: h.dial, Clock: fixedClock{epoch}, ClockSkew: skew})
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(a.Close)
		ids = append(ids, id)
	}
	waitFor(t, "every fake agent to be online", func() bool {
		for _, id := range ids {
			if a, err := h.mgr.Get(id); err != nil || a.State != agents.StateOnline || !a.HasClockOffset {
				return false
			}
		}
		return true
	})
	return ids
}

// TestHeartbeatClocksRefuseASkewOf15Milliseconds is the acceptance item of
// CTL-51: a fake agent with a skew of 15 ms makes a three agent run refuse to
// start through the real heartbeat path.
func TestHeartbeatClocksRefuseASkewOf15Milliseconds(t *testing.T) {
	ms := time.Millisecond
	h := newHarness(t)
	h.skewedFakes(t, map[string]time.Duration{"agent-a": 0, "agent-b": 15 * ms, "agent-c": -3 * ms})
	coord := h.coordinator(t, multiagent.HeartbeatClocks{Agents: h.mgr})

	run, err := coord.Start(context.Background(), request(t, "agent-a", "agent-b", "agent-c"))
	if run != nil {
		t.Fatal("Start returned a run")
	}
	var ce *multiagent.ClockError
	if !errors.Is(err, multiagent.ErrClockOffset) || !errors.As(err, &ce) {
		t.Fatalf("error %v, want a ClockError", err)
	}
	if len(ce.Offenders) != 1 || ce.Offenders[0].AgentID != "agent-b" || ce.Offenders[0].Offset != 15*ms {
		t.Fatalf("offenders %+v, want agent-b at 15ms", ce.Offenders)
	}
	for _, id := range []string{"agent-a", "agent-b", "agent-c"} {
		if a, err := h.mgr.Get(id); err != nil || a.ReservedBy != "" {
			t.Fatalf("agent %s after the refusal: %+v, %v", id, a, err)
		}
	}
}

// TestHeartbeatClocksStartWithASkewOf2Milliseconds is the other half of the
// acceptance item.
func TestHeartbeatClocksStartWithASkewOf2Milliseconds(t *testing.T) {
	ms := time.Millisecond
	h := newHarness(t)
	h.skewedFakes(t, map[string]time.Duration{"agent-a": 0, "agent-b": 2 * ms, "agent-c": -2 * ms})
	run, err := h.coordinator(t, multiagent.HeartbeatClocks{Agents: h.mgr}).Start(context.Background(), request(t, "agent-a", "agent-b", "agent-c"))
	if err != nil {
		t.Fatalf("Start: %v", err)
	}
	h.run.Store(run)
	if got := len(run.Plans()); got != 3 {
		t.Fatalf("%d plans, want 3", got)
	}
	if _, err := run.Stop(context.Background(), agentpb.StopMode_STOP_MODE_ABORT); err != nil {
		t.Fatalf("Stop: %v", err)
	}
}

func TestHeartbeatClocksMeasureTheSkewAndFailClosed(t *testing.T) {
	h := newHarness(t)
	ids := h.skewedFakes(t, map[string]time.Duration{"agent-a": 7 * time.Millisecond})
	src := multiagent.HeartbeatClocks{Agents: h.mgr}
	ctx := context.Background()

	if d, err := src.ClockOffset(ctx, ids[0]); err != nil || d != 7*time.Millisecond {
		t.Fatalf("offset %s, %v, want 7ms", d, err)
	}
	if _, err := src.ClockOffset(ctx, "nobody"); !errors.Is(err, multiagent.ErrNoClockOffset) {
		t.Fatalf("unknown agent: %v, want ErrNoClockOffset", err)
	}
	h.mgr.Shutdown()
	if _, err := src.ClockOffset(ctx, ids[0]); !errors.Is(err, multiagent.ErrNoClockOffset) {
		t.Fatalf("offline agent: %v, want ErrNoClockOffset", err)
	}
}
