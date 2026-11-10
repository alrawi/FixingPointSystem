// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"slices"
	"strings"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/metrics"
)

// ScenarioCount is one row of the scenarios table of the report: the sessions
// that the run started for one scenario and the payload bytes that it moved.
// The RLM-07 mix check reads the table, and its field names are the ones of
// the table form that the check accepts, {"scenarios": [{"scenario",
// "sessions", "bytes"}]}.
type ScenarioCount struct {
	// Scenario is <application>/<scenario>, the key of the scenario view of
	// the metrics.
	Scenario string `json:"scenario"`
	// Sessions is sessions.attempted.
	Sessions uint64 `json:"sessions"`
	// Bytes is bytes.tx plus bytes.rx.
	Bytes uint64 `json:"bytes"`
}

// scenarioTable returns the scenarios table of series, ordered by scenario.
// Like the totals of the report it counts the client side of every metric
// that has a side, because the two sides see the same session. A scenario
// appears when the run started at least one series of sessions.attempted for
// it. The table is nil when no series names a scenario, as for a report built
// from totals alone.
func scenarioTable(series []*metrics.Series) []ScenarioCount {
	rows := map[string]*ScenarioCount{}
	for _, s := range series {
		name := s.Metric.Name
		if name != "sessions.attempted" && name != "bytes.tx" && name != "bytes.rx" {
			continue
		}
		if side, ok := s.Label("side"); ok && side != metrics.SideClient {
			continue
		}
		app, ok1 := s.Label("application")
		sc, ok2 := s.Label("scenario")
		if !ok1 || !ok2 {
			continue
		}
		key := app + "/" + sc
		row := rows[key]
		if row == nil {
			row = &ScenarioCount{Scenario: key}
			rows[key] = row
		}
		if name == "sessions.attempted" {
			row.Sessions += s.Count
		} else {
			row.Bytes += s.Count
		}
	}
	var out []ScenarioCount
	for _, row := range rows {
		out = append(out, *row)
	}
	slices.SortFunc(out, func(a, b ScenarioCount) int { return strings.Compare(a.Scenario, b.Scenario) })
	return out
}

// DeclarationSource returns the arrival and size declarations of the test
// configuration with the given id, or nil when it declares none.
// ConfigObjectives satisfies it.
type DeclarationSource interface {
	Declarations(ctx context.Context, configID string) (*compiler.Declarations, error)
}

var _ DeclarationSource = ConfigObjectives{}

// Declarations parses the configuration configID and returns its arrival and
// size declarations, which are the targets of the realism check.
func (c ConfigObjectives) Declarations(ctx context.Context, configID string) (*compiler.Declarations, error) {
	rec, err := c.Store.Get(ctx, configID)
	if err != nil {
		return nil, fmt.Errorf("declarations of config %s: %w", configID, err)
	}
	cfg, err := compiler.ParseConfig(rec.Body)
	if err != nil {
		return nil, fmt.Errorf("declarations of config %s: %w", configID, err)
	}
	return cfg.Declarations(), nil
}

// UseDeclarations sets the source of the declarations that the report carries
// for realism targets. UseObjectives sets it too when its argument is a
// DeclarationSource. A run whose declarations the source cannot return gets a
// report without them. It returns a for chaining.
func (a *API) UseDeclarations(d DeclarationSource) *API {
	a.declarations = d
	return a
}

// fileReport is the report as a file holds it: the RunReport of the REST
// contract, which stays as it is, and beside it the tables that only the file
// and the internal structs carry. The REST report of GET /v1/runs/{id}/report
// is RunReport alone, and CTL-39 adds the scenario table to it.
type fileReport struct {
	api.RunReport
	// Scenarios is the scenarios table, left out when the report has none.
	Scenarios []ScenarioCount `json:"scenarios,omitempty"`
	// Declared holds the arrival and size declarations of the test, which are
	// the targets of the realism check, and is left out when it has none.
	Declared *compiler.Declarations `json:"declared,omitempty"`
}

// FileJSON renders the report as indented JSON for a file, in the layout of
// JSON with the scenarios table under "scenarios" and the declarations under
// "declared". The RLM-07 mix check reads the table form from this file.
func (r Report) FileJSON() ([]byte, error) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", "  ")
	f := fileReport{RunReport: r.API(), Scenarios: r.Scenarios, Declared: r.Declared}
	if err := enc.Encode(f); err != nil {
		return nil, fmt.Errorf("encode file report of run %s: %w", r.Run.ID, err)
	}
	return buf.Bytes(), nil
}
