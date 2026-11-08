// SPDX-License-Identifier: GPL-3.0-only

package html

import (
	"bytes"
	"encoding/json"
	"flag"
	"maps"
	"math"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strings"
	"testing"
)

var update = flag.Bool("update", false, "rewrite the golden HTML files")

// goldenRuns lists the JSON report goldens of CTL-09 that the HTML goldens
// render. The labeled golden holds filtered criteria and a run reason.
var goldenRuns = []string{"pass", "fail", "degraded", "labeled"}

// goldenReport is the part of the RunReport schema that the page shows. The
// test decodes the golden JSON files of internal/report with it, so the HTML
// goldens follow the report goldens.
type goldenReport struct {
	Realism []struct {
		Observed float64 `json:"observed"`
		Passed   bool    `json:"passed"`
		Property string  `json:"property"`
		Target   string  `json:"target"`
	} `json:"realism"`
	Run struct {
		ConfigID      string             `json:"config_id"`
		CreatedAt     string             `json:"created_at"`
		Degraded      bool               `json:"degraded"`
		EndedAt       string             `json:"ended_at"`
		EngineVersion string             `json:"engine_version"`
		ID            string             `json:"id"`
		ModuleVersion map[string]string  `json:"module_versions"`
		Phase         string             `json:"phase"`
		Reason        string             `json:"reason"`
		Seed          uint64             `json:"seed"`
		StartedAt     string             `json:"started_at"`
		State         string             `json:"state"`
		Summary       map[string]float64 `json:"summary"`
		Verdict       *struct {
			Criteria []struct {
				Metric   string   `json:"metric"`
				Observed *float64 `json:"observed"`
				Op       string   `json:"op"`
				Passed   *bool    `json:"passed"`
				Reason   string   `json:"reason"`
				Value    float64  `json:"value"`
			} `json:"criteria"`
			Passed bool `json:"passed"`
		} `json:"verdict"`
	} `json:"run"`
}

// pageOfGolden converts a golden report to a page the way FromReport does,
// and adds a deterministic ramp as the per second series, because the
// goldens hold no samples.
func pageOfGolden(t *testing.T, name string) Page {
	t.Helper()
	raw, err := os.ReadFile(filepath.Join("..", "testdata", name+".json"))
	if err != nil {
		t.Fatal(err)
	}
	var g goldenReport
	if err := json.Unmarshal(raw, &g); err != nil {
		t.Fatalf("decode %s: %v", name, err)
	}
	r := g.Run
	p := Page{
		RunID: r.ID, ConfigID: r.ConfigID, State: r.State, Phase: r.Phase,
		Degraded: r.Degraded, Reason: r.Reason, Created: r.CreatedAt,
		Started: r.StartedAt, Ended: r.EndedAt, Duration: "1m0s", Seed: r.Seed,
		Engine: r.EngineVersion, Objective: "sessions per second 1666",
		Verdict: VerdictNotEvaluated,
	}
	for _, k := range slices.Sorted(maps.Keys(r.ModuleVersion)) {
		p.Modules = append(p.Modules, KV{k, r.ModuleVersion[k]})
	}
	for _, k := range slices.Sorted(maps.Keys(r.Summary)) {
		p.Summary = append(p.Summary, KV{k, plain(r.Summary[k])})
	}
	if r.Verdict != nil {
		p.Verdict = VerdictFail
		if r.Verdict.Passed {
			p.Verdict = VerdictPass
		}
		for _, c := range r.Verdict.Criteria {
			cr := Criterion{Metric: c.Metric, Op: c.Op, Threshold: plain(c.Value), Reason: c.Reason}
			switch {
			case c.Observed != nil && c.Passed != nil && *c.Passed:
				cr.Status, cr.Observed = "pass", plain(*c.Observed)
			case c.Observed != nil:
				cr.Status, cr.Observed = "fail", plain(*c.Observed)
			case c.Passed != nil && !*c.Passed:
				cr.Status = "invalid"
			default:
				cr.Status = "missing"
			}
			p.Criteria = append(p.Criteria, cr)
		}
	}
	for _, rl := range g.Realism {
		p.Realism = append(p.Realism, Realism{rl.Property, rl.Target, plain(rl.Observed), rl.Passed})
	}
	ramp := Line{Name: "throughput.bps"}
	conc := Line{Name: "sessions.concurrent"}
	for i := 0; i <= 60; i++ {
		ramp.Points = append(ramp.Points, Point{T: float64(i), V: float64(i) * 1e7})
		conc.Points = append(conc.Points, Point{T: float64(i), V: math.Min(float64(i)*20, 640)})
	}
	p.Charts = []Chart{
		{Title: "Throughput", Unit: "bits per second", Series: []Line{ramp}},
		{Title: "Concurrent sessions", Unit: "sessions", Series: []Line{conc}},
	}
	return p
}

