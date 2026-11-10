// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"reflect"
	"strings"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
	"github.com/alrawi/FixingPointSystem/controller/internal/store"
)

// labeledSeriesReport returns the report of the labeled run over the
// two sided aggregator of the labeled golden. The client side of dns/query
// attempted 1000 sessions and moved 1 000 000 + 2 000 000 bytes, and the
// client side of http1/web_get attempted 3000 sessions and moved 5 000 000 +
// 30 000 000 bytes. The server side counts the same sessions again with the
// directions swapped, and the table must leave it out.
func labeledSeriesReport(t *testing.T, run runs.Run) Report {
	t.Helper()
	series, err := twoSideAggregator(t, run.ID).Series(run.ID)
	if err != nil {
		t.Fatal(err)
	}
	return newEvaluator(t).BuildSeries(run, series)
}

// TestScenariosTable checks the table of the labeled run against literal
// counts, and that the CSV report carries the same rows.
func TestScenariosTable(t *testing.T) {
	rep := labeledSeriesReport(t, baseRun("run-labeled", runs.Completed, "agent-a"))
	want := []ScenarioCount{
		{Scenario: "dns/query", Sessions: 1000, Bytes: 3_000_000},
		{Scenario: "http1/web_get", Sessions: 3000, Bytes: 35_000_000},
	}
	if !reflect.DeepEqual(rep.Scenarios, want) {
		t.Fatalf("Scenarios = %+v, want %+v", rep.Scenarios, want)
	}
	csv, err := rep.CSV()
	if err != nil {
		t.Fatal(err)
	}
	for _, row := range []string{
		"scenario,dns/query,,,1000,,bytes=3000000\n",
		"scenario,http1/web_get,,,3000,,bytes=35000000\n",
	} {
		if !strings.Contains(string(csv), row) {
			t.Errorf("csv lacks the row %q", row)
		}
	}
}

// TestScenariosTableFollowsOrchestratorCounters checks that the table counts
// the sessions that the orchestrator holds for a series, as the totals do.
func TestScenariosTableFollowsOrchestratorCounters(t *testing.T) {
	run := baseRun("run-merged", runs.Failed, "agent-a", "agent-b")
	run.Series = []runs.Series{
		{Metric: "sessions.attempted", Labels: seriesLabels("dns", "query", "client"), Value: 1300},
		{Metric: "bytes.tx", Labels: seriesLabels("dns", "query", "client"), Value: 1_500_000},
	}
	rep := labeledSeriesReport(t, run)
	want := []ScenarioCount{
		{Scenario: "dns/query", Sessions: 1300, Bytes: 3_500_000}, // 1 500 000 tx and 2 000 000 rx
		{Scenario: "http1/web_get", Sessions: 3000, Bytes: 35_000_000},
	}
	if !reflect.DeepEqual(rep.Scenarios, want) {
		t.Fatalf("Scenarios = %+v, want %+v", rep.Scenarios, want)
	}
}

// TestReportWithoutSeriesHasNoScenarios checks that a report built from
// totals alone has no table, and that a series without a scenario label does
// not make a row.
func TestReportWithoutSeriesHasNoScenarios(t *testing.T) {
	e := newEvaluator(t)
	rep := e.Build(baseRun("run-totals", runs.Completed, "agent-a"), map[string]float64{"sessions.attempted": 10})
	if rep.Scenarios != nil {
		t.Errorf("Scenarios = %+v, want nil", rep.Scenarios)
	}
	run := baseRun("run-bare", runs.Completed, "agent-a")
	run.Series = []runs.Series{{Metric: "sessions.attempted", Labels: map[string]string{"application": "dns"}, Value: 5}}
	if got := e.BuildSeries(run, nil).Scenarios; got != nil {
		t.Errorf("Scenarios = %+v, want nil", got)
	}
}

// TestFileJSONCarriesTheTable checks that the file report holds the table
// in the form that the RLM-07 mix check reads, next to the RunReport, and that
// the table form of the file shadows the scenarios of the REST report.
func TestFileJSONCarriesTheTable(t *testing.T) {
	rep := labeledSeriesReport(t, baseRun("run-labeled", runs.Completed, "agent-a"))
	file, err := rep.FileJSON()
	if err != nil {
		t.Fatal(err)
	}
	var table struct {
		Scenarios []struct {
			Scenario string `json:"scenario"`
			Sessions uint64 `json:"sessions"`
			Bytes    uint64 `json:"bytes"`
		} `json:"scenarios"`
	}
	if err := json.Unmarshal(file, &table); err != nil {
		t.Fatal(err)
	}
	if len(table.Scenarios) != 2 || table.Scenarios[0].Scenario != "dns/query" || table.Scenarios[0].Sessions != 1000 ||
		table.Scenarios[0].Bytes != 3_000_000 || table.Scenarios[1].Scenario != "http1/web_get" ||
		table.Scenarios[1].Sessions != 3000 || table.Scenarios[1].Bytes != 35_000_000 {
		t.Errorf("scenarios of the file report = %+v", table.Scenarios)
	}
	rest, err := rep.JSON()
	if err != nil {
		t.Fatal(err)
	}
	var restDoc, fileDoc map[string]json.RawMessage
	if err := json.Unmarshal(rest, &restDoc); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(file, &fileDoc); err != nil {
		t.Fatal(err)
	}
	if string(restDoc["scenarios"]) == string(fileDoc["scenarios"]) {
		t.Error("the file report carries the scenarios of the REST report and not its own table")
	}
	for _, key := range []string{"run", "realism"} {
		if string(restDoc[key]) != string(fileDoc[key]) {
			t.Errorf("%s differs between the REST report and the file report", key)
		}
	}
	if _, ok := fileDoc["declared"]; ok {
		t.Error("the file report has declarations without a source")
	}
}

