// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"bytes"
	"encoding/binary"
	"net/http"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
	"github.com/alrawi/FixingPointSystem/controller/internal/metrics"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"

	golden "github.com/alrawi/FixingPointSystem/tests/golden/go"
)

// encodeHist returns the uncompressed HdrHistogram V2 encoding of counts,
// which maps values below 2048 to their number of samples. The canonical
// layout of the aggregator, with three significant digits, keeps such values
// exact, so every statistic of the histogram is a value that the test chose.
func encodeHist(counts map[uint64]uint64) []byte {
	var top uint64
	for v := range counts {
		top = max(top, v)
	}
	var payload []byte
	put := func(mag uint64, negative bool) {
		u := mag << 1
		if negative {
			u = (mag-1)<<1 | 1
		}
		for range 8 {
			if u < 0x80 {
				payload = append(payload, byte(u))
				return
			}
			payload = append(payload, byte(u)|0x80)
			u >>= 7
		}
		payload = append(payload, byte(u))
	}
	for i := uint64(0); i <= top; {
		if c := counts[i]; c != 0 {
			put(c, false)
			i++
			continue
		}
		j := i
		for j <= top && counts[j] == 0 {
			j++
		}
		if j-i == 1 {
			put(0, false)
		} else {
			put(j-i, true)
		}
		i = j
	}
	var b bytes.Buffer
	for _, v := range []any{
		uint32(0x1c849313), uint32(len(payload)), //nolint:gosec // G115: test payloads are small
		uint32(0), uint32(3), uint64(1), uint64(3_600_000_000), float64(1),
	} {
		_ = binary.Write(&b, binary.BigEndian, v)
	}
	b.Write(payload)
	return b.Bytes()
}

// seriesLabels returns the labels of a scenario series, with extra name and
// value pairs.
func seriesLabels(application, scenario, sd string, extra ...string) map[string]string {
	l := map[string]string{"application": application, "scenario": scenario, "side": sd}
	for i := 0; i+1 < len(extra); i += 2 {
		l[extra[i]] = extra[i+1]
	}
	return l
}

// where returns c with the label filter of the name and value pairs kv.
func where(c runs.Criterion, kv ...string) runs.Criterion {
	c.Where = map[string]string{}
	for i := 0; i+1 < len(kv); i += 2 {
		c.Where[kv[i]] = kv[i+1]
	}
	return c
}

func counter(metric string, n uint64, labels map[string]string) *agentpb.CounterValue {
	return &agentpb.CounterValue{Metric: metric, Labels: labels, Value: &agentpb.CounterValue_Count{Count: n}}
}

func hist(metric string, labels map[string]string, counts map[uint64]uint64) *agentpb.HistogramValue {
	return &agentpb.HistogramValue{Metric: metric, Labels: labels, HdrV2: encodeHist(counts)}
}

