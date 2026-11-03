// SPDX-License-Identifier: GPL-3.0-only

package metrics

import (
	"errors"
	"math"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/api"
)

func sampleOf(t *testing.T, samples []Sample, key string) Sample {
	t.Helper()
	for _, s := range samples {
		if s.Key == key {
			return s
		}
	}
	t.Fatalf("no sample with key %q in %v", key, samples)
	return Sample{}
}

func TestRunOfPlan(t *testing.T) {
	cases := []struct{ agent, plan, want string }{
		{"agent-a", "run-1/agent-a", "run-1"},
		{"agent-a", "run-1/agent-b", "run-1"},
		{"agent-a", "solo", "solo"},
		{"agent-a", "", ""},
		{"agent-a", "/agent-a", ""},
	}
	for _, c := range cases {
		if got := RunOfPlan(c.agent, c.plan); got != c.want {
			t.Errorf("RunOfPlan(%q, %q) = %q, want %q", c.agent, c.plan, got, c.want)
		}
	}
}

func TestPerWorkerReplacesAggregate(t *testing.T) {
	a := newAgg(t, nil)
	a.Add("agent-a", t0, snapshot("r/agent-a", 1, []*agentpb.CounterValue{counter("sessions.attempted", 100, webGet...)}))
	w := func(id uint32, seq, n uint64) *agentpb.CounterSnapshot {
		return &agentpb.CounterSnapshot{PlanId: "r/agent-a", Sequence: seq, WorkerId: id, Counters: []*agentpb.CounterValue{counter("sessions.attempted", n, webGet...)}}
	}
	a.Add("agent-a", t0, w(0, 2, 30))
	a.Add("agent-a", t0, w(1, 3, 40))
	tot, err := a.Totals("r")
	if err != nil {
		t.Fatal(err)
	}
	if tot["sessions.attempted"] != 70 {
		t.Fatalf("sessions.attempted = %v, want the per worker sum 70", tot["sessions.attempted"])
	}
}

func TestGaugesAndDerivedMetrics(t *testing.T) {
	a := newAgg(t, nil)
	tlsPort := []string{"port", "p1"}
	a.Add("agent-a", t0, snapshot("r/agent-a", 1, []*agentpb.CounterValue{
		counter("sessions.attempted", 100, webGet...),
		counter("sessions.failed", 5, append(webGet, "reason", "expect_timeout")...),
		counter("sessions.failed", 5, append(webGet, "reason", "aborted")...),
		gauge("sessions.concurrent", 10, webGet...),
		gauge("sessions.failed_ratio", 0.9, webGet...), // derived, so ignored
		gauge("realism.conformance_ratio", 0.8, "application", "http1", "classifier", "ndpi"),
		counter("tls.handshakes", 10, tlsPort...),
		counter("tls.resumptions", 4, tlsPort...),
	}))
	a.Add("agent-b", t0, snapshot("r/agent-b", 1, []*agentpb.CounterValue{
		counter("sessions.attempted", 300, webGet...),
		counter("sessions.failed", 10, append(webGet, "reason", "aborted")...),
		gauge("sessions.concurrent", 20, webGet...),
		gauge("realism.conformance_ratio", 0.6, "application", "http1", "classifier", "ndpi"),
		counter("tls.handshakes", 30, tlsPort...),
	}))
	tot, err := a.Totals("r")
	if err != nil {
		t.Fatal(err)
	}
	want := map[string]float64{
		"sessions.attempted":        400,
		"sessions.failed":           20,
		"sessions.failed_ratio":     0.05,
		"sessions.concurrent":       30,
		"realism.conformance_ratio": 0.7,
		"tls.resumption_ratio":      0.1,
	}
	for k, v := range want {
		if math.Abs(tot[k]-v) > 1e-12 {
			t.Errorf("%s = %v, want %v", k, tot[k], v)
		}
	}
	// The scenario view derives the ratio per key from the summed counters.
	sc, err := a.Latest("r", "scenario")
	if err != nil {
		t.Fatal(err)
	}
	if got := sampleOf(t, sc, "http1/web_get").Values["sessions.failed_ratio"]; math.Abs(got-0.05) > 1e-12 {
		t.Errorf("scenario failed ratio = %v", got)
	}
	tls, _ := a.Latest("r", "tls")
	if got := sampleOf(t, tls, "p1").Values["tls.resumption_ratio"]; math.Abs(got-0.1) > 1e-12 {
		t.Errorf("tls resumption ratio = %v", got)
	}
	// The Prometheus series derive the ratio for each label set of the gauge.
	series, _ := a.Series("r")
	for _, s := range series {
		if s.Metric.Name == "sessions.failed_ratio" && math.Abs(s.Gauge-0.05) > 1e-12 {
			t.Errorf("series failed ratio = %v", s.Gauge)
		}
	}
}

