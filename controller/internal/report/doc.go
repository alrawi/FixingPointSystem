// SPDX-License-Identifier: GPL-3.0-only

// Package report evaluates the pass and fail criteria of a run and serves its
// report. It implements CTL-09.
//
// An Evaluator reads the metric names of contracts/metrics/catalog.yaml. It
// recomputes every derived gauge of the catalog, such as
// sessions.failed_ratio, from the merged counters with its catalog formula,
// accepts histogram statistics under <metric>.<stat>, applies the six
// operators of contracts/schemas/test-config.schema.json, and gives every
// criterion one of four results. A criterion passes or fails by comparing the
// observed value, is missing when the run has no value for the metric, and is
// invalid when it names no catalog metric, operator, or label. Failed and
// invalid criteria fail the run, and missing ones do not decide it. Every
// result carries its reason.
//
// A criterion may select labeled series with its where filter. Without a side
// in the filter, a criterion on a metric that has the side label counts the
// client side only, because a total over both sides counts each session twice.
// The same evaluator decides the state of a run when it stops and builds the
// report, over the labeled series of the metrics aggregator.
//
// The package serves GET /v1/runs/{id}/report in JSON, which follows the
// RunReport schema of the REST contract, and in CSV, one table with a row per
// run field, criterion, and run total.
//
// The JSON report carries the scenarios of the run (CTL-39), one entry per
// application and scenario with the three session counts and both byte
// directions of the client side, and the label filter of each criterion as
// where. Its realism list holds the stored realism results of the run, which
// is empty until the controller stores any.
//
// The binary wires the package with two calls.
//
//	ev, _ := report.NewEvaluator(agg.Catalog())
//	// runs.Config{..., Evaluator: ev.RunEvaluator()}
//	ops := report.NewAPI(ev, orchestrator, agg) // embeds api.Unimplemented
package report
