// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"math"
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

// scenarioRow sums the client side counters of one scenario of one application.
type scenarioRow struct {
	application, scenario string
	attempted, succeeded  uint64
	failed                uint64 // over every reason
	bytesTx, bytesRx      uint64
}

// scenarioRows returns the rows of the scenarios of series, ordered by
// application and then by scenario. Like the totals of the report it counts the
// client side of every metric that has a side, because the two sides see the
// same session, and it adds the sessions.failed series of every reason. A
// scenario appears when the run recorded at least one of the five counters for
// it, and a counter that the run did not record is 0. The result is nil when no
// series names a scenario, as for a report built from totals alone.
func scenarioRows(series []*metrics.Series) []*scenarioRow {
	byName := map[[2]string]*scenarioRow{}
	for _, s := range series {
		name := s.Metric.Name
		switch name {
		case "sessions.attempted", "sessions.succeeded", "sessions.failed", "bytes.tx", "bytes.rx":
		default:
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
		key := [2]string{app, sc}
		row := byName[key]
		if row == nil {
			row = &scenarioRow{application: app, scenario: sc}
			byName[key] = row
		}
		switch name {
		case "sessions.attempted":
			row.attempted += s.Count
		case "sessions.succeeded":
			row.succeeded += s.Count
		case "sessions.failed":
			row.failed += s.Count
		case "bytes.tx":
			row.bytesTx += s.Count
		case "bytes.rx":
			row.bytesRx += s.Count
		}
	}
	var out []*scenarioRow
	for _, row := range byName {
		out = append(out, row)
	}
	slices.SortFunc(out, func(a, b *scenarioRow) int {
		if c := strings.Compare(a.application, b.application); c != 0 {
			return c
		}
		return strings.Compare(a.scenario, b.scenario)
	})
	return out
}

// scenarioTable returns the scenarios table of the file and CSV report from
// rows, ordered by scenario: the sessions are sessions.attempted and the bytes
// are bytes.tx plus bytes.rx.
func scenarioTable(rows []*scenarioRow) []ScenarioCount {
	var out []ScenarioCount
	for _, row := range rows {
		out = append(out, ScenarioCount{
			Scenario: row.application + "/" + row.scenario,
			Sessions: row.attempted,
			Bytes:    row.bytesTx + row.bytesRx,
		})
	}
	slices.SortFunc(out, func(a, b ScenarioCount) int { return strings.Compare(a.Scenario, b.Scenario) })
	return out
}

// scenarioTotals returns the scenarios of the REST report from rows, in their
// order, which is the order of the contract: by application and then by
// scenario name. The sum of a property over all entries is the property of the
// run summary, because both count the client side only.
func scenarioTotals(rows []*scenarioRow) []api.ScenarioTotals {
	var out []api.ScenarioTotals
	for _, row := range rows {
		out = append(out, api.ScenarioTotals{
			Application:       row.application,
			Scenario:          row.scenario,
			SessionsAttempted: clampInt(row.attempted),
			SessionsSucceeded: clampInt(row.succeeded),
			SessionsFailed:    clampInt(row.failed),
			BytesTx:           clampInt(row.bytesTx),
			BytesRx:           clampInt(row.bytesRx),
		})
	}
	return out
}

// clampInt converts a counter to the int of the REST schema, and a counter
// that no int holds to the largest int.
func clampInt(v uint64) int {
	if v > math.MaxInt {
		return math.MaxInt
	}
	return int(v)
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
// is RunReport alone, which carries its own scenarios in the form of the
// contract (CTL-39). The table of the file shadows that property, because the
// outer field of an embedding struct wins in encoding/json, and so the file
// keeps the form that the RLM-07 mix check reads.
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
