// SPDX-License-Identifier: GPL-3.0-only

package runs

import (
	"fmt"
	"sort"
	"strconv"
	"strings"

	"github.com/alrawi/FixingPointSystem/controller/internal/compiler"
	"github.com/alrawi/FixingPointSystem/controller/internal/mixcontrol"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs/calibrate"
)

// profileBasisBytes is the value of profile.basis that asks for byte weights.
const profileBasisBytes = "bytes"

// attachByteMix adds the byte weighted mix control (CTL-12) to the calibration
// loop of a run whose test sets profile.basis to bytes. The run keeps exactly
// one loop that sends UpdateRates: the calibration loop. It feeds the samples
// of each window to mixcontrol as well, and multiplies the multiplier of each
// mix entry into the per entry total that it sends, so a window costs one
// command per agent whatever changed. A run with another basis, or with
// ByteMixOff, is untouched. Without a calibration (another objective, or a
// negative window) the run keeps its compiled rates, and the log says so.
func (o *Orchestrator) attachByteMix(id string, cfg compiler.Config, c *calibrator, plans []compiler.Plan, scenarios []compiler.Scenario) {
	if cfg.Profile.Basis != profileBasisBytes || o.cfg.Calibration.ByteMixOff {
		return
	}
	if c == nil {
		o.log.Warn("byte weighted mix control is off: it needs a throughput_bps objective and the calibration loop, so the mix keeps its compiled session weights", "run_id", id)
		return
	}
	mix, err := newByteMix(o.cfg.Calibration.MixTuning, plans, scenarios)
	if err != nil {
		o.log.Warn("byte weighted mix control is off", "run_id", id, "error", err)
		return
	}
	c.mix, c.mixOn = mix, true
	c.mult = mix.Multipliers()
	c.applied = map[string]uint64{}
	if o.cfg.Calibration.Window == 0 {
		// The catalog asks for a correction every second. The default window of
		// the calibration alone is longer, and the one loop takes the shorter
		// window, which both corrections tolerate (see mixcontrol).
		c.window = mixcontrol.DefaultWindow
	}
}

// newByteMix builds the mixcontrol state of a run from the compiled mix and the
// bytes per session estimates of the catalog. For the basis bytes the compiler
// has turned the byte weights of the test into session shares (weight over
// bytes per session), so the weight of a compiled entry is its session weight
// and the byte weight that mixcontrol converges to is that weight times the
// estimate, which gives back the configured byte weight. The run therefore
// starts at the compiled session shares, which are right when the estimates
// are, and mixcontrol corrects only what the measurements disagree with.
func newByteMix(tuning mixcontrol.Config, plans []compiler.Plan, scenarios []compiler.Scenario) (mixcontrol.State, error) {
	if len(plans) == 0 {
		return mixcontrol.State{}, fmt.Errorf("%w: no plans", mixcontrol.ErrConfig)
	}
	estimate := map[string]float64{}
	for _, s := range scenarios {
		estimate[s.Module.Name+"/"+s.Name] = s.BytesPerSession
	}
	tuning.Preserve = mixcontrol.PreserveBytes
	tuning.Entries = nil
	for _, m := range plans[0].Mix {
		bytes, ok := estimate[m.Scenario]
		if !ok {
			return mixcontrol.State{}, fmt.Errorf("mix entry %s: scenario %s is not in the catalog", m.ID, m.Scenario)
		}
		tuning.Entries = append(tuning.Entries, mixcontrol.Entry{ID: m.ID, ByteWeight: m.Weight * bytes, SessionWeight: m.Weight, ExpectedBytes: bytes})
	}
	return mixcontrol.New(tuning)
}

// updateByteMix folds the window into the byte mix control and reports whether
// a multiplier changed. It does nothing for a run without byte weighted mix
// control. Only the calibration goroutine calls it.
func (o *Orchestrator) updateByteMix(r *run, c *calibrator, samples []calibrate.Sample) bool {
	if !c.mixOn {
		return false
	}
	ms := make([]mixcontrol.Sample, len(samples))
	for i, s := range samples {
		ms[i] = mixcontrol.Sample{Entry: s.Entry, Bytes: s.Bytes, Sessions: s.Sessions}
	}
	next, res, err := c.mix.Update(o.cfg.Clock.Now(), ms)
	if err != nil {
		o.log.Warn("byte weighted mix control skipped a window", "run_id", r.ID, "error", err)
		return false
	}
	c.mix = next
	if !res.Changed {
		return false
	}
	c.mult = next.Multipliers()
	c.rev++
	o.log.Info("byte mix step", "run_id", r.ID, "max_error_pp", strconv.FormatFloat(res.MaxErrorPP, 'f', 2, 64),
		"multipliers", multipliers(res.Entries))
	return true
}

// multipliers words the multipliers of the mix entries for a log line.
func multipliers(es []mixcontrol.EntryResult) string {
	parts := make([]string, len(es))
	for i, e := range es {
		parts[i] = e.ID + "=" + strconv.FormatFloat(e.Multiplier, 'g', 4, 64)
	}
	sort.Strings(parts)
	return strings.Join(parts, ",")
}