// twoSideAggregator returns an aggregator that holds the run runID with the
// session metrics of two applications, each seen from the client and from the
// server side, as the engine reports them once it labels the side. The server
// side of http1 saw 60 failed sessions that the client side did not, and its
// response times differ from the client side.
//
//	client dns/query     1000 attempted, 990 succeeded, 10 failed, response time 800
//	server dns/query     1000 attempted, 1000 succeeded,           response time 1500
//	client http1/web_get 3000 attempted, 3000 succeeded,           response time 300
//	server http1/web_get 3000 attempted, 2940 succeeded, 60 failed, response time 400
func twoSideAggregator(t *testing.T, runID string) *metrics.Aggregator {
	t.Helper()
	agg, err := metrics.New(metrics.Config{})
	if err != nil {
		t.Fatal(err)
	}
	dnsC, dnsS := seriesLabels("dns", "query", "client"), seriesLabels("dns", "query", "server")
	webC, webS := seriesLabels("http1", "web_get", "client"), seriesLabels("http1", "web_get", "server")
	snap := &agentpb.CounterSnapshot{
		PlanId: runID + "/agent-a", Sequence: 1,
		Counters: []*agentpb.CounterValue{
			counter("sessions.attempted", 1000, dnsC), counter("sessions.succeeded", 990, dnsC),
			counter("sessions.failed", 10, seriesLabels("dns", "query", "client", "reason", "connect_timeout")),
			counter("bytes.tx", 1_000_000, dnsC), counter("bytes.rx", 2_000_000, dnsC),
			counter("sessions.attempted", 1000, dnsS), counter("sessions.succeeded", 1000, dnsS),
			counter("bytes.tx", 2_000_000, dnsS), counter("bytes.rx", 1_000_000, dnsS),
			counter("sessions.attempted", 3000, webC), counter("sessions.succeeded", 3000, webC),
			counter("bytes.tx", 5_000_000, webC), counter("bytes.rx", 30_000_000, webC),
			counter("sessions.attempted", 3000, webS), counter("sessions.succeeded", 2940, webS),
			counter("sessions.failed", 60, seriesLabels("http1", "web_get", "server", "reason", "expect_timeout")),
			counter("bytes.tx", 30_000_000, webS), counter("bytes.rx", 5_000_000, webS),
		},
		Histograms: []*agentpb.HistogramValue{
			hist("sessions.response_time", dnsC, map[uint64]uint64{800: 1000}),
			hist("sessions.response_time", dnsS, map[uint64]uint64{1500: 1000}),
			hist("sessions.response_time", webC, map[uint64]uint64{300: 3000}),
			hist("sessions.response_time", webS, map[uint64]uint64{400: 3000}),
		},
	}
	if !agg.Add("agent-a", t0, snap) {
		t.Fatal("aggregator rejected the snapshot")
	}
	return agg
}

// labeledCriteria are the criteria of the labeled golden report. The numbers
// in the comments are the values over the client side, which the criteria
// count unless they name a side.
func labeledCriteria() []runs.Criterion {
	return []runs.Criterion{
		crit("sessions.attempted", "==", 4000),                                  // 4000 on the client side, and 8000 over both
		where(crit("sessions.failed_ratio", "<", 0.005), "application", "dns"),  // 10 of 1000
		where(crit("sessions.failed_ratio", "<", 0.01), "side", "server"),       // 60 of 4000
		where(crit("sessions.failed_ratio", "<", 0.01), "application", "http1"), // no failures on the client
		crit("sessions.response_time.p99", "<=", 1000),                          // 800 on the client side
		where(crit("sessions.response_time.p99", "<=", 1000), "application", "dns", "side", "server"),
		where(crit("bytes.tx", "<", 1), "port", "eth0"),
		where(crit("sessions.attempted", ">", 0), "application", "ftp"),
	}
}

func labeledRun() runs.Run {
	run := baseRun("run-labeled", runs.Failed, "agent-a")
	run.Reason = "criteria failed: sessions.failed_ratio: observed 0.01 < 0.005 does not hold for application=dns,side=client; " +
		"sessions.failed_ratio: observed 0.015 < 0.01 does not hold for side=server; " +
		"sessions.response_time.p99: observed 1500 <= 1000 does not hold for application=dns,side=server; and 1 more"
	run.Verdict = verdictOf(false, labeledCriteria()...)
	return run
}

// TestGoldenLabeledReport is the golden report of a failed run with a
// reason, filtered criteria, and histogram criteria. The criteria without a
// side count the client side, the criteria with side=server count the server
// side, and an unknown label or a missing series has its own result.
func TestGoldenLabeledReport(t *testing.T) {
	h := newHarness(t, fakeRuns{"run-labeled": labeledRun()}, twoSideAggregator(t, "run-labeled"))
	rec := h.get(t, "run-labeled", "")
	if rec.Code != http.StatusOK {
		t.Fatalf("status %d: %s", rec.Code, rec.Body)
	}
	golden.Assert(t, "labeled.json", rec.Body.Bytes())
	rec = h.get(t, "run-labeled", "format=csv")
	if rec.Code != http.StatusOK {
		t.Fatalf("csv: status %d: %s", rec.Code, rec.Body)
	}
	golden.Assert(t, "labeled.csv", rec.Body.Bytes())
}