func TestGoldenPages(t *testing.T) {
	for _, name := range goldenRuns {
		t.Run(name, func(t *testing.T) {
			got, err := Render(pageOfGolden(t, name))
			if err != nil {
				t.Fatal(err)
			}
			path := filepath.Join("testdata", name+".html")
			if *update {
				if err := os.MkdirAll("testdata", 0o755); err != nil {
					t.Fatal(err)
				}
				if err := os.WriteFile(path, got, 0o644); err != nil {
					t.Fatal(err)
				}
			}
			want, err := os.ReadFile(path)
			if err != nil {
				t.Fatalf("%v (run the test with -update to write it)", err)
			}
			if !bytes.Equal(got, want) {
				t.Errorf("page of %s differs from %s; run the test with -update and review the diff", name, path)
			}
		})
	}
}

func TestGoldenContent(t *testing.T) {
	cases := map[string][]string{
		"pass":     {"Verdict: pass", "run-pass", "PASS"},
		"fail":     {"Verdict: fail", "criteria failed:", "FAIL", "MISSING", "sessions.response_time.p99", "87000"},
		"degraded": {"degraded", "Verdict: fail"},
		"labeled":  {"INVALID", "run-labeled"},
	}
	for name, wants := range cases {
		out, err := Render(pageOfGolden(t, name))
		if err != nil {
			t.Fatal(err)
		}
		for _, w := range wants {
			if !bytes.Contains(out, []byte(w)) {
				t.Errorf("page of %s lacks %q", name, w)
			}
		}
		if !bytes.Contains(out, []byte("<polyline")) {
			t.Errorf("page of %s has no chart line", name)
		}
	}
}

// violations lists the external references of an HTML document: an attribute
// that loads or links a resource over http, https, or a protocol relative URL,
// any script, any style sheet link, and any CSS import or url().
func violations(doc string) []string {
	var out []string
	tag := regexp.MustCompile(`(?is)<([a-z][a-z0-9-]*)((?:\s+[^<>]*?)?)\s*/?>`)
	attr := regexp.MustCompile(`(?is)([a-z_:][a-z0-9_:.-]*)\s*=\s*("[^"]*"|'[^']*'|[^\s"'>]+)`)
	external := regexp.MustCompile(`(?i)^\s*(https?:)?//|^\s*https?:`)
	for _, m := range tag.FindAllStringSubmatch(doc, -1) {
		name, rest := strings.ToLower(m[1]), m[2]
		attrs := map[string]string{}
		for _, a := range attr.FindAllStringSubmatch(rest, -1) {
			attrs[strings.ToLower(a[1])] = strings.Trim(a[2], `"'`)
		}
		for _, k := range []string{"src", "href", "srcset", "action", "poster", "data", "xlink:href", "formaction", "cite", "background"} {
			if v, ok := attrs[k]; ok && external.MatchString(v) {
				out = append(out, "<"+name+" "+k+"="+v+">")
			}
		}
		if name == "script" {
			out = append(out, "<script>")
		}
		if name == "link" && strings.Contains(strings.ToLower(attrs["rel"]), "stylesheet") {
			out = append(out, "<link rel=stylesheet>")
		}
		for _, bad := range []string{"img", "iframe", "object", "embed", "video", "audio", "source", "form"} {
			if name == bad {
				out = append(out, "<"+bad+">")
			}
		}
	}
	css := regexp.MustCompile(`(?i)@import|url\s*\(`)
	if css.MatchString(doc) {
		out = append(out, "css import or url()")
	}
	return out
}

