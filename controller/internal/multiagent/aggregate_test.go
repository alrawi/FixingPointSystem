// SPDX-License-Identifier: GPL-3.0-only

package multiagent_test

import (
	"errors"
	"math"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/multiagent"
)

func count(metric string, n uint64, labels ...string) *agentpb.CounterValue {
	v := &agentpb.CounterValue{Metric: metric, Value: &agentpb.CounterValue_Count{Count: n}}
	if len(labels) > 0 {
		v.Labels = map[string]string{}
		for i := 0; i+1 < len(labels); i += 2 {
			v.Labels[labels[i]] = labels[i+1]
		}
	}
	return v
}

func gauge(metric string, g float64) *agentpb.CounterValue {
	return &agentpb.CounterValue{Metric: metric, Value: &agentpb.CounterValue_Gauge{Gauge: g}}
}

func snap(plan string, seq uint64, final bool, cs ...*agentpb.CounterValue) *agentpb.CounterSnapshot {
	return &agentpb.CounterSnapshot{PlanId: plan, Sequence: seq, Aggregate: true, IsFinal: final, Counters: cs}
}

func plans(m map[string]string) func(string) string { return func(id string) string { return m[id] } }

func TestCollectorSumsExactlyBeyondFloatPrecision(t *testing.T) {
	// 2^53+1 is the first integer that a float64 cannot hold. Three agents
	// whose sum needs the low bit must still add up exactly.
	const big = uint64(1<<53) + 1
	c := multiagent.NewCollector(nil)
	c.Add("a", snap("p", 1, true, count("sessions.attempted", big)))
	c.Add("b", snap("p", 1, true, count("sessions.attempted", big)))
	c.Add("c", snap("p", 1, true, count("sessions.attempted", big+2)))
	got, err := c.Totals()
	if err != nil {
		t.Fatal(err)
	}
	if want := 3*big + 2; got["sessions.attempted"] != want {
		t.Fatalf("sum %d, want %d", got["sessions.attempted"], want)
	}
}

func TestCollectorNewestSnapshotReplacesAndKeepsOmittedCounters(t *testing.T) {
	c := multiagent.NewCollector(plans(map[string]string{"a": "p"}))
	c.Add("a", snap("p", 1, false, count("m.x", 10), count("m.y", 5)))
	c.Add("a", snap("p", 2, false, count("m.x", 25)))
	got, err := c.Totals()
	if err != nil {
		t.Fatal(err)
	}
	if got["m.x"] != 25 || got["m.y"] != 5 {
		t.Fatalf("totals %v, want m.x 25 and m.y 5", got)
	}
}

func TestCollectorDropsDuplicatesAndOldSnapshots(t *testing.T) {
	c := multiagent.NewCollector(nil)
	if !c.Add("a", snap("p", 3, false, count("m.x", 30))) {
		t.Fatal("first snapshot dropped")
	}
	if c.Add("a", snap("p", 3, false, count("m.x", 999))) {
		t.Fatal("a repeated sequence was taken")
	}
	if c.Add("a", snap("p", 2, false, count("m.x", 999))) {
		t.Fatal("an older sequence was taken")
	}
	if c.Add("a", nil) {
		t.Fatal("a nil snapshot was taken")
	}
	got, _ := c.Totals()
	if got["m.x"] != 30 {
		t.Fatalf("m.x is %d, want 30", got["m.x"])
	}
}

func TestCollectorIgnoresOtherPlans(t *testing.T) {
	c := multiagent.NewCollector(plans(map[string]string{"a": "run1/a"}))
	if c.Add("a", snap("run0/a", 1, false, count("m.x", 7))) {
		t.Fatal("a snapshot of an earlier plan was taken")
	}
	if !c.Add("a", snap("run1/a", 1, false, count("m.x", 9))) {
		t.Fatal("a snapshot of the plan was dropped")
	}
	got, _ := c.Totals()
	if got["m.x"] != 9 {
		t.Fatalf("m.x is %d, want 9", got["m.x"])
	}
}

