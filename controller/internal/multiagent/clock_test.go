// SPDX-License-Identifier: GPL-3.0-only

package multiagent_test

import (
	"context"
	"errors"
	"fmt"
	"testing"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/multiagent"
)

// offsets is a ClockSource over a fixed table.
type offsets map[string]time.Duration

func (o offsets) ClockOffset(_ context.Context, id string) (time.Duration, error) {
	d, ok := o[id]
	if !ok {
		return 0, fmt.Errorf("agent %s: %w", id, multiagent.ErrNoClockOffset)
	}
	return d, nil
}

func TestCheckClocks(t *testing.T) {
	ms := time.Millisecond
	tests := []struct {
		name      string
		src       offsets
		ids       []string
		limit     time.Duration
		offenders []string
	}{
		{name: "all inside", src: offsets{"a": 0, "b": 3 * ms, "c": -4 * ms}, ids: []string{"a", "b", "c"}},
		{name: "exactly the limit passes", src: offsets{"a": 10 * ms, "b": -10 * ms}, ids: []string{"a", "b"}},
		{name: "one ahead above the limit", src: offsets{"a": 0, "b": 10*ms + time.Nanosecond}, ids: []string{"a", "b"}, offenders: []string{"b"}},
		{name: "one behind above the limit", src: offsets{"a": -11 * ms, "b": 0}, ids: []string{"a", "b"}, offenders: []string{"a"}},
		{name: "two offenders are listed by id", src: offsets{"z": 50 * ms, "a": -50 * ms, "m": 1 * ms}, ids: []string{"z", "m", "a"}, offenders: []string{"a", "z"}},
		{name: "custom limit", src: offsets{"a": 3 * ms}, ids: []string{"a"}, limit: 2 * ms, offenders: []string{"a"}},
		{name: "extreme negative offset", src: offsets{"a": time.Duration(-1 << 63)}, ids: []string{"a"}, offenders: []string{"a"}},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := multiagent.CheckClocks(context.Background(), tt.src, tt.ids, tt.limit)
			if len(tt.offenders) == 0 {
				if err != nil {
					t.Fatalf("CheckClocks: %v", err)
				}
				if len(got) != len(tt.ids) {
					t.Fatalf("got %d offsets, want %d", len(got), len(tt.ids))
				}
				return
			}
			if !errors.Is(err, multiagent.ErrClockOffset) {
				t.Fatalf("error %v does not match ErrClockOffset", err)
			}
			var ce *multiagent.ClockError
			if !errors.As(err, &ce) {
				t.Fatalf("error %v is not a *ClockError", err)
			}
			var ids []string
			for _, o := range ce.Offenders {
				ids = append(ids, o.AgentID)
			}
			if fmt.Sprint(ids) != fmt.Sprint(tt.offenders) {
				t.Fatalf("offenders %v, want %v", ids, tt.offenders)
			}
		})
	}
}

func TestCheckClocksDefaultLimitIsTenMilliseconds(t *testing.T) {
	if multiagent.MaxClockOffset != 10*time.Millisecond {
		t.Fatalf("MaxClockOffset is %s", multiagent.MaxClockOffset)
	}
	_, err := multiagent.CheckClocks(context.Background(), offsets{"a": 11 * time.Millisecond}, []string{"a"}, 0)
	if !errors.Is(err, multiagent.ErrClockOffset) {
		t.Fatalf("an offset of 11 ms passed the default limit: %v", err)
	}
}

func TestCheckClocksMissingMeasurementFailsClosed(t *testing.T) {
	_, err := multiagent.CheckClocks(context.Background(), offsets{"a": 0}, []string{"a", "b"}, 0)
	if !errors.Is(err, multiagent.ErrNoClockOffset) {
		t.Fatalf("error %v does not match ErrNoClockOffset", err)
	}
	if errors.Is(err, multiagent.ErrClockOffset) {
		t.Fatalf("a missing measurement must not look like an offset error: %v", err)
	}
}

func TestCheckClocksReturnsOffsetsSortedAndUnique(t *testing.T) {
	got, err := multiagent.CheckClocks(context.Background(),
		offsets{"b": time.Millisecond, "a": 2 * time.Millisecond}, []string{"b", "a", "b"}, 0)
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 2 || got[0].AgentID != "a" || got[1].AgentID != "b" || got[0].Offset != 2*time.Millisecond {
		t.Fatalf("offsets %+v", got)
	}
}