func TestViews(t *testing.T) {
	a := newAgg(t, nil)
	dns := []string{"application", "dns", "scenario", "query"}
	dnsTCP := []string{"application", "dns", "scenario", "tcp"}
	a.Add("agent-a", t0, snapshot("r/agent-a", 1,
		[]*agentpb.CounterValue{
			counter("sessions.attempted", 1, webGet...),
			counter("sessions.attempted", 2, dns...),
			counter("sessions.attempted", 4, dnsTCP...),
			counter("port.packets.tx", 8, "port", "p1"),
			counter("port.packets.tx", 16, "port", "p2"),
			counter("host.arp_failures", 1, "port", "p1", "host_pool", "a"),
			counter("host.arp_failures", 2, "port", "p1", "host_pool", "b"),
			counter("tcp.handshakes", 3, "port", "p1"),
			gauge("mix.max_abs_error_pp", 1.5),
		},
		hist("sessions.response_time", record(100, 200), dns...),
		hist("sessions.response_time", record(300), dnsTCP...),
	))
	app, _ := a.Latest("r", "application")
	if got := sampleOf(t, app, "dns").Values["sessions.attempted"]; got != 6 {
		t.Errorf("application dns attempted = %v, want 6", got)
	}
	if got := sampleOf(t, app, "dns").Values["sessions.response_time.count"]; got != 3 {
		t.Errorf("application dns response time count = %v, want 3", got)
	}
	if got := sampleOf(t, app, "dns").Values["sessions.response_time.max"]; got != float64(canonical.highestEquivalent(300)) {
		t.Errorf("application dns response time max = %v", got)
	}
	run := sampleOf(t, app, RunKey)
	if run.Values["sessions.attempted"] != 7 || run.Values["mix.max_abs_error_pp"] != 1.5 {
		t.Errorf("run sample %v", run.Values)
	}
	if app[0].Key != RunKey {
		t.Errorf("first sample key %q, want %q", app[0].Key, RunKey)
	}
	sc, _ := a.Latest("r", "scenario")
	if got := sampleOf(t, sc, "dns/tcp").Values["sessions.attempted"]; got != 4 {
		t.Errorf("scenario dns/tcp attempted = %v", got)
	}
	port, _ := a.Latest("r", "port")
	if v := sampleOf(t, port, "p1").Values; v["port.packets.tx"] != 8 || v["host.arp_failures"] != 3 {
		t.Errorf("port p1 %v", v)
	}
	if _, ok := sampleOf(t, port, "p1").Values["tcp.handshakes"]; ok {
		t.Error("port view holds a tcp metric")
	}
	tcp, _ := a.Latest("r", "tcp")
	if sampleOf(t, tcp, "p1").Values["tcp.handshakes"] != 3 {
		t.Error("tcp view lacks tcp.handshakes")
	}
	if tls, _ := a.Latest("r", "tls"); len(tls) != 0 {
		t.Errorf("tls view %v, want empty", tls)
	}
}

