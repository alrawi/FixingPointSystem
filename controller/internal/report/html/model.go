// SPDX-License-Identifier: GPL-3.0-only

package html

// ContentType is the media type of the page.
const ContentType = "text/html; charset=utf-8"

// Verdict values of a Page.
const (
	VerdictPass         = "pass"
	VerdictFail         = "fail"
	VerdictNotEvaluated = "not evaluated"
)

// Page is everything that the template shows. It holds text and numbers only,
// so a renderer test needs no controller state.
type Page struct {
	// Title heads the page. Empty means "Run report <RunID>".
	Title string

	RunID    string
	ConfigID string
	State    string
	Phase    string
	Degraded bool
	// Reason says in words why the run failed or was aborted.
	Reason string
	// Created, Started, and Ended are RFC 3339 times in UTC, or empty.
	Created, Started, Ended string
	// Duration is the time from start to end, or empty.
	Duration string
	Seed     uint64
	Engine   string
	// Objective describes the load objective, for example
	// "sessions per second 1000". Empty shows that the report has none.
	Objective string
	// Modules holds the module versions, sorted by name.
	Modules      []KV
	Agents       []string
	FailedAgents []string
	// Backends names the backend that each agent ran, and is empty while no
	// agent reported one.
	Backends []Backend
	// Verdict is VerdictPass, VerdictFail, or VerdictNotEvaluated.
	Verdict string

	// Summary holds the run totals that the report shows.
	Summary  []KV
	Criteria []Criterion
	Realism  []Realism
	Charts   []Chart
}

// KV is one labeled value.
type KV struct{ Name, Value string }

// Backend is the backend that one agent ran for the run.
type Backend struct {
	Agent string
	// Backend is kernel, af_xdp, or dpdk, and "unknown" for an agent that
	// reported none, which is never shown as the kernel backend.
	Backend string
	// Selected is the backend that the inventory listed at the start of the
	// run, or empty.
	Selected string
	// Mismatch is true when Backend and Selected are both known and differ.
	Mismatch bool
	// Ports holds the XDP mode of each port that reported one.
	Ports []KV
}

// Criterion is one row of the criteria table.
type Criterion struct {
	Metric string
	// Where is the label filter, such as "application=dns", or empty.
	Where string
	Op    string
	// Threshold is the value that the rule compares with.
	Threshold string
	// Observed is empty when the criterion compared no value.
	Observed string
	// Status is pass, fail, missing, or invalid.
	Status string
	Reason string
}

// Realism is one realism result.
type Realism struct {
	Property string
	Target   string
	Observed string
	Passed   bool
}

// Chart is a line chart of per second values.
type Chart struct {
	Title string
	// Unit names the unit of the values, as the catalog does.
	Unit   string
	Series []Line
}

// Line is one series of a chart.
type Line struct {
	Name   string
	Points []Point
}

// Point is one sample, T seconds after the first sample of the run.
type Point struct{ T, V float64 }