// TestFileJSONCarriesDeclarations checks that the declarations of the test
// configuration reach the file report as the realism targets.
func TestFileJSONCarriesDeclarations(t *testing.T) {
	mean := 20.0
	rep := Report{Run: baseRun("run-declared", runs.Completed, "agent-a"), Declared: &compiler.Declarations{
		Arrival: &compiler.Arrival{Dist: "exponential", MeanMs: &mean},
	}}
	file, err := rep.FileJSON()
	if err != nil {
		t.Fatal(err)
	}
	var doc struct {
		Declared json.RawMessage `json:"declared"`
	}
	if err := json.Unmarshal(file, &doc); err != nil {
		t.Fatal(err)
	}
	var compact bytes.Buffer
	if err := json.Compact(&compact, doc.Declared); err != nil {
		t.Fatal(err)
	}
	if want := `{"arrival":{"dist":"exponential","mean_ms":20}}`; compact.String() != want {
		t.Errorf("declared = %s, want %s", compact.String(), want)
	}
}

// fakeConfigs is a ConfigStore over fixed configuration bodies.
type fakeConfigs map[string]string

func (f fakeConfigs) Get(_ context.Context, id string) (store.Record, error) {
	body, ok := f[id]
	if !ok {
		return store.Record{}, errors.New("no such configuration")
	}
	return store.Record{Body: []byte(body)}, nil
}

// TestConfigObjectivesServeDeclarations checks the source of the declarations
// over a stored configuration, and that UseObjectives adopts it.
func TestConfigObjectivesServeDeclarations(t *testing.T) {
	const body = `
profile:
  basis: sessions
  arrival: { dist: exponential, mean_ms: 20 }
  size: { both: { dist: constant, value: 512 } }
  mix:
    - { scenario: dns/lookup, weight: 1 }
load:
  objective: { type: sessions_per_second, value: 100 }
`
	src := ConfigObjectives{Store: fakeConfigs{"cfg-a": body, "cfg-plain": "profile: { basis: sessions }\n"}}
	got, err := src.Declarations(context.Background(), "cfg-a")
	if err != nil {
		t.Fatal(err)
	}
	if got == nil || got.Arrival == nil || got.Arrival.Dist != "exponential" || *got.Arrival.MeanMs != 20 ||
		got.Size == nil || got.Size.Both == nil || *got.Size.Both.Value != 512 {
		t.Errorf("Declarations = %+v", got)
	}
	if len(got.Mix) != 1 || got.Mix[0].Scenario != "dns/lookup" || got.Mix[0].Arrival != nil || got.Mix[0].Size == nil || *got.Mix[0].Size.Both.Value != 512 {
		t.Errorf("mix declarations = %+v, want dns/lookup with the profile size and no arrival", got.Mix)
	}
	if d, err := src.Declarations(context.Background(), "cfg-plain"); err != nil || d != nil {
		t.Errorf("a configuration without declarations gave %+v, %v", d, err)
	}
	if _, err := src.Declarations(context.Background(), "cfg-gone"); err == nil {
		t.Error("an unknown configuration gave no error")
	}
	a := NewAPI(newEvaluator(t), fakeRuns{}, nil).UseObjectives(src)
	if a.declarations == nil {
		t.Error("UseObjectives did not adopt the declaration source")
	}
}

// TestScenarioTotalsInTheRESTReport checks the scenarios of the REST report
// against literal counts: the client side only, the failures of every reason
// added, a count of 0 for a counter that the run did not record, and sums
// that equal the run summary.
func TestScenarioTotalsInTheRESTReport(t *testing.T) {
	rep := labeledSeriesReport(t, baseRun("run-labeled", runs.Completed, "agent-a"))
	got := rep.API()
	want := []api.ScenarioTotals{
		{Application: "dns", Scenario: "query", SessionsAttempted: 1000, SessionsSucceeded: 990, SessionsFailed: 10, BytesTx: 1_000_000, BytesRx: 2_000_000},
		{Application: "http1", Scenario: "web_get", SessionsAttempted: 3000, SessionsSucceeded: 3000, SessionsFailed: 0, BytesTx: 5_000_000, BytesRx: 30_000_000},
	}
	if got.Scenarios == nil || !reflect.DeepEqual(*got.Scenarios, want) {
		t.Fatalf("Scenarios = %+v, want %+v", got.Scenarios, want)
	}
	sum := api.RunSummary{}
	for _, sc := range *got.Scenarios {
		sum.SessionsAttempted = ptr(sc.SessionsAttempted + deref(sum.SessionsAttempted))
		sum.SessionsSucceeded = ptr(sc.SessionsSucceeded + deref(sum.SessionsSucceeded))
		sum.SessionsFailed = ptr(sc.SessionsFailed + deref(sum.SessionsFailed))
		sum.BytesTx = ptr(sc.BytesTx + deref(sum.BytesTx))
		sum.BytesRx = ptr(sc.BytesRx + deref(sum.BytesRx))
	}
	s := got.Run.Summary
	if s == nil || *s.SessionsAttempted != *sum.SessionsAttempted || *s.SessionsSucceeded != *sum.SessionsSucceeded ||
		*s.SessionsFailed != *sum.SessionsFailed || *s.BytesTx != *sum.BytesTx || *s.BytesRx != *sum.BytesRx {
		t.Errorf("the scenarios add up to %+v, the run summary is %+v", sum, s)
	}
}

