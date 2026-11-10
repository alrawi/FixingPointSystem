// SPDX-License-Identifier: GPL-3.0-only

package report

import (
	"bytes"
	_ "embed" // the realism report schema
	"encoding/json"
	"errors"
	"fmt"
	"maps"
	"math"
	"slices"
	"sort"
	"strings"
	"sync"

	"github.com/santhosh-tekuri/jsonschema/v6"
	"github.com/santhosh-tekuri/jsonschema/v6/kind"
	"golang.org/x/text/language"
	"golang.org/x/text/message"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
)

//go:generate cp ../../../realism/schema/realism-report.schema.json schema/realism-report.schema.json

// realismSchema is a copy of realism/schema/realism-report.schema.json, which
// go generate refreshes and a test compares with the source.
//
//go:embed schema/realism-report.schema.json
var realismSchema []byte

// RealismSchemaURL is the $id of the realism report schema.
const RealismSchemaURL = "https://github.com/alrawi/FixingPointSystem/realism/schema/realism-report.schema.json"

// MaxRealismBytes bounds the body of attachRealismReport, as CON-22 sets it.
const MaxRealismBytes = 4 << 20

// Check kinds and metric names that the mapping of CON-22 treats apart.
const (
	checkConformance = "conformance"
	checkTemporal    = "temporal"

	metricConformanceRatio = "realism.conformance_ratio"
	metricKSPValue         = "realism.ks_p_value"
	metricKSStatistic      = "realism.ks_statistic"

	// targetCheckPassed is the target of every metric that has no rule of
	// its own, whose outcome is the outcome of its check.
	targetCheckPassed = "check passed"
)

// realismValidator checks realism reports against the embedded schema. It
// compiles the schema on first use, so a handler that never receives a report
// pays nothing.
type realismValidator struct {
	once    sync.Once
	sch     *jsonschema.Schema
	printer *message.Printer
	err     error
}

func (v *realismValidator) compile() {
	doc, err := jsonschema.UnmarshalJSON(bytes.NewReader(realismSchema))
	if err != nil {
		v.err = fmt.Errorf("parse realism report schema: %w", err)
		return
	}
	c := jsonschema.NewCompiler()
	if err := c.AddResource(RealismSchemaURL, doc); err != nil {
		v.err = fmt.Errorf("add realism report schema: %w", err)
		return
	}
	if v.sch, err = c.Compile(RealismSchemaURL); err != nil {
		v.err = fmt.Errorf("compile realism report schema: %w", err)
		return
	}
	v.printer = message.NewPrinter(language.English)
}

// check validates the JSON document doc and returns one error finding per
// failing field, sorted by path. An empty result means that the document
// conforms. The caller has checked that doc is well formed JSON.
func (v *realismValidator) check(doc []byte) ([]api.Finding, error) {
	v.once.Do(v.compile)
	if v.err != nil {
		return nil, v.err
	}
	inst, err := jsonschema.UnmarshalJSON(bytes.NewReader(doc))
	if err != nil {
		return nil, fmt.Errorf("read realism report: %w", err)
	}
	err = v.sch.Validate(inst)
	if err == nil {
		return nil, nil
	}
	var ve *jsonschema.ValidationError
	if !errors.As(err, &ve) {
		return nil, fmt.Errorf("validate realism report: %w", err)
	}
	var out []api.Finding
	v.leaves(ve, &out)
	return sortFindings(out), nil
}

// leaves appends a finding for each leaf of the error tree. A missing or an
// unknown property becomes a finding at the property itself, so the path
// names the field rather than its parent object.
func (v *realismValidator) leaves(ve *jsonschema.ValidationError, out *[]api.Finding) {
	if len(ve.Causes) > 0 {
		for _, c := range ve.Causes {
			v.leaves(c, out)
		}
		return
	}
	at := pointer(ve.InstanceLocation)
	switch k := ve.ErrorKind.(type) {
	case *kind.Required:
		for _, name := range k.Missing {
			*out = append(*out, errorFinding(at+"/"+escapeToken(name), "required property is missing"))
		}
	case *kind.AdditionalProperties:
		for _, name := range k.Properties {
			*out = append(*out, errorFinding(at+"/"+escapeToken(name), "property is not allowed here"))
		}
	default:
		*out = append(*out, errorFinding(at, ve.ErrorKind.LocalizedString(v.printer)))
	}
}

