// SPDX-License-Identifier: GPL-3.0-only

package html

import (
	"fmt"
	"html/template"
	"math"
	"strconv"
	"strings"
)

// Geometry of a chart in SVG user units. The SVG scales to the width of its
// container through its viewBox.
const (
	chartW      = 640.0
	chartH      = 240.0
	marginLeft  = 64.0
	marginRight = 14.0
	marginTop   = 12.0
	marginBot   = 34.0
	// maxLines is the number of distinct line styles. A chart with more
	// series repeats the styles.
	maxLines = 4
)

// chartSVG draws c as an inline SVG, and returns the empty string when c has
// no finite point. All text is escaped, and every number is formatted here,
// so the result is safe to embed as template.HTML.
func chartSVG(c Chart) template.HTML {
	lines := make([][]Point, len(c.Series))
	var all []Point
	for i, s := range c.Series {
		for _, p := range s.Points {
			if finite(p.T) && finite(p.V) {
				lines[i] = append(lines[i], p)
				all = append(all, p)
			}
		}
	}
	if len(all) == 0 {
		return ""
	}

	minV, maxV, maxT := 0.0, 0.0, 0.0
	for _, p := range all {
		minV, maxV, maxT = math.Min(minV, p.V), math.Max(maxV, p.V), math.Max(maxT, p.T)
	}
	if maxV == minV {
		maxV = minV + 1
	}
	if maxT <= 0 {
		maxT = 1
	}
	step := niceStep((maxV - minV) / 4)
	lo := math.Floor(minV/step) * step
	hi := math.Ceil(maxV/step) * step

	plotW := chartW - marginLeft - marginRight
	plotH := chartH - marginTop - marginBot
	x := func(t float64) float64 { return marginLeft + t/maxT*plotW }
	y := func(v float64) float64 { return marginTop + (hi-v)/(hi-lo)*plotH }

	var b strings.Builder
	title := template.HTMLEscapeString(c.Title)
	fmt.Fprintf(&b, `<svg class="chart" viewBox="0 0 %g %g" role="img" aria-label="%s" preserveAspectRatio="xMidYMid meet">`, chartW, chartH, title)
	fmt.Fprintf(&b, `<title>%s</title>`, title)

	// Horizontal grid lines with value labels.
	for v := lo; v <= hi+step/2; v += step {
		fmt.Fprintf(&b, `<line class="grid" x1="%.1f" x2="%.1f" y1="%.1f" y2="%.1f"/>`, marginLeft, chartW-marginRight, y(v), y(v))
		fmt.Fprintf(&b, `<text class="tick" x="%.1f" y="%.1f" text-anchor="end">%s</text>`, marginLeft-6, y(v)+3.5, template.HTMLEscapeString(Compact(v)))
	}
	// Time axis, in seconds from the first sample.
	tstep := niceStep(maxT / 5)
	for t := 0.0; t <= maxT+tstep/2; t += tstep {
		fmt.Fprintf(&b, `<text class="tick" x="%.1f" y="%.1f" text-anchor="middle">%s</text>`, x(t), chartH-marginBot+16, template.HTMLEscapeString(Compact(t)))
	}
	fmt.Fprintf(&b, `<text class="tick" x="%.1f" y="%.1f" text-anchor="end">s</text>`, chartW-marginRight, chartH-4)
	fmt.Fprintf(&b, `<line class="axis" x1="%.1f" x2="%.1f" y1="%.1f" y2="%.1f"/>`, marginLeft, chartW-marginRight, y(lo), y(lo))
	fmt.Fprintf(&b, `<line class="axis" x1="%.1f" x2="%.1f" y1="%.1f" y2="%.1f"/>`, marginLeft, marginLeft, marginTop, y(lo))

	for i, pts := range lines {
		if len(pts) == 0 {
			continue
		}
		class := fmt.Sprintf("l%d", i%maxLines)
		if len(pts) == 1 {
			fmt.Fprintf(&b, `<circle class="dot %s" cx="%.1f" cy="%.1f" r="3"/>`, class, x(pts[0].T), y(pts[0].V))
			continue
		}
		var pp strings.Builder
		for j, p := range pts {
			if j > 0 {
				pp.WriteByte(' ')
			}
			fmt.Fprintf(&pp, "%.1f,%.1f", x(p.T), y(p.V))
		}
		fmt.Fprintf(&b, `<polyline class="line %s" points="%s"/>`, class, pp.String())
	}
	b.WriteString(`</svg>`)
	return template.HTML(b.String()) //nolint:gosec // built from escaped text and formatted numbers only
}

func finite(v float64) bool { return !math.IsNaN(v) && !math.IsInf(v, 0) }

// niceStep rounds a raw step up to 1, 2, 5, or 10 times a power of ten.
func niceStep(raw float64) float64 {
	if !(raw > 0) || !finite(raw) {
		return 1
	}
	exp := math.Floor(math.Log10(raw))
	base := math.Pow(10, exp)
	f := raw / base
	switch {
	case f <= 1:
		f = 1
	case f <= 2:
		f = 2
	case f <= 5:
		f = 5
	default:
		f = 10
	}
	return f * base
}

// Compact formats v with three significant digits and a metric suffix, such
// as 1.5k, 610M, or 0.005.
func Compact(v float64) string {
	if v == 0 || !finite(v) {
		if v == 0 {
			return "0"
		}
		return strconv.FormatFloat(v, 'g', -1, 64)
	}
	a := math.Abs(v)
	scales := []struct {
		min    float64
		suffix string
	}{{1e12, "T"}, {1e9, "G"}, {1e6, "M"}, {1e3, "k"}}
	for _, s := range scales {
		if a >= s.min {
			return sig3(v/s.min) + s.suffix
		}
	}
	if a >= 1 {
		return sig3(v)
	}
	return strconv.FormatFloat(v, 'g', 3, 64)
}

// sig3 rounds v to about three significant digits without an exponent and
// drops trailing zeros.
func sig3(v float64) string {
	a := math.Abs(v)
	dec := 0
	switch {
	case a < 10:
		dec = 2
	case a < 100:
		dec = 1
	}
	s := strconv.FormatFloat(v, 'f', dec, 64)
	if strings.Contains(s, ".") {
		s = strings.TrimRight(strings.TrimRight(s, "0"), ".")
	}
	return s
}