func deref(p *int) int {
	if p == nil {
		return 0
	}
	return *p
}

// TestRESTReportWithoutScenariosSendsAnEmptyList checks that a report built
// from totals alone still sends scenarios, as the contract requires of a
// controller of 0.9.0 or later, and that the JSON shows an empty array.
func TestRESTReportWithoutScenariosSendsAnEmptyList(t *testing.T) {
	rep := newEvaluator(t).Build(baseRun("run-totals", runs.Completed, "agent-a"), map[string]float64{"sessions.attempted": 10})
	got := rep.API()
	if got.Scenarios == nil || len(*got.Scenarios) != 0 {
		t.Fatalf("Scenarios = %+v, want an empty list", got.Scenarios)
	}
	body, err := rep.JSON()
	if err != nil {
		t.Fatal(err)
	}
	var doc map[string]json.RawMessage
	if err := json.Unmarshal(body, &doc); err != nil {
		t.Fatal(err)
	}
	if string(doc["scenarios"]) != "[]" || string(doc["realism"]) != "[]" {
		t.Errorf("scenarios = %s, realism = %s, want empty arrays", doc["scenarios"], doc["realism"])
	}
}

// TestCriterionWhereInTheRESTReport checks that the label filter of a
// criterion reaches its result as stated, without a side that the
// configuration left out, and that a criterion without a filter has none. Two
// criteria that differ only in their filter keep their own.
func TestCriterionWhereInTheRESTReport(t *testing.T) {
	run := baseRun("run-where", runs.Completed, "agent-a")
	run.Verdict = verdictOf(true,
		crit("sessions.attempted", ">", 0),
		where(crit("sessions.attempted", ">", 0), "application", "dns"),
		where(crit("sessions.attempted", ">", 0), "application", "http1", "side", "server"),
	)
	rep := newEvaluator(t).BuildSeries(run, nil)
	got := rep.API()
	if got.Run.Verdict == nil || got.Run.Verdict.Criteria == nil || len(*got.Run.Verdict.Criteria) != 3 {
		t.Fatalf("verdict = %+v", got.Run.Verdict)
	}
	cs := *got.Run.Verdict.Criteria
	if cs[0].Where != nil {
		t.Errorf("a criterion without a filter has where %v", *cs[0].Where)
	}
	if cs[1].Where == nil || !reflect.DeepEqual(*cs[1].Where, map[string]string{"application": "dns"}) {
		t.Errorf("where of the dns criterion = %v", cs[1].Where)
	}
	if cs[2].Where == nil || !reflect.DeepEqual(*cs[2].Where, map[string]string{"application": "http1", "side": "server"}) {
		t.Errorf("where of the http1 criterion = %v", cs[2].Where)
	}
	// The result owns its map: a change of it leaves the stored criterion alone.
	(*cs[1].Where)["application"] = "changed"
	if rep.Run.Verdict.Criteria[1].Where["application"] != "dns" {
		t.Error("the REST result shares the where map of the report")
	}
}

// TestRealismResultsInTheRESTReport checks that the stored realism results of
// a report reach the REST report in their order, and that a report without
// any sends an empty array.
func TestRealismResultsInTheRESTReport(t *testing.T) {
	labels := map[string]string{"application": "http1", "classifier": "ndpi"}
	metric := "realism.conformance_ratio"
	rep := Report{Run: baseRun("run-realism", runs.Completed, "agent-a"), Realism: []api.RealismResult{
		{Metric: &metric, Labels: &labels, Property: "conformance of http1 by ndpi", Target: ">= 0.99", Observed: 0.995, Passed: true},
		{Property: "mix error", Target: "<= 2", Observed: 3.5, Passed: false},
	}}
	got := rep.API().Realism
	want := []api.RealismResult{
		{Metric: &metric, Labels: &labels, Property: "conformance of http1 by ndpi", Target: ">= 0.99", Observed: 0.995, Passed: true},
		{Property: "mix error", Target: "<= 2", Observed: 3.5, Passed: false},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("Realism = %+v, want %+v", got, want)
	}
	if empty := (Report{Run: rep.Run}).API().Realism; empty == nil || len(empty) != 0 {
		t.Errorf("a report without realism results sends %#v, want an empty array", empty)
	}
}
