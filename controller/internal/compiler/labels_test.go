// SPDX-License-Identifier: GPL-3.0-only

package compiler

import (
	"errors"
	"path/filepath"
	"strings"
	"testing"
)

// TestLabeledCriteriaReachTheAgentsThatCanApplyThem checks CON-15: the where
// filter of a criterion becomes Criterion.labels in every plan, an agent
// below engine_api 0.2.3 makes the compiler refuse the plan, and a criterion
// without a filter needs no version.
func TestLabeledCriteriaReachTheAgentsThatCanApplyThem(t *testing.T) {
	req := func(t *testing.T, engine ...string) Request {
		cfg := loadConfig(t, filepath.Join("testdata", "two_agents.yaml"))
		cfg.Criteria = []CriterionConfig{
			{Metric: "sessions.failed", Op: "<=", Value: 0},
			{Metric: "sessions.failed", Op: "<=", Value: 0, Where: map[string]string{"application": "dns", "side": "server"}},
		}
		r := Request{RunID: "r", Config: cfg, Scenarios: exampleCatalog(t)}
		for i, v := range engine {
			r.Agents = append(r.Agents, Agent{ID: string(rune('a' + i)), Cores: 1, EngineAPI: v})
		}
		return r
	}
	t.Run("labels in every plan", func(t *testing.T) {
		plans, err := Compile(req(t, "0.2.3", "0.3.0"))
		if err != nil {
			t.Fatal(err)
		}
		for _, p := range plans {
			if len(p.Criteria) != 2 || p.Criteria[0].Labels != nil {
				t.Fatalf("plan %s criteria %+v, want an unlabeled and a labeled criterion", p.PlanID, p.Criteria)
			}
			if got := p.Criteria[1].Labels; len(got) != 2 || got["application"] != "dns" || got["side"] != "server" {
				t.Fatalf("plan %s labels %v, want the where filter", p.PlanID, got)
			}
			data, err := p.MarshalCanonical()
			if err != nil || !strings.Contains(string(data), `"labels"`) {
				t.Fatalf("canonical plan lacks labels: %v\n%s", err, data)
			}
		}
	})
	t.Run("old agent is refused", func(t *testing.T) {
		_, err := Compile(req(t, "0.2.3", "0.2.2"))
		if !errors.Is(err, ErrAgentTooOld) {
			t.Fatalf("error %v, want ErrAgentTooOld", err)
		}
		for _, want := range []string{"criteria[1]", "agent b", "0.2.2", "0.2.3"} {
			if !strings.Contains(err.Error(), want) {
				t.Errorf("error %q lacks %q", err, want)
			}
		}
	})
	t.Run("unparsable version counts as old", func(t *testing.T) {
		if _, err := Compile(req(t, "garbage")); !errors.Is(err, ErrAgentTooOld) {
			t.Fatalf("error %v, want ErrAgentTooOld", err)
		}
	})
	t.Run("unknown version passes", func(t *testing.T) {
		if _, err := Compile(req(t, "")); err != nil {
			t.Fatal(err)
		}
	})
	t.Run("plain criteria need no version", func(t *testing.T) {
		r := req(t, "0.2.0")
		r.Config.Criteria = r.Config.Criteria[:1]
		plans, err := Compile(r)
		if err != nil {
			t.Fatal(err)
		}
		if plans[0].Criteria[0].Labels != nil {
			t.Fatalf("labels %v on a criterion without where", plans[0].Criteria[0].Labels)
		}
	})
}

func TestEngineAtLeast(t *testing.T) {
	for _, tc := range []struct {
		v    string
		want bool
	}{
		{"0.2.3", true}, {"0.2.10", true}, {"0.3.0", true}, {"1.0.0", true}, {"0.2.3-rc1", true},
		{"0.2.2", false}, {"0.1.9", false}, {"0.2", false}, {"", false}, {"x.y.z", false},
	} {
		if got := engineAtLeast(tc.v, LabelsEngineAPI); got != tc.want {
			t.Errorf("engineAtLeast(%q) = %v, want %v", tc.v, got, tc.want)
		}
	}
}
