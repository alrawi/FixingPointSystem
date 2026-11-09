// SPDX-License-Identifier: GPL-3.0-only

package fpscli

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"math"
	"strconv"
	"strings"
	"text/tabwriter"
)

// The types hold the fields that the text output reads. They decode only what
// they need, so a field that a later contract version adds does not break the
// client. The JSON output mode prints the controller's answer unchanged.

// ConfigRef is the answer of createConfig.
type ConfigRef struct {
	ID        string `json:"id"`
	Name      string `json:"name"`
	CreatedAt string `json:"created_at,omitempty"`
}

// ValidationReport is the answer of validateConfig.
type ValidationReport struct {
	Valid    bool      `json:"valid"`
	Findings []Finding `json:"findings"`
	Capacity *struct {
		AgentsRequired  *int `json:"agents_required"`
		AgentsAvailable *int `json:"agents_available"`
	} `json:"capacity"`
}

// CriterionResult is one criterion of a run verdict.
type CriterionResult struct {
	Metric   string          `json:"metric"`
	Op       string          `json:"op"`
	Value    json.RawMessage `json:"value"`
	Observed *float64        `json:"observed"`
	Passed   *bool           `json:"passed"`
	Reason   string          `json:"reason"`
}

// Verdict is the outcome of the criteria of a run.
type Verdict struct {
	Passed   *bool             `json:"passed"`
	Criteria []CriterionResult `json:"criteria"`
}

// Run is a run of the API contract.
type Run struct {
	ID            string             `json:"id"`
	ConfigID      string             `json:"config_id"`
	State         string             `json:"state"`
	Degraded      bool               `json:"degraded"`
	Reason        string             `json:"reason"`
	Phase         string             `json:"phase"`
	CreatedAt     string             `json:"created_at"`
	StartedAt     string             `json:"started_at"`
	EndedAt       string             `json:"ended_at"`
	Seed          *int64             `json:"seed"`
	EngineVersion string             `json:"engine_version"`
	Summary       map[string]float64 `json:"summary"`
	Verdict       *Verdict           `json:"verdict"`
}

// RunReport is the JSON report of a run.
type RunReport struct {
	Run     Run `json:"run"`
	Realism []struct {
		Property string  `json:"property"`
		Observed float64 `json:"observed"`
		Target   string  `json:"target"`
		Passed   bool    `json:"passed"`
	} `json:"realism"`
}

// MetricSample is one sample of a metrics view.
type MetricSample struct {
	Ts     string             `json:"ts"`
	View   string             `json:"view"`
	Key    string             `json:"key"`
	Values map[string]float64 `json:"values"`
}

// summaryOrder lists the keys of RunSummary in contract order.
var summaryOrder = []string{
	"sessions_attempted", "sessions_succeeded", "sessions_failed",
	"bytes_tx", "bytes_rx", "throughput_bps", "sessions_per_second", "concurrent_sessions",
}

// finalState reports whether state ends a run.
func finalState(state string) bool {
	switch state {
	case "completed", "failed", "aborted":
		return true
	}
	return false
}

// outcome maps the final state of a run to a nil error when the run completed
// and its verdict did not fail.
func (r *Run) outcome() error {
	switch {
	case r.State == "completed" && (r.Verdict == nil || r.Verdict.Passed == nil || *r.Verdict.Passed):
		return nil
	case r.State == "completed":
		return &outcomeError{msg: fmt.Sprintf("run %s completed but missed its criteria", r.ID)}
	case finalState(r.State):
		msg := fmt.Sprintf("run %s %s", r.ID, r.State)
		if r.Reason != "" {
			msg += ": " + r.Reason
		}
		return &outcomeError{msg: msg}
	}
	return nil
}

// fmtNum formats a metric value without noise: whole numbers print as
// integers and others with four significant digits.
func fmtNum(v float64) string {
	if math.IsNaN(v) || math.IsInf(v, 0) {
		return strconv.FormatFloat(v, 'g', -1, 64)
	}
	if v == math.Trunc(v) && math.Abs(v) < 1e15 {
		return strconv.FormatFloat(v, 'f', 0, 64)
	}
	return strconv.FormatFloat(v, 'g', 4, 64)
}

