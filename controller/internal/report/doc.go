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
// invalid when it names no catalog metric or operator. Failed and invalid
// criteria fail the run, and missing ones do not decide it.
//
// The package serves GET /v1/runs/{id}/report in JSON, which follows the
// RunReport schema of the REST contract, and in CSV, one table with a row per
// run field, criterion, and run total.
//
// The binary wires the package with two calls.
//
//	ev, _ := report.NewEvaluator(agg.Catalog())
//	// runs.Config{..., Evaluator: ev.RunEvaluator()}
//	ops := report.NewAPI(ev, orchestrator, agg) // embeds api.Unimplemented
package report
