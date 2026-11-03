// SPDX-License-Identifier: GPL-3.0-only

package metrics

import (
	"bytes"
	"compress/zlib"
	"encoding/binary"
	"encoding/json"
	"os"
	"sync"
	"testing"
	"time"

	"google.golang.org/protobuf/encoding/protojson"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents/agentpb"
)

// reference is testdata/reference_merge.json, which the Rust crate fps-stats
// wrote with Snapshot::merge_all. It holds three agent snapshots, their
// merge, and the merged histograms with their buckets and statistics.
type reference struct {
	Snapshots []struct {
		Agent    string          `json:"agent"`
		Snapshot json.RawMessage `json:"snapshot"`
	} `json:"snapshots"`
	Merged struct {
		Snapshot json.RawMessage `json:"snapshot"`
		Counters []struct {
			Metric string            `json:"metric"`
			Labels map[string]string `json:"labels"`
			Count  string            `json:"count"`
		} `json:"counters"`
		Histograms []refHist `json:"histograms"`
	} `json:"merged"`
}

type refHist struct {
	Metric  string            `json:"metric"`
	Labels  map[string]string `json:"labels"`
	Count   string            `json:"count"`
	Min     string            `json:"min"`
	Max     string            `json:"max"`
	Mean    float64           `json:"mean"`
	P50     string            `json:"p50"`
	P90     string            `json:"p90"`
	P99     string            `json:"p99"`
	P999    string            `json:"p999"`
	Buckets [][2]string       `json:"buckets"`
}

var (
	refOnce sync.Once
	refData reference
	refErr  error
)

func loadReference(t *testing.T) reference {
	t.Helper()
	refOnce.Do(func() {
		var raw []byte
		raw, refErr = os.ReadFile("testdata/reference_merge.json")
		if refErr == nil {
			refErr = json.Unmarshal(raw, &refData)
		}
	})
	if refErr != nil {
		t.Fatalf("load reference: %v", refErr)
	}
	return refData
}

func parseSnapshot(t *testing.T, raw []byte) *agentpb.CounterSnapshot {
	t.Helper()
	var snap agentpb.CounterSnapshot
	if err := protojson.Unmarshal(raw, &snap); err != nil {
		t.Fatalf("parse snapshot: %v", err)
	}
	return &snap
}

// encodeV2 writes counts in the uncompressed V2 encoding of a histogram with
// the given layout, using zero runs as writers do.
func encodeV2(lowest, highest uint64, sigfig uint32, counts []uint64) []byte {
	var payload []byte
	put := func(mag uint64, negative bool) {
		u := mag << 1
		if negative {
			u = (mag-1)<<1 | 1
		}
		for i := 0; i < 8; i++ {
			if u < 0x80 {
				payload = append(payload, byte(u))
				return
			}
			payload = append(payload, byte(u)|0x80)
			u >>= 7
		}
		payload = append(payload, byte(u))
	}
	for i := 0; i < len(counts); {
		if counts[i] != 0 {
			put(counts[i], false)
			i++
			continue
		}
		j := i
		for j < len(counts) && counts[j] == 0 {
			j++
		}
		if j-i == 1 {
			put(0, false)
		} else {
			put(uint64(j-i), true) //nolint:gosec // G115: j is above i
		}
		i = j
	}
	var b bytes.Buffer
	_ = binary.Write(&b, binary.BigEndian, uint32(cookieV2|0x10))
	_ = binary.Write(&b, binary.BigEndian, uint32(len(payload))) //nolint:gosec // G115: test payloads are small
	_ = binary.Write(&b, binary.BigEndian, uint32(0))
	_ = binary.Write(&b, binary.BigEndian, sigfig)
	_ = binary.Write(&b, binary.BigEndian, lowest)
	_ = binary.Write(&b, binary.BigEndian, highest)
	_ = binary.Write(&b, binary.BigEndian, float64(1))
	b.Write(payload)
	return b.Bytes()
}

// compress wraps an uncompressed V2 encoding in the compressed form.
func compress(raw []byte) []byte {
	var z bytes.Buffer
	w := zlib.NewWriter(&z)
	_, _ = w.Write(raw)
	_ = w.Close()
	var b bytes.Buffer
	_ = binary.Write(&b, binary.BigEndian, uint32(cookieV2Compressed|0x10))
	_ = binary.Write(&b, binary.BigEndian, uint32(z.Len())) //nolint:gosec // G115: test payloads are small
	b.Write(z.Bytes())
	return b.Bytes()
}

// record returns the compressed canonical encoding of values.
func record(values ...uint64) []byte {
	var counts []uint64
	for _, v := range values {
		i := canonical.index(v)
		for uint64(len(counts)) <= i {
			counts = append(counts, 0)
		}
		counts[i]++
	}
	return compress(encodeV2(CanonicalLowest, 3_600_000_000, CanonicalSigFig, counts))
}

// fakeClock is a settable clock.
type fakeClock struct {
	mu  sync.Mutex
	now time.Time
}

func (c *fakeClock) Now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.now
}

func (c *fakeClock) Add(d time.Duration) {
	c.mu.Lock()
	c.now = c.now.Add(d)
	c.mu.Unlock()
}

var t0 = time.Date(2026, 11, 3, 12, 0, 0, 0, time.UTC)

func newAgg(t *testing.T, mutate func(*Config)) *Aggregator {
	t.Helper()
	cfg := Config{Clock: &fakeClock{now: t0}}
	if mutate != nil {
		mutate(&cfg)
	}
	a, err := New(cfg)
	if err != nil {
		t.Fatal(err)
	}
	return a
}

func counter(metric string, n uint64, labels ...string) *agentpb.CounterValue {
	return &agentpb.CounterValue{Metric: metric, Labels: labelMap(labels), Value: &agentpb.CounterValue_Count{Count: n}}
}

func gauge(metric string, v float64, labels ...string) *agentpb.CounterValue {
	return &agentpb.CounterValue{Metric: metric, Labels: labelMap(labels), Value: &agentpb.CounterValue_Gauge{Gauge: v}}
}

func hist(metric string, enc []byte, labels ...string) *agentpb.HistogramValue {
	return &agentpb.HistogramValue{Metric: metric, Labels: labelMap(labels), HdrV2: enc}
}

func labelMap(kv []string) map[string]string {
	if len(kv) == 0 {
		return nil
	}
	m := map[string]string{}
	for i := 0; i+1 < len(kv); i += 2 {
		m[kv[i]] = kv[i+1]
	}
	return m
}

var webGet = []string{"application", "http1", "scenario", "web_get"}

func snapshot(plan string, seq uint64, cs []*agentpb.CounterValue, hs ...*agentpb.HistogramValue) *agentpb.CounterSnapshot {
	return &agentpb.CounterSnapshot{PlanId: plan, Sequence: seq, Aggregate: true, Counters: cs, Histograms: hs}
}