func dash(s string) string {
	if s == "" {
		return "-"
	}
	return s
}

func verdictWord(v *Verdict) string {
	switch {
	case v == nil || v.Passed == nil:
		return "-"
	case *v.Passed:
		return "passed"
	}
	return "failed"
}

func passWord(p *bool) string {
	switch {
	case p == nil:
		return "no result"
	case *p:
		return "pass"
	}
	return "fail"
}

func newTable(w io.Writer) *tabwriter.Writer {
	return tabwriter.NewWriter(w, 0, 4, 2, ' ', 0)
}

// writeRun prints the text view of a run.
func writeRun(w io.Writer, r *Run) {
	tw := newTable(w)
	row := func(k, v string) {
		if v != "" {
			_, _ = fmt.Fprintf(tw, "%s:\t%s\n", k, v)
		}
	}
	row("Run", r.ID)
	row("Config", r.ConfigID)
	row("State", r.State)
	row("Phase", r.Phase)
	if r.Degraded {
		row("Degraded", "yes, an agent failed during the run")
	}
	row("Reason", r.Reason)
	row("Created", r.CreatedAt)
	row("Started", r.StartedAt)
	row("Ended", r.EndedAt)
	if r.Seed != nil {
		row("Seed", strconv.FormatInt(*r.Seed, 10))
	}
	row("Engine", r.EngineVersion)
	if r.Verdict != nil && r.Verdict.Passed != nil {
		row("Verdict", verdictWord(r.Verdict))
	}
	_ = tw.Flush()
	if len(r.Summary) > 0 {
		_, _ = fmt.Fprintln(w, "\nTotals:")
		tw = newTable(w)
		for _, k := range summaryOrder {
			if v, ok := r.Summary[k]; ok {
				_, _ = fmt.Fprintf(tw, "  %s\t%s\n", k, fmtNum(v))
			}
		}
		_ = tw.Flush()
	}
	if r.Verdict != nil && len(r.Verdict.Criteria) > 0 {
		_, _ = fmt.Fprintln(w, "\nCriteria:")
		tw = newTable(w)
		_, _ = fmt.Fprintln(tw, "  RESULT\tCRITERION\tOBSERVED\tREASON")
		for _, c := range r.Verdict.Criteria {
			obs := "-"
			if c.Observed != nil {
				obs = fmtNum(*c.Observed)
			}
			crit := strings.TrimSpace(fmt.Sprintf("%s %s %s", c.Metric, c.Op, strings.TrimSpace(string(c.Value))))
			_, _ = fmt.Fprintf(tw, "  %s\t%s\t%s\t%s\n", passWord(c.Passed), crit, obs, c.Reason)
		}
		_ = tw.Flush()
	}
}

// writeRunTable prints one line per run.
func writeRunTable(w io.Writer, runs []Run) {
	tw := newTable(w)
	_, _ = fmt.Fprintln(tw, "RUN\tSTATE\tCONFIG\tSTARTED\tVERDICT")
	for _, r := range runs {
		_, _ = fmt.Fprintf(tw, "%s\t%s\t%s\t%s\t%s\n", r.ID, r.State, r.ConfigID, dash(r.StartedAt), verdictWord(r.Verdict))
	}
	_ = tw.Flush()
}

// printJSON prints data, indented, followed by a newline. Data that does not
// parse as JSON prints unchanged.
func printJSON(w io.Writer, data []byte) {
	var buf bytes.Buffer
	if err := json.Indent(&buf, bytes.TrimSpace(data), "", "  "); err != nil {
		_, _ = w.Write(data)
		return
	}
	buf.WriteByte('\n')
	_, _ = w.Write(buf.Bytes())
}

// printValue marshals v and prints it as printJSON does.
func printValue(w io.Writer, v any) error {
	data, err := json.Marshal(v)
	if err != nil {
		return err
	}
	printJSON(w, data)
	return nil
}
