// SPDX-License-Identifier: GPL-3.0-only

package html

import (
	"bytes"
	_ "embed"
	"fmt"
	"html/template"
	"math"
	"strconv"
	"strings"
)

//go:embed page.tmpl
var pageSource string

// pageTemplate is parsed once. html/template escapes every action by its
// context, so report strings never become markup.
var pageTemplate = template.Must(template.New("report").Parse(pageSource))

// view is the data of the template: the page with the parts that the
// template cannot compute.
type view struct {
	Page
	VerdictClass string
	Criteria     []criterionView
	ChartViews   []chartView
}

type criterionView struct {
	Criterion
	StatusLabel string
}

type chartView struct {
	Title  string
	Unit   string
	SVG    template.HTML
	Legend []legendItem
	Stats  string
}

type legendItem struct{ Class, Name string }

// Render writes the page as one HTML document. The document holds its own
// style sheet and charts and refers to no other resource.
func Render(p Page) ([]byte, error) {
	if p.Title == "" {
		p.Title = "Run report " + p.RunID
	}
	v := view{Page: p, VerdictClass: verdictClass(p.Verdict)}
	for _, c := range p.Criteria {
		v.Criteria = append(v.Criteria, criterionView{Criterion: c, StatusLabel: strings.ToUpper(c.Status)})
	}
	for _, c := range p.Charts {
		svg := chartSVG(c)
		if svg == "" {
			continue
		}
		cv := chartView{Title: c.Title, Unit: c.Unit, SVG: svg, Stats: chartStats(c)}
		for i, s := range c.Series {
			cv.Legend = append(cv.Legend, legendItem{Class: fmt.Sprintf("l%d", i%maxLines), Name: s.Name})
		}
		v.ChartViews = append(v.ChartViews, cv)
	}
	var buf bytes.Buffer
	if err := pageTemplate.Execute(&buf, v); err != nil {
		return nil, fmt.Errorf("render report of run %s: %w", p.RunID, err)
	}
	return buf.Bytes(), nil
}

// plain formats v in plain decimal notation, as the CSV report does.
func plain(v float64) string { return strconv.FormatFloat(v, 'f', -1, 64) }

func verdictClass(verdict string) string {
	switch verdict {
	case VerdictPass:
		return "pass"
	case VerdictFail:
		return "fail"
	}
	return "none"
}

// chartStats summarizes the finite points of every series of c as text, so
// the numbers of a chart are readable without the drawing.
func chartStats(c Chart) string {
	var parts []string
	for _, s := range c.Series {
		n, lo, hi, last := 0, math.Inf(1), math.Inf(-1), 0.0
		for _, p := range s.Points {
			if !finite(p.V) || !finite(p.T) {
				continue
			}
			n++
			lo, hi, last = math.Min(lo, p.V), math.Max(hi, p.V), p.V
		}
		if n == 0 {
			continue
		}
		name := ""
		if len(c.Series) > 1 {
			name = s.Name + ": "
		}
		parts = append(parts, fmt.Sprintf("%smin %s, max %s, last %s, %d samples", name, Compact(lo), Compact(hi), Compact(last), n))
	}
	return strings.Join(parts, "; ")
}