func TestViolationsFindsExternalReferences(t *testing.T) {
	bad := `<html><head><script src="https://cdn.example/x.js"></script>
<link rel="stylesheet" href="//cdn.example/a.css"><link rel=stylesheet href=a.css>
<style>@import url(x.css);</style></head>
<body><a href='HTTP://evil.example/'>x</a><img src=http://e/x.png><iframe src="https://e/"></iframe></body></html>`
	got := violations(bad)
	for _, want := range []string{
		"<script>", "<link rel=stylesheet>", "<link href=//cdn.example/a.css>",
		"<a href=HTTP://evil.example/>", "<img src=http://e/x.png>", "<img>", "<iframe>", "css import or url()",
	} {
		if !slices.Contains(got, want) {
			t.Errorf("violations lacks %q in %q", want, got)
		}
	}
	if got := violations(`<a href="#criteria">x</a><svg viewBox="0 0 1 1"><line x1="0"/></svg>`); len(got) != 0 {
		t.Errorf("violations flags a local page: %q", got)
	}
}

func TestNoExternalRequests(t *testing.T) {
	for _, name := range goldenRuns {
		out, err := Render(pageOfGolden(t, name))
		if err != nil {
			t.Fatal(err)
		}
		if v := violations(string(out)); len(v) != 0 {
			t.Errorf("page of %s makes external requests: %q", name, v)
		}
		if bytes.Contains(out, []byte("http://")) || bytes.Contains(out, []byte("https://")) {
			t.Errorf("page of %s mentions an http URL", name)
		}
	}
}

func TestNoExternalRequestsWithHostileStrings(t *testing.T) {
	out, err := Render(hostilePage())
	if err != nil {
		t.Fatal(err)
	}
	if v := violations(string(out)); len(v) != 0 {
		t.Errorf("hostile strings made external requests: %q", v)
	}
}

const hostile = `<script src="https://evil.example/x.js"></script><img src=http://evil.example/p.png onerror=alert(1)><a href="javascript:alert(1)">x</a>`

func hostilePage() Page {
	return Page{
		Title: hostile, RunID: hostile, ConfigID: hostile, State: hostile, Phase: hostile,
		Reason: hostile, Engine: hostile, Objective: hostile, Verdict: hostile,
		Modules: []KV{{hostile, hostile}}, Agents: []string{hostile}, FailedAgents: []string{hostile},
		Summary: []KV{{hostile, hostile}},
		Criteria: []Criterion{{Metric: hostile, Where: hostile, Op: hostile, Threshold: hostile,
			Observed: hostile, Status: hostile, Reason: hostile}},
		Realism: []Realism{{hostile, hostile, hostile, true}},
		Charts: []Chart{{Title: hostile, Unit: hostile, Series: []Line{
			{Name: hostile, Points: []Point{{0, 1}, {1, 2}}},
			{Name: hostile, Points: []Point{{0, 2}, {1, 1}}},
		}}},
	}
}

func TestEscapesReportStrings(t *testing.T) {
	p := pageOfGolden(t, "fail")
	reason := `observed <b onmouseover="x()">1</b> & <script>alert(1)</script>`
	p.Criteria[0].Reason = reason
	out, err := Render(p)
	if err != nil {
		t.Fatal(err)
	}
	s := string(out)
	for _, bad := range []string{"<script>alert", `<b onmouseover`, "</b>"} {
		if strings.Contains(s, bad) {
			t.Errorf("page holds unescaped markup %q", bad)
		}
	}
	if !strings.Contains(s, "&lt;script&gt;alert(1)&lt;/script&gt;") {
		t.Error("the reason does not show as escaped text")
	}
	if !strings.Contains(s, "&amp;") {
		t.Error("the ampersand of the reason is not escaped")
	}
}

func TestEscapesEveryField(t *testing.T) {
	out, err := Render(hostilePage())
	if err != nil {
		t.Fatal(err)
	}
	s := string(out)
	// The only tags in the page are the ones of the template: no <img>, no
	// attribute that came from a string, and no script of any kind.
	for _, bad := range []string{"<script", "<img", "<a ", `href="javascript`, `" onerror`} {
		if strings.Contains(s, bad) {
			t.Errorf("page holds %q from a hostile string", bad)
		}
	}
	if n := strings.Count(s, "&lt;script"); n < 12 {
		t.Errorf("only %d hostile strings show as escaped text", n)
	}
}