// TestClientSideIsTheDefault checks that a criterion without a side counts
// each session once, and that side=server selects the other side.
func TestClientSideIsTheDefault(t *testing.T) {
	e := newEvaluator(t)
	agg := twoSideAggregator(t, "run-sides")
	series, err := agg.Series("run-sides")
	if err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		name string
		c    runs.Criterion
		want float64 // zero means that no series matches, so the criterion is missing
	}{
		{"without a side", crit("sessions.attempted", ">=", 0), 4000},
		{"client", where(crit("sessions.attempted", ">=", 0), "side", "client"), 4000},
		{"server", where(crit("sessions.attempted", ">=", 0), "side", "server"), 4000},
		{"failed without a side", crit("sessions.failed", ">=", 0), 10},
		{"failed on the server", where(crit("sessions.failed", ">=", 0), "side", "server"), 60},
		{"ratio without a side", crit("sessions.failed_ratio", ">=", 0), 0.0025},
		{"ratio on the server", where(crit("sessions.failed_ratio", ">=", 0), "side", "server"), 0.015},
		{"bytes without a side", crit("bytes.tx", ">=", 0), 6_000_000},
		{"bytes on the server", where(crit("bytes.tx", ">=", 0), "side", "server"), 32_000_000},
		{"p99 without a side", crit("sessions.response_time.p99", ">=", 0), 800},
		{"p99 on the server", where(crit("sessions.response_time.p99", ">=", 0), "side", "server"), 1500},
		{"application dns", where(crit("sessions.attempted", ">=", 0), "application", "dns"), 1000},
		{"application dns on the server", where(crit("bytes.rx", ">=", 0), "application", "dns", "side", "server"), 1_000_000},
		{"reason", where(crit("sessions.failed", ">=", 0), "reason", "connect_timeout"), 10},
		{"reason on the server", where(crit("sessions.failed", ">=", 0), "reason", "connect_timeout", "side", "server"), 0},
	}
	for _, c := range cases {
		res := e.EvaluateData([]runs.Criterion{c.c}, Data{Series: series, Labeled: true}).Results[0]
		if c.want == 0 {
			if res.Status != Missing {
				t.Errorf("%s: %s %q, want missing", c.name, res.Status, res.Reason)
			}
			continue
		}
		if res.Status != Pass || res.Observed != c.want {
			t.Errorf("%s: %s observed %v (%s), want %v", c.name, res.Status, res.Observed, res.Reason, c.want)
		}
	}
	// A total over both sides would count every session twice.
	totals, err := agg.Totals("run-sides")
	if err != nil {
		t.Fatal(err)
	}
	if totals["sessions.attempted"] != 8000 {
		t.Fatalf("the aggregator totals count %v sessions, the premise of the default side is 8000", totals["sessions.attempted"])
	}
}

func TestFilterReasons(t *testing.T) {
	e := newEvaluator(t)
	series, err := twoSideAggregator(t, "run-reasons").Series("run-reasons")
	if err != nil {
		t.Fatal(err)
	}
	cases := []struct {
		c      runs.Criterion
		status Status
		reason string
	}{
		{crit("sessions.attempted", "==", 4000), Pass, "observed 4000 == 4000 holds for side=client"},
		{where(crit("sessions.attempted", "==", 4000), "side", "server"), Pass, "observed 4000 == 4000 holds for side=server"},
		{where(crit("sessions.failed_ratio", "<", 0.005), "application", "dns"), Fail, "observed 0.01 < 0.005 does not hold for application=dns,side=client"},
		{where(crit("sessions.attempted", ">", 0), "application", "ftp"), Missing, "the run reported no value for application=ftp,side=client"},
		{where(crit("sessions.failed_ratio", "<", 1), "application", "ftp"), Missing, "undefined because the run reported no sessions.attempted for application=ftp,side=client"},
		{where(crit("bytes.tx", "<", 1), "port", "eth0"), Invalid, "metric bytes.tx has no label port"},
		{where(crit("sessions.failed_ratio", "<", 1), "reason", "internal"), Invalid, "metric sessions.failed_ratio has no label reason"},
		{where(crit("port.drops", "==", 0), "port", "p0"), Missing, "the run reported no value for port=p0"},
	}
	for _, c := range cases {
		res := e.EvaluateData([]runs.Criterion{c.c}, Data{Series: series, Labeled: true}).Results[0]
		if res.Status != c.status || res.Reason != c.reason {
			t.Errorf("%s %v: %s %q, want %s %q", c.c.Metric, c.c.Where, res.Status, res.Reason, c.status, c.reason)
		}
	}
}

