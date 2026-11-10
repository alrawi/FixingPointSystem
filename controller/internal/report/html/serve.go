// SPDX-License-Identifier: GPL-3.0-only

package html

import (
	"strconv"

	"github.com/alrawi/FixingPointSystem/controller/internal/report"
)

var _ report.HTMLRenderer = Renderer

// Renderer renders rep as the HTML page, with the objective and the per
// second samples of in and the realism results of rep. It is the report.HTMLRenderer that
// report.API.UseHTML takes, and so serves GET /v1/runs/{id}/report?format=html.
func Renderer(rep report.Report, in report.HTMLInput) ([]byte, error) {
	o := Options{Samples: in.Samples}
	if in.Objective.Type != "" {
		o.Objective = FormatObjective(in.Objective.Type, in.Objective.Value)
	}
	for _, r := range rep.Realism {
		o.Realism = append(o.Realism, Realism{
			Property: r.Property,
			Target:   r.Target,
			// The API holds the value as a 32 bit float, so the digits that
			// read back as that float are the ones to show.
			Observed: strconv.FormatFloat(float64(r.Observed), 'f', -1, 32),
			Passed:   r.Passed,
		})
	}
	return Render(FromReport(rep, o))
}