func TestWindows(t *testing.T) {
	clock := &fakeClock{now: t0}
	a := newAgg(t, func(c *Config) { c.Clock = clock })
	at := t0
	for i := uint64(0); i < 10; i++ {
		// Two snapshots in one second leave one point with the newer value.
		a.Add("agent-a", at, snapshot("r/agent-a", 2*i+1, []*agentpb.CounterValue{counter("sessions.attempted", 10*i, webGet...)}))
		a.Add("agent-a", at.Add(500*time.Millisecond), snapshot("r/agent-a", 2*i+2, []*agentpb.CounterValue{counter("sessions.attempted", 10*i+5, webGet...)}))
		at = at.Add(time.Second)
	}
	get := func(w Window) []float64 {
		ss, err := a.Samples("r", "application", w)
		if err != nil {
			t.Fatal(err)
		}
		var out []float64
		for _, s := range ss {
			if s.Key == RunKey {
				out = append(out, s.Values["sessions.attempted"])
			}
		}
		return out
	}
	all := get(Window{})
	if len(all) != 10 || all[0] != 5 || all[9] != 95 {
		t.Fatalf("whole run %v", all)
	}
	win := get(Window{From: t0.Add(2 * time.Second), To: t0.Add(4 * time.Second)})
	if len(win) != 3 || win[0] != 25 || win[2] != 45 {
		t.Fatalf("window %v", win)
	}
	step := get(Window{Step: 3 * time.Second})
	if want := []float64{25, 55, 85, 95}; !equalFloats(step, want) {
		t.Fatalf("step 3 s %v, want %v", step, want)
	}
	ss, _ := a.Samples("r", "application", Window{From: t0.Add(3 * time.Second), To: t0.Add(3 * time.Second)})
	if len(ss) == 0 || !ss[0].Ts.Equal(t0.Add(3*time.Second)) {
		t.Fatalf("sample time %v", ss)
	}
	if _, err := a.Samples("other", "application", Window{}); !errors.Is(err, ErrRunUnknown) {
		t.Fatalf("err = %v", err)
	}
}

func equalFloats(a, b []float64) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

func TestLimitsAndDrops(t *testing.T) {
	a := newAgg(t, func(c *Config) { c.MaxRuns = 2; c.MaxPoints = 3; c.MaxSeries = 2 })
	ok := a.Add("agent-a", t0, snapshot("r1/agent-a", 1, []*agentpb.CounterValue{
		counter("no.such.metric", 1),
		gauge("sessions.attempted", 1, webGet...),                                     // wrong kind
		counter("sessions.attempted", 1, "port", "p1"),                                // undeclared label
		counter("sessions.attempted", 1, webGet...),                                   // kept
		counter("sessions.succeeded", 1, webGet...),                                   // kept
		counter("sessions.established", 1, webGet...),                                 // past MaxSeries
		{Metric: "sessions.attempted", Labels: map[string]string{"application": "x"}}, // no value
	}, hist("sessions.response_time", []byte{1, 2, 3}, webGet...), hist("bytes.tx", record(1), webGet...)))
	if !ok {
		t.Fatal("snapshot rejected")
	}
	if got := a.Dropped("r1"); got != 7 {
		t.Errorf("dropped %d entries, want 7", got)
	}
	at := t0
	for seq := uint64(2); seq < 7; seq++ {
		at = at.Add(time.Second)
		a.Add("agent-a", at, snapshot("r1/agent-a", seq, nil))
	}
	ss, _ := a.Samples("r1", "application", Window{})
	points := 0
	for _, s := range ss {
		if s.Key == RunKey {
			points++
		}
	}
	if points != 3 {
		t.Errorf("%d points kept, want 3", points)
	}
	a.Add("agent-a", t0.Add(time.Minute), snapshot("r2/agent-a", 1, nil))
	a.Add("agent-a", t0.Add(2*time.Minute), snapshot("r3/agent-a", 1, nil))
	if a.Has("r1") || !a.Has("r2") || !a.Has("r3") {
		t.Errorf("runs after eviction %v", a.Runs())
	}
	a.Forget("r2")
	if a.Has("r2") {
		t.Error("Forget kept the run")
	}
	if a.Add("", t0, snapshot("r/x", 1, nil)) || a.Add("agent-a", t0, nil) || a.Add("agent-a", t0, snapshot("", 1, nil)) {
		t.Error("invalid snapshot accepted")
	}
}

func TestNonFiniteGaugeLeavesSample(t *testing.T) {
	a := newAgg(t, nil)
	a.Add("agent-a", t0, snapshot("r/agent-a", 1, []*agentpb.CounterValue{gauge("sessions.rate", math.NaN(), webGet...)}))
	app, _ := a.Latest("r", "application")
	for _, s := range app {
		if _, ok := s.Values["sessions.rate"]; ok {
			t.Fatal("NaN gauge in sample")
		}
	}
}

func TestValidView(t *testing.T) {
	for _, v := range Views {
		if !ValidView(v) {
			t.Errorf("%s invalid", v)
		}
	}
	if ValidView(api.MetricView("host")) {
		t.Error("host valid")
	}
}