// TestFilterNeedsLabeledMetrics checks that totals without labels cannot
// answer a criterion with a where filter.
func TestFilterNeedsLabeledMetrics(t *testing.T) {
	e := newEvaluator(t)
	c := crit("bytes.tx", ">", 0)
	c.Where = map[string]string{"application": "dns"}
	res := e.Evaluate([]runs.Criterion{c}, map[string]float64{"bytes.tx": 5}).Results[0]
	if res.Status != Missing {
		t.Fatalf("%s %q, want missing", res.Status, res.Reason)
	}
}

// TestSeriesWithoutSideLabelStayInTheDefault checks that series that predate
// the side label count for a criterion without a side, and that a criterion
// with a side leaves them out.
func TestSeriesWithoutSideLabelStayInTheDefault(t *testing.T) {
	e := newEvaluator(t)
	agg, err := metrics.New(metrics.Config{})
	if err != nil {
		t.Fatal(err)
	}
	labels := map[string]string{"application": "http1", "scenario": "web_get"}
	if !agg.Add("agent-a", t0, &agentpb.CounterSnapshot{PlanId: "run-old/agent-a", Sequence: 1, Counters: []*agentpb.CounterValue{
		counter("sessions.attempted", 7, labels),
	}}) {
		t.Fatal("aggregator rejected the snapshot")
	}
	series, err := agg.Series("run-old")
	if err != nil {
		t.Fatal(err)
	}
	server := crit("sessions.attempted", "==", 7)
	server.Where = map[string]string{"side": "server"}
	ev := e.EvaluateData([]runs.Criterion{crit("sessions.attempted", "==", 7), server}, Data{Series: series, Labeled: true})
	if ev.Results[0].Status != Pass || ev.Results[1].Status != Missing {
		t.Fatalf("results %+v", ev.Results)
	}
}

// TestRunEvaluatorDecidesHistogramsByRunID checks that the evaluator of the
// orchestrator reads the aggregator by the run id of the outcome, so that a
// histogram criterion decides the run state as it decides the report, and
// that the verdict equals the one of the report.
func TestRunEvaluatorDecidesHistogramsByRunID(t *testing.T) {
	e := newEvaluator(t)
	agg := twoSideAggregator(t, "run-labeled")
	NewAPI(e, fakeRuns{}, agg)
	criteria := labeledCriteria()
	o := runs.Outcome{RunID: "run-labeled", Criteria: criteria, Agents: 1}
	v := e.RunEvaluator()(o)
	if v.Passed {
		t.Fatal("a run with failing criteria passed")
	}
	want := []Status{Pass, Fail, Fail, Pass, Pass, Fail, Invalid, Missing}
	series, err := agg.Series("run-labeled")
	if err != nil {
		t.Fatal(err)
	}
	built := e.BuildSeries(runs.Run{ID: "run-labeled", Agents: []string{"agent-a"}, Verdict: &runs.Verdict{Criteria: resultsOf(criteria)}}, series)
	if built.Evaluation == nil || len(built.Evaluation.Results) != len(want) || len(v.Criteria) != len(want) {
		t.Fatalf("verdict %+v, report %+v", v, built.Evaluation)
	}
	for i, w := range want {
		got := built.Evaluation.Results[i]
		if got.Status != w {
			t.Errorf("report criterion %d: %s %q, want %s", i, got.Status, got.Reason, w)
		}
		r := v.Criteria[i]
		if r.Reason != got.Reason || r.Evaluated != got.Evaluated() || r.Observed != got.Observed || r.Passed != (got.Status == Pass) || r.Invalid != (got.Status == Invalid) {
			t.Errorf("criterion %d: run verdict %+v differs from the report %+v", i, r, got)
		}
	}
	// Without a source and without series the histogram criterion has no
	// value, which is the gap that the run id closes.
	bare := newEvaluator(t)
	p99 := runs.Outcome{RunID: "run-labeled", Criteria: []runs.Criterion{crit("sessions.response_time.p99", "<=", 1000)}, Agents: 1}
	if r := bare.RunEvaluator()(p99).Criteria[0]; r.Evaluated {
		t.Fatalf("a histogram criterion was evaluated without a source: %+v", r)
	}
	if r := e.RunEvaluator()(p99).Criteria[0]; !r.Evaluated || !r.Passed || r.Observed != 800 {
		t.Fatalf("histogram criterion over the aggregator: %+v", r)
	}
}