func TestSelfContainedAndThemed(t *testing.T) {
	out, err := Render(pageOfGolden(t, "pass"))
	if err != nil {
		t.Fatal(err)
	}
	s := string(out)
	for _, want := range []string{
		"<!doctype html>", `<meta charset="utf-8">`, `name="viewport"`,
		"prefers-color-scheme: dark", "@media print", "<style>", "<svg",
	} {
		if !strings.Contains(s, want) {
			t.Errorf("page lacks %q", want)
		}
	}
	if strings.Contains(s, "<script") || strings.Contains(s, "<link") {
		t.Error("page has a script or link element")
	}
}

func TestRenderIsDeterministic(t *testing.T) {
	p := pageOfGolden(t, "labeled")
	a, err := Render(p)
	if err != nil {
		t.Fatal(err)
	}
	b, err := Render(p)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(a, b) {
		t.Error("two renders of one page differ")
	}
}

func TestEmptyPage(t *testing.T) {
	out, err := Render(Page{RunID: "run-x", Verdict: VerdictNotEvaluated})
	if err != nil {
		t.Fatal(err)
	}
	s := string(out)
	for _, want := range []string{"Run report run-x", "never reached its criteria", "no per second samples", "no realism results", "not recorded"} {
		if !strings.Contains(s, want) {
			t.Errorf("empty page lacks %q", want)
		}
	}
	if strings.Contains(s, "<svg") {
		t.Error("empty page draws a chart")
	}
}

func TestChartSVG(t *testing.T) {
	line := func(pts ...Point) Chart { return Chart{Title: "t", Series: []Line{{Name: "a", Points: pts}}} }
	if got := chartSVG(line()); got != "" {
		t.Errorf("chart without points = %q", got)
	}
	if got := chartSVG(line(Point{0, math.NaN()}, Point{1, math.Inf(1)})); got != "" {
		t.Errorf("chart of non finite points = %q", got)
	}
	one := string(chartSVG(line(Point{0, 5})))
	if !strings.Contains(one, "<circle") || strings.Contains(one, "<polyline") {
		t.Errorf("single point chart = %s", one)
	}
	two := string(chartSVG(line(Point{0, 0}, Point{1, 10}, Point{2, math.NaN()}, Point{3, 4})))
	if strings.Count(two, "<polyline") != 1 || !strings.Contains(two, `viewBox="0 0 640 240"`) {
		t.Errorf("line chart = %s", two)
	}
	if strings.Contains(two, "NaN") || strings.Contains(two, "Inf") {
		t.Errorf("chart prints a non finite number: %s", two)
	}
	flat := string(chartSVG(line(Point{0, 0}, Point{5, 0})))
	if !strings.Contains(flat, "<polyline") || strings.Contains(flat, "NaN") {
		t.Errorf("flat chart = %s", flat)
	}
	neg := string(chartSVG(line(Point{0, -4}, Point{1, 4})))
	if !strings.Contains(neg, "-4") {
		t.Errorf("negative chart lacks the -4 tick: %s", neg)
	}
	named := string(chartSVG(Chart{Title: `a"<b>`, Series: []Line{{Points: []Point{{0, 1}, {1, 2}}}}}))
	if strings.Contains(named, "<b>") || !strings.Contains(named, "&lt;b&gt;") {
		t.Errorf("chart title is not escaped: %s", named)
	}
}

func TestCompact(t *testing.T) {
	cases := map[float64]string{
		0: "0", 5: "5", 12.345: "12.3", 999: "999", 1500: "1.5k", 610e6: "610M",
		1e9: "1G", 2.5e12: "2.5T", 0.005: "0.005", -1500: "-1.5k", 0.5: "0.5",
	}
	for in, want := range cases {
		if got := Compact(in); got != want {
			t.Errorf("Compact(%v) = %q, want %q", in, got, want)
		}
	}
}

func TestNiceStep(t *testing.T) {
	cases := map[float64]float64{0: 1, -3: 1, 0.7: 1, 1.1: 2, 3: 5, 7: 10, 160: 200, 0.03: 0.05}
	for in, want := range cases {
		if got := niceStep(in); math.Abs(got-want) > 1e-12 {
			t.Errorf("niceStep(%v) = %v, want %v", in, got, want)
		}
	}
}
