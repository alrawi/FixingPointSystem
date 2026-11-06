// SPDX-License-Identifier: GPL-3.0-only

package runs

import "testing"

func agentSeries(ss ...Series) map[string]Series {
	out := map[string]Series{}
	for _, s := range ss {
		out[seriesKey(s.Metric, s.Labels)] = s
	}
	return out
}

func labels(side string, extra ...string) map[string]string {
	l := map[string]string{"application": "dns", "scenario": "query", "side": side}
	for i := 0; i+1 < len(extra); i += 2 {
		l[extra[i]] = extra[i+1]
	}
	return l
}

func find(ss []Series, metric string, want map[string]string) (float64, bool) {
	for _, s := range ss {
		if s.Metric == metric && seriesKey("", s.Labels) == seriesKey("", want) {
			return s.Value, true
		}
	}
	return 0, false
}

func TestCloseOpenCountsOpenSessionsAsAborted(t *testing.T) {
	client := labels("client")
	got := closeOpen(agentSeries(
		Series{Metric: MetricSessionsAttempted, Labels: client, Value: 100},
		Series{Metric: MetricSessionsSucceeded, Labels: client, Value: 40},
		Series{Metric: MetricSessionsFailed, Labels: labels("client", "reason", "connect_timeout"), Value: 6},
		Series{Metric: MetricSessionsFailed, Labels: labels("client", "reason", "internal"), Value: 4},
		// The server side closed all its sessions.
		Series{Metric: MetricSessionsAttempted, Labels: labels("server"), Value: 10},
		Series{Metric: MetricSessionsSucceeded, Labels: labels("server"), Value: 10},
	))
	var all []Series
	for _, s := range got {
		all = append(all, s)
	}
	if v, ok := find(all, MetricSessionsFailed, labels("client", "reason", ReasonAborted)); !ok || v != 50 {
		t.Errorf("aborted client sessions = %v (found %v), want 50", v, ok)
	}
	if _, ok := find(all, MetricSessionsFailed, labels("server", "reason", ReasonAborted)); ok {
		t.Error("the server side has no open sessions, yet it got aborted failures")
	}
}

func TestCloseOpenWithoutLabels(t *testing.T) {
	got := closeOpen(agentSeries(
		Series{Metric: MetricSessionsAttempted, Value: 10},
		Series{Metric: MetricSessionsSucceeded, Value: 4},
	))
	if s := got[seriesKey(MetricSessionsFailed, nil)]; s.Value != 6 || len(s.Labels) != 0 {
		t.Fatalf("unlabeled failed series %+v, want 6 without labels", s)
	}
}

func TestCloseOpenAddsToAnExistingAbortedSeries(t *testing.T) {
	client := labels("client")
	got := closeOpen(agentSeries(
		Series{Metric: MetricSessionsAttempted, Labels: client, Value: 10},
		Series{Metric: MetricSessionsFailed, Labels: labels("client", "reason", ReasonAborted), Value: 3},
	))
	if s := got[seriesKey(MetricSessionsFailed, labels("client", "reason", ReasonAborted))]; s.Value != 10 {
		t.Fatalf("aborted series %+v, want the 3 reported plus the 7 open", s)
	}
}

func TestSumSeriesAndTotalsCountTheClientSide(t *testing.T) {
	client, server := labels("client"), labels("server")
	perAgent := map[string]map[string]Series{
		"agent-a": agentSeries(
			Series{Metric: MetricSessionsAttempted, Labels: client, Value: 100},
			Series{Metric: MetricSessionsSucceeded, Labels: client, Value: 100},
			Series{Metric: MetricSessionsAttempted, Labels: server, Value: 100},
			Series{Metric: MetricSessionsSucceeded, Labels: server, Value: 100},
		),
		"agent-b": agentSeries(
			Series{Metric: MetricSessionsAttempted, Labels: client, Value: 100},
			Series{Metric: MetricSessionsSucceeded, Labels: client, Value: 50},
			Series{Metric: MetricSessionsAttempted, Labels: server, Value: 100},
			Series{Metric: MetricSessionsSucceeded, Labels: server, Value: 100},
		),
	}
	series := sumSeries(perAgent, map[string]bool{"agent-b": true})
	if v, _ := find(series, MetricSessionsAttempted, client); v != 200 {
		t.Errorf("client attempted = %v, want 200", v)
	}
	if v, _ := find(series, MetricSessionsAttempted, server); v != 200 {
		t.Errorf("server attempted = %v, want 200", v)
	}
	tot := totals(map[string]map[string]float64{"agent-a": nil, "agent-b": nil}, series, map[string]bool{"agent-b": true})
	if tot[MetricSessionsAttempted] != 200 || tot[MetricSessionsFailed] != 50 || tot[MetricSessionsFailedRatio] != 0.25 {
		t.Fatalf("totals %v, want 200 attempted, 50 failed, ratio 0.25 over the client side", tot)
	}
	for i := 1; i < len(series); i++ {
		a, b := series[i-1], series[i]
		if seriesKey(a.Metric, a.Labels) >= seriesKey(b.Metric, b.Labels) {
			t.Fatalf("series out of order at %d: %v before %v", i, a, b)
		}
	}
}

func TestTotalsMixUnlabeledAndLabeled(t *testing.T) {
	tot := totals(
		map[string]map[string]float64{"a": {MetricSessionsAttempted: 10, MetricSessionsFailed: 1}},
		[]Series{{Metric: MetricBytesTx, Labels: labels("client"), Value: 5}, {Metric: MetricBytesTx, Labels: labels("server"), Value: 7}, {Metric: MetricBytesRx, Value: 9}},
		nil,
	)
	if tot[MetricBytesTx] != 5 || tot[MetricSessionsFailedRatio] != 0.1 {
		t.Fatalf("totals %v", tot)
	}
	if _, ok := tot[MetricBytesRx]; ok {
		t.Fatal("an unlabeled series entered the totals twice, once through the unlabeled counters")
	}
}