func resultsOf(cs []runs.Criterion) []runs.CriterionResult {
	out := make([]runs.CriterionResult, len(cs))
	for i, c := range cs {
		out[i] = runs.CriterionResult{Criterion: c}
	}
	return out
}

// TestOrchestratorCountersTakePrecedence checks that the counters that the
// orchestrator holds replace the aggregator values of the same series, and
// that a series that only the orchestrator holds joins.
func TestOrchestratorCountersTakePrecedence(t *testing.T) {
	e := newEvaluator(t)
	agg := twoSideAggregator(t, "run-merge")
	NewAPI(e, fakeRuns{}, agg)
	dns := map[string]string{"application": "dns", "scenario": "query", "side": "client"}
	failed := map[string]string{"application": "dns", "scenario": "query", "side": "client", "reason": "aborted"}
	o := runs.Outcome{
		RunID: "run-merge", Agents: 2, FailedAgents: 1,
		Criteria: []runs.Criterion{crit("sessions.failed", "==", 310), crit("sessions.attempted", "==", 4300), crit("throughput.bps", "==", 9)},
		Series: []runs.Series{
			{Metric: "sessions.attempted", Labels: dns, Value: 1300},
			{Metric: "sessions.failed", Labels: failed, Value: 300},
			{Metric: "unknown.metric", Value: 1},
			{Metric: "sessions.failed", Labels: map[string]string{"bogus": "x"}, Value: 1},
		},
		Totals: map[string]float64{"throughput.bps": 9},
	}
	v := e.RunEvaluator()(o)
	for i, c := range v.Criteria {
		if !c.Evaluated || !c.Passed {
			t.Errorf("criterion %d %s: %+v", i, o.Criteria[i].Metric, c)
		}
	}
}

// TestUnlabeledRunTotalsRepeatLabeledSeries checks that a run total without
// labels counts only for a metric that has no labeled series, so a snapshot
// that carries both counts each session once.
func TestUnlabeledRunTotalsRepeatLabeledSeries(t *testing.T) {
	e := newEvaluator(t)
	agg, err := metrics.New(metrics.Config{})
	if err != nil {
		t.Fatal(err)
	}
	labels := map[string]string{"application": "http1", "scenario": "web_get"}
	if !agg.Add("agent-a", t0, &agentpb.CounterSnapshot{PlanId: "run-total/agent-a", Sequence: 1, Counters: []*agentpb.CounterValue{
		counter("sessions.attempted", 7, labels), counter("sessions.attempted", 7, nil),
		counter("tls.handshakes", 4, nil),
	}}) {
		t.Fatal("aggregator rejected the snapshot")
	}
	series, err := agg.Series("run-total")
	if err != nil {
		t.Fatal(err)
	}
	ev := e.EvaluateData([]runs.Criterion{
		crit("sessions.attempted", "==", 7),
		crit("tls.handshakes", "==", 4),
	}, Data{Series: series, Labeled: true})
	for i, res := range ev.Results {
		if res.Status != Pass {
			t.Errorf("criterion %d: %s %q, want pass", i, res.Status, res.Reason)
		}
	}
}