// pointer returns the JSON Pointer of RFC 6901 for tokens.
func pointer(tokens []string) string {
	var sb strings.Builder
	for _, t := range tokens {
		sb.WriteByte('/')
		sb.WriteString(escapeToken(t))
	}
	return sb.String()
}

func escapeToken(t string) string {
	return strings.ReplaceAll(strings.ReplaceAll(t, "~", "~0"), "/", "~1")
}

func errorFinding(path, msg string) api.Finding {
	return api.Finding{Severity: api.FindingSeverityError, Path: path, Message: msg}
}

// sortFindings orders findings by path, then severity and message, and drops
// exact duplicates, so answers are stable.
func sortFindings(in []api.Finding) []api.Finding {
	sort.SliceStable(in, func(i, j int) bool {
		a, b := in[i], in[j]
		if a.Path != b.Path {
			return a.Path < b.Path
		}
		if a.Severity != b.Severity {
			return a.Severity < b.Severity
		}
		return a.Message < b.Message
	})
	out := in[:0]
	for _, f := range in {
		if len(out) > 0 && f == out[len(out)-1] {
			continue
		}
		out = append(out, f)
	}
	return out
}

// realismMetric is one metric of a check, as the report schema has it.
type realismMetric struct {
	Name   string            `json:"name"`
	Labels map[string]string `json:"labels"`
	Value  float64           `json:"value"`
}

// realismHead holds the fields that every check carries.
type realismHead struct {
	Check   string          `json:"check"`
	Passed  bool            `json:"passed"`
	Metrics []realismMetric `json:"metrics"`
}

// conformanceCheck holds the fields of a conformance check that the mapping
// reads.
type conformanceCheck struct {
	Threshold    float64 `json:"threshold"`
	Applications []struct {
		Application string `json:"application"`
		Passed      bool   `json:"passed"`
	} `json:"applications"`
}

// temporalCheck holds the fields of a temporal check that the mapping reads.
type temporalCheck struct {
	Comparisons []struct {
		Property    string  `json:"property"`
		Application *string `json:"application"`
		Direction   *string `json:"direction"`
		Alpha       float64 `json:"alpha"`
		Status      string  `json:"status"`
	} `json:"comparisons"`
}

// MapRealism returns the RealismResult entries of the realism report doc, which
// the caller has validated against the report schema. It follows the mapping
// of contracts/changes/CON-22.md: one entry for each metric of each check, in
// the order of the document, with the target and the outcome that the check
// gives the metric. The findings name results that the API cannot hold, with
// JSON Pointers into doc: a metric and labels pair that repeats, and a value
// that a 32 bit float cannot represent. A document with findings gives no
// results.
func MapRealism(doc []byte) ([]api.RealismResult, []api.Finding, error) {
	var top struct {
		Checks []json.RawMessage `json:"checks"`
	}
	if err := json.Unmarshal(doc, &top); err != nil {
		return nil, nil, fmt.Errorf("read realism report: %w", err)
	}
	results := []api.RealismResult{}
	var findings []api.Finding
	seen := map[string]string{}
	for i, raw := range top.Checks {
		var head realismHead
		if err := json.Unmarshal(raw, &head); err != nil {
			return nil, nil, fmt.Errorf("read check %d of the realism report: %w", i, err)
		}
		rule, err := checkRule(head, raw)
		if err != nil {
			return nil, nil, fmt.Errorf("read check %d of the realism report: %w", i, err)
		}
		for j, m := range head.Metrics {
			at := fmt.Sprintf("/checks/%d/metrics/%d", i, j)
			key := metricKey(m)
			if first, dup := seen[key]; dup {
				findings = append(findings, errorFinding(at, "the metric and labels repeat those of "+first))
				continue
			}
			seen[key] = at
			if v := float32(m.Value); math.IsInf(float64(v), 0) {
				findings = append(findings, errorFinding(at+"/value", "the value does not fit a 32 bit float"))
				continue
			}
			results = append(results, rule(m))
		}
	}
	if len(findings) > 0 {
		return nil, sortFindings(findings), nil
	}
	return results, nil, nil
}

