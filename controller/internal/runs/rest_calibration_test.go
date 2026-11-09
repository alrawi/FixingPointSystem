// SPDX-License-Identifier: GPL-3.0-only

package runs_test

import (
	"encoding/json"
	"reflect"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/runs"
	"github.com/alrawi/FixingPointSystem/controller/internal/runs/calibrate"
)

// TestToAPICalibration checks the JSON of the calibration array against the
// property names of CalibrationStep in the REST contract (CON-14).
func TestToAPICalibration(t *testing.T) {
	at := time.Date(2026, 11, 9, 12, 0, 0, 0, time.UTC)
	run := runs.Run{ID: "r1", ConfigID: "c1", State: runs.Running, Calibration: []runs.CalibrationStep{
		{Time: at, Factor: 1.5, Target: 2, SessionsPerSecond: 300,
			Entries:     []calibrate.EntryResult{{ID: "web", BytesPerSession: 4096, Measured: true}},
			Multipliers: map[string]float64{"web": 1.25}},
		{Time: at, Factor: 1.5, Target: 1.5, SessionsPerSecond: 300},
	}}
	raw, err := json.Marshal(runs.ToAPI(run))
	if err != nil {
		t.Fatal(err)
	}
	var got struct {
		Calibration []map[string]any `json:"calibration"`
	}
	if err := json.Unmarshal(raw, &got); err != nil {
		t.Fatal(err)
	}
	want := []map[string]any{
		{"time": "2026-11-09T12:00:00Z", "factor": 1.5, "target": 2.0, "sessions_per_second": 300.0,
			"entries":     []any{map[string]any{"id": "web", "bytes_per_session": 4096.0, "measured": true}},
			"multipliers": map[string]any{"web": 1.25}},
		{"time": "2026-11-09T12:00:00Z", "factor": 1.5, "target": 1.5, "sessions_per_second": 300.0},
	}
	if !reflect.DeepEqual(got.Calibration, want) {
		t.Errorf("calibration = %v, want %v", got.Calibration, want)
	}
	raw, _ = json.Marshal(runs.ToAPI(runs.Run{ID: "r2", ConfigID: "c1", State: runs.Running}))
	var bare map[string]any
	_ = json.Unmarshal(raw, &bare)
	if _, ok := bare["calibration"]; ok {
		t.Errorf("run without steps has calibration %v", bare["calibration"])
	}
}
