// SPDX-License-Identifier: GPL-3.0-only

package runs

import (
	"math"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs/calibrate"
)

func TestEngineAtLeast(t *testing.T) {
	for v, want := range map[string]bool{
		"0.1.0": false, "0.1.9": false, "0.2.0": true, "0.2.1": true, "0.3.0": true,
		"1.0.0": true, "0.2": true, "0.10.0": true, "": false, "0": false, "x.y.z": false, "0.x.0": false,
	} {
		if got := engineAtLeast(v, updateRatesMajor, updateRatesMinor); got != want {
			t.Errorf("engineAtLeast(%q) = %v, want %v", v, got, want)
		}
	}
}

func f64(v float64) *float64 { return &v }

// throughputPlans returns the plans of two agents for a mix of two entries
// that run scenarios dns/query and http1/web_get, with a throughput objective
// of 8 Mbit/s split 3:1 between the agents.
func throughputPlans() []compiler.Plan {
	mix := []compiler.MixEntry{
		{ID: "query", Scenario: "dns/query", Weight: 0.25},
		{ID: "web_get", Scenario: "http1/web_get", Weight: 0.75},
	}
	worker := func(q, w float64) []compiler.WorkerPlan {
		return []compiler.WorkerPlan{
			{WorkerID: 0, Rates: []compiler.WorkerRate{{MixEntry: "query", SessionsPerSecond: f64(q)}, {MixEntry: "web_get", SessionsPerSecond: f64(w)}}},
			{WorkerID: 1, Rates: []compiler.WorkerRate{{MixEntry: "query", SessionsPerSecond: f64(q)}, {MixEntry: "web_get", SessionsPerSecond: f64(w)}}},
		}
	}
	return []compiler.Plan{
		{PlanID: "r/a", AgentID: "a", Mix: mix, Workers: worker(10, 30), Load: &compiler.LoadPlan{Objective: objectiveThroughput, ObjectiveValue: 6e6}},
		{PlanID: "r/b", AgentID: "b", Mix: mix, Workers: worker(5, 10), Load: &compiler.LoadPlan{Objective: objectiveThroughput, ObjectiveValue: 2e6}},
	}
}

func catalogOf(query, webGet float64) []compiler.Scenario {
	return []compiler.Scenario{
		{Module: compiler.ModuleRef{Name: "dns"}, Name: "query", BytesPerSession: query},
		{Module: compiler.ModuleRef{Name: "http1"}, Name: "web_get", BytesPerSession: webGet},
	}
}

func TestNewCalibratorOnlyForThroughput(t *testing.T) {
	c, err := newCalibrator(CalibrationConfig{}, throughputPlans(), catalogOf(200, 4000))
	if err != nil || c == nil {
		t.Fatalf("throughput objective: calibrator %v, error %v", c, err)
	}
	if c.window != DefaultCalibrationWindow {
		t.Errorf("window %s, want the default %s", c.window, DefaultCalibrationWindow)
	}
	// Weighted bytes per session 0.25*200+0.75*4000 = 3050, so the compiled
	// total rate is 8e6/(8*3050) sessions per second.
	if got, want := c.state.BaseSessionsPerSecond(), 8e6/(8*3050); math.Abs(got-want) > 1e-9 {
		t.Errorf("base rate %v, want %v", got, want)
	}

	for name, mutate := range map[string]func([]compiler.Plan) []compiler.Plan{
		"sessions_per_second": func(ps []compiler.Plan) []compiler.Plan {
			for i := range ps {
				ps[i].Load.Objective = "LOAD_OBJECTIVE_SESSIONS_PER_SECOND"
			}
			return ps
		},
		"concurrent sessions": func(ps []compiler.Plan) []compiler.Plan {
			for i := range ps {
				ps[i].Load.Objective = "LOAD_OBJECTIVE_CONCURRENT_SESSIONS"
			}
			return ps
		},
		"no load":  func(ps []compiler.Plan) []compiler.Plan { ps[0].Load = nil; return ps },
		"no plans": func([]compiler.Plan) []compiler.Plan { return nil },
	} {
		if c, err := newCalibrator(CalibrationConfig{}, mutate(throughputPlans()), catalogOf(200, 4000)); c != nil || err != nil {
			t.Errorf("%s: calibrator %v, error %v, want neither", name, c, err)
		}
	}
	if c, err := newCalibrator(CalibrationConfig{Window: -1}, throughputPlans(), catalogOf(200, 4000)); c != nil || err != nil {
		t.Errorf("negative window: calibrator %v, error %v, want neither", c, err)
	}
	if _, err := newCalibrator(CalibrationConfig{}, throughputPlans(), catalogOf(200, 0)); err == nil {
		t.Error("a scenario without a byte estimate gave no error")
	}
	if _, err := newCalibrator(CalibrationConfig{}, throughputPlans(), catalogOf(200, 4000)[:1]); err == nil {
		t.Error("a scenario that the catalog lacks gave no error")
	}
}

