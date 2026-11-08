// SPDX-License-Identifier: GPL-3.0-only

// Package html renders the report of a run as one self contained HTML page.
// It implements CTL-13.
//
// The page holds a summary of the run (id, state, reason, duration,
// objective, versions, agents), the verdict with every criterion, its
// observed value, its result and its reason, the realism results, and a line
// chart of each per second series of the run. The charts are inline SVG that
// this package draws, the style sheet is inline, and there is no script, font,
// image, or other resource from outside the page, so it opens from a file
// and from a network without access to the internet. The page follows
// prefers-color-scheme for light and dark and has a print style.
//
// The renderer works on a Page, which holds only strings and numbers.
// FromReport builds the Page from the report.Report of CTL-09 and the per
// second samples of the metrics aggregator, and Render writes it. Every string
// of the report passes through html/template, so markup in a criterion reason
// shows as text.
//
//	page := html.FromReport(rep, html.Options{Samples: samples, Objective: "sessions per second 1000"})
//	body, err := html.Render(page)
//
// The package does not serve HTTP. GET /v1/runs/{id}/report?format=html of
// the REST contract should call it from report.API.GetRunReport, with the
// samples of Aggregator.Samples(id, api.MetricViewApplication, Window{}) and
// the content type ContentType.
package html