func TestCollectorLabelsStaySeparate(t *testing.T) {
	c := multiagent.NewCollector(nil)
	c.Add("a", snap("p", 1, false, count("m.x", 1), count("m.x", 10, "scenario", "web_get")))
	c.Add("b", snap("p", 1, false, count("m.x", 2), count("m.x", 20, "scenario", "web_get"), count("m.x", 30, "scenario", "lookup")))
	series, err := c.Series()
	if err != nil {
		t.Fatal(err)
	}
	byKey := map[string]uint64{}
	for _, s := range series {
		byKey[s.Labels["scenario"]] = s.Count
	}
	if len(series) != 3 || byKey[""] != 3 || byKey["web_get"] != 30 || byKey["lookup"] != 30 {
		t.Fatalf("series %+v", series)
	}
	totals, _ := c.Totals()
	if len(totals) != 1 || totals["m.x"] != 3 {
		t.Fatalf("totals %v keep only the unlabeled counter", totals)
	}
}

func TestCollectorPerWorkerStreamsAddUp(t *testing.T) {
	c := multiagent.NewCollector(nil)
	w := func(worker uint32, seq uint64, n uint64) *agentpb.CounterSnapshot {
		return &agentpb.CounterSnapshot{PlanId: "p", Sequence: seq, WorkerId: worker, Counters: []*agentpb.CounterValue{count("m.x", n)}}
	}
	// The sequence runs over all snapshots of the agent, so the workers
	// interleave. Worker 0 is the first worker and sends aggregate false.
	c.Add("a", w(1, 1, 10))
	c.Add("a", w(2, 2, 20))
	c.Add("a", w(1, 3, 15))
	c.Add("a", w(2, 4, 25))
	got, _ := c.Totals()
	if got["m.x"] != 40 {
		t.Fatalf("m.x is %d, want 15 + 25", got["m.x"])
	}
}

func TestCollectorGaugesSumAsFloats(t *testing.T) {
	c := multiagent.NewCollector(nil)
	c.Add("a", snap("p", 1, false, gauge("sessions.active", 1.5)))
	c.Add("b", snap("p", 1, false, gauge("sessions.active", 2.25)))
	series, err := c.Series()
	if err != nil {
		t.Fatal(err)
	}
	if len(series) != 1 || !series[0].Gauge || series[0].Value != 3.75 {
		t.Fatalf("series %+v", series)
	}
	if totals, _ := c.Totals(); len(totals) != 0 {
		t.Fatalf("a gauge is not a total: %v", totals)
	}
}

func TestCollectorOverflowIsAnError(t *testing.T) {
	c := multiagent.NewCollector(nil)
	c.Add("a", snap("p", 1, false, count("m.x", math.MaxUint64)))
	c.Add("b", snap("p", 1, false, count("m.x", 1)))
	if _, err := c.Totals(); !errors.Is(err, multiagent.ErrCounterOverflow) {
		t.Fatalf("error %v, want ErrCounterOverflow", err)
	}
}

func TestCollectorKindMismatchIsAnError(t *testing.T) {
	c := multiagent.NewCollector(nil)
	c.Add("a", snap("p", 1, false, count("m.x", 1)))
	c.Add("b", snap("p", 1, false, gauge("m.x", 1)))
	if _, err := c.Series(); err == nil {
		t.Fatal("a counter and a gauge of one metric were summed")
	}
}

func TestCollectorFinal(t *testing.T) {
	c := multiagent.NewCollector(nil)
	c.Add("a", snap("p", 1, false))
	if c.Final("a") {
		t.Fatal("final before the final snapshot")
	}
	c.Add("a", snap("p", 2, true))
	if !c.Final("a") || c.Final("b") {
		t.Fatal("Final does not follow the final snapshots")
	}
}