func TestCalibrationSample(t *testing.T) {
	c, err := newCalibrator(CalibrationConfig{}, throughputPlans(), catalogOf(200, 4000))
	if err != nil {
		t.Fatal(err)
	}
	byEntry := func(ss []calibrate.Sample) map[string]calibrate.Sample {
		m := map[string]calibrate.Sample{}
		for _, s := range ss {
			m[s.Entry] = s
		}
		return m
	}

	// The first window counts everything since the start.
	got := byEntry(c.sample(map[string]calTotals{"query": {bytes: 1000, sessions: 5}, "web_get": {bytes: 40000, sessions: 10}}))
	if got["query"] != (calibrate.Sample{Entry: "query", Bytes: 1000, Sessions: 5}) || got["web_get"] != (calibrate.Sample{Entry: "web_get", Bytes: 40000, Sessions: 10}) {
		t.Fatalf("first window %v", got)
	}
	// The next window counts the difference.
	got = byEntry(c.sample(map[string]calTotals{"query": {bytes: 1600, sessions: 8}, "web_get": {bytes: 60000, sessions: 15}}))
	if got["query"].Bytes != 600 || got["query"].Sessions != 3 || got["web_get"].Bytes != 20000 || got["web_get"].Sessions != 5 {
		t.Fatalf("second window %v", got)
	}
	// Bytes without a completed session wait for the next window that has one.
	got = byEntry(c.sample(map[string]calTotals{"query": {bytes: 1600, sessions: 8}, "web_get": {bytes: 70000, sessions: 15}}))
	if got["web_get"] != (calibrate.Sample{Entry: "web_get"}) {
		t.Fatalf("a window without sessions gave %v", got["web_get"])
	}
	got = byEntry(c.sample(map[string]calTotals{"query": {bytes: 1600, sessions: 8}, "web_get": {bytes: 90000, sessions: 20}}))
	if got["web_get"].Bytes != 30000 || got["web_get"].Sessions != 5 {
		t.Fatalf("the window after it gave %v, want the 30000 bytes of both windows", got["web_get"])
	}
	// A total that fell restarts the count without a sample.
	got = byEntry(c.sample(map[string]calTotals{"query": {bytes: 100, sessions: 1}, "web_get": {bytes: 90000, sessions: 20}}))
	if got["query"].Sessions != 0 || got["query"].Bytes != 0 {
		t.Fatalf("a restarted counter gave %v", got["query"])
	}
	got = byEntry(c.sample(map[string]calTotals{"query": {bytes: 300, sessions: 2}, "web_get": {bytes: 90000, sessions: 20}}))
	if got["query"].Bytes != 200 || got["query"].Sessions != 1 {
		t.Fatalf("the window after a restart gave %v", got["query"])
	}
}

// TestCalibrationSampleSharedScenario checks that two mix entries of one
// scenario split its counters by weight and keep its bytes per session.
func TestCalibrationSampleSharedScenario(t *testing.T) {
	plans := throughputPlans()
	for i := range plans {
		plans[i].Mix = []compiler.MixEntry{
			{ID: "web_get", Scenario: "http1/web_get", Weight: 0.5},
			{ID: "web_get_2", Scenario: "http1/web_get", Weight: 0.5},
		}
	}
	c, err := newCalibrator(CalibrationConfig{}, plans, catalogOf(200, 4000)[1:])
	if err != nil {
		t.Fatal(err)
	}
	ss := c.sample(map[string]calTotals{"web_get": {bytes: 80000, sessions: 20}})
	if len(ss) != 2 {
		t.Fatalf("%d samples, want 2", len(ss))
	}
	for _, s := range ss {
		if s.Bytes != 40000 || s.Sessions != 10 {
			t.Errorf("sample %v, want half of the window", s)
		}
	}
}

func TestRatesCommand(t *testing.T) {
	plans := throughputPlans()
	c, err := newCalibrator(CalibrationConfig{}, plans, catalogOf(200, 4000))
	if err != nil {
		t.Fatal(err)
	}
	a := &runAgent{id: "a", plan: plans[0]}
	cmd := ratesCommand(a, c.entries, 0.5)
	if cmd.GetPlanId() != "r/a" || len(cmd.GetRates()) != 2 {
		t.Fatalf("command %v", cmd)
	}
	// Two workers carry 10 and 30 sessions per second each, and the command
	// holds the whole agent total, scaled by the factor.
	want := map[string]float64{"query": 10, "web_get": 30}
	for _, r := range cmd.GetRates() {
		if r.GetSessionsPerSecond() != want[r.GetMixEntry()] {
			t.Errorf("entry %s: %v sessions per second, want %v", r.GetMixEntry(), r.GetSessionsPerSecond(), want[r.GetMixEntry()])
		}
	}

	// An agent without client rates, such as a server side agent, gets none.
	srv := &runAgent{id: "s", plan: compiler.Plan{PlanID: "r/s", Workers: []compiler.WorkerPlan{{WorkerID: 0}}}}
	if cmd := ratesCommand(srv, c.entries, 0.5); cmd != nil {
		t.Errorf("server side agent got %v", cmd)
	}
	// An entry that the agent does not run stays out of the command.
	one := &runAgent{id: "o", plan: compiler.Plan{PlanID: "r/o", Workers: []compiler.WorkerPlan{{Rates: []compiler.WorkerRate{{MixEntry: "web_get", SessionsPerSecond: f64(8)}}}}}}
	cmd = ratesCommand(one, c.entries, 2)
	if len(cmd.GetRates()) != 1 || cmd.GetRates()[0].GetMixEntry() != "web_get" || cmd.GetRates()[0].GetSessionsPerSecond() != 16 {
		t.Errorf("single entry command %v", cmd)
	}
}
