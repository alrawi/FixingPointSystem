// SPDX-License-Identifier: GPL-3.0-only

package html

import "github.com/alrawi/FixingPointSystem/controller/internal/report"

var _ report.HTMLRenderer = Renderer

// Renderer renders rep as the HTML page, with the objective and the per
// second samples of in. It is the report.HTMLRenderer that
// report.API.UseHTML takes, and so serves GET /v1/runs/{id}/report?format=html.
func Renderer(rep report.Report, in report.HTMLInput) ([]byte, error) {
	o := Options{Samples: in.Samples}
	if in.Objective.Type != "" {
		o.Objective = FormatObjective(in.Objective.Type, in.Objective.Value)
	}
	return Render(FromReport(rep, o))
}