// metricKey identifies a metric by its name and labels, which CON-21 says are
// unique within one report.
func metricKey(m realismMetric) string {
	var sb strings.Builder
	sb.WriteString(m.Name)
	for _, k := range slices.Sorted(maps.Keys(m.Labels)) {
		sb.WriteByte(0)
		sb.WriteString(k)
		sb.WriteByte('=')
		sb.WriteString(m.Labels[k])
	}
	return sb.String()
}

// checkRule returns the function that builds the result of a metric of the
// check head, whose JSON is raw. A metric that the check kind gives no rule,
// and a metric that finds no entry to take its outcome from, gets the target
// `check passed` and the outcome of the check, which is the rule of CON-22
// for every check kind that follows the first two.
func checkRule(head realismHead, raw json.RawMessage) (func(realismMetric) api.RealismResult, error) {
	generic := func(m realismMetric) api.RealismResult {
		return newResult(m, genericProperty(m), targetCheckPassed, head.Passed)
	}
	switch head.Check {
	case checkConformance:
		var c conformanceCheck
		if err := json.Unmarshal(raw, &c); err != nil {
			return nil, err
		}
		return func(m realismMetric) api.RealismResult {
			if m.Name != metricConformanceRatio {
				return generic(m)
			}
			for _, a := range c.Applications {
				if a.Application == m.Labels["application"] {
					return newResult(m, conformanceProperty(m), ">= "+formatNumber(c.Threshold), a.Passed)
				}
			}
			return generic(m)
		}, nil
	case checkTemporal:
		var c temporalCheck
		if err := json.Unmarshal(raw, &c); err != nil {
			return nil, err
		}
		return func(m realismMetric) api.RealismResult {
			if m.Name != metricKSPValue && m.Name != metricKSStatistic {
				return generic(m)
			}
			for _, cmp := range c.Comparisons {
				if cmp.Application != nil && *cmp.Application == m.Labels["application"] &&
					comparisonProperty(cmp.Property, cmp.Direction) == m.Labels["property"] {
					return newResult(m, ksProperty(m), "p value >= "+formatNumber(cmp.Alpha), cmp.Status == "pass")
				}
			}
			return generic(m)
		}, nil
	}
	return generic, nil
}

// comparisonProperty returns the value of the property label that names a
// comparison of the temporal check: inter_arrival, request_bytes for the
// payloads of the client, and response_bytes for those of the server. The
// comparisons over both directions have no label and give "".
func comparisonProperty(property string, direction *string) string {
	switch {
	case property == "inter_arrival":
		return "inter_arrival"
	case property == "payload_size" && direction != nil && *direction == "client":
		return "request_bytes"
	case property == "payload_size" && direction != nil && *direction == "server":
		return "response_bytes"
	}
	return ""
}

// newResult builds the entry of metric m.
func newResult(m realismMetric, property, target string, passed bool) api.RealismResult {
	name := m.Name
	res := api.RealismResult{
		Metric:   &name,
		Observed: float32(m.Value),
		Passed:   passed,
		Property: property,
		Target:   target,
	}
	if len(m.Labels) > 0 {
		res.Labels = ptr(maps.Clone(m.Labels))
	}
	return res
}

// conformanceProperty words a conformance ratio, for example
// `conformance of http1 by ndpi`.
func conformanceProperty(m realismMetric) string {
	s := "conformance of " + m.Labels["application"]
	if c := m.Labels["classifier"]; c != "" {
		s += " by " + c
	}
	return s
}

// ksProperty words a KS metric, for example `inter arrival of http1, KS p value`.
func ksProperty(m realismMetric) string {
	what := "KS p value"
	if m.Name == metricKSStatistic {
		what = "KS statistic"
	}
	return strings.ReplaceAll(m.Labels["property"], "_", " ") + " of " + m.Labels["application"] + ", " + what
}

// genericProperty words a metric of a check kind without a rule of its own,
// as its name and its labels in key order.
func genericProperty(m realismMetric) string {
	if len(m.Labels) == 0 {
		return m.Name
	}
	parts := make([]string, 0, len(m.Labels))
	for _, k := range slices.Sorted(maps.Keys(m.Labels)) {
		parts = append(parts, k+" "+m.Labels[k])
	}
	return m.Name + " of " + strings.Join(parts, ", ")
}
