// SPDX-License-Identifier: GPL-3.0-only

package metrics

import (
	"encoding/binary"
	"errors"
	"math"
	"math/rand/v2"
	"slices"
	"strconv"
	"testing"
)

func parseU(t *testing.T, s string) uint64 {
	t.Helper()
	v, err := strconv.ParseUint(s, 10, 64)
	if err != nil {
		t.Fatal(err)
	}
	return v
}

// checkAgainst compares h with a merged histogram of the Rust reference:
// every bucket, the count, and every statistic.
func checkAgainst(t *testing.T, h Hist, ref refHist) {
	t.Helper()
	var got [][2]uint64
	for _, b := range h.buckets {
		got = append(got, [2]uint64{canonical.highestEquivalent(canonical.valueAt(b.index)), b.count})
	}
	var want [][2]uint64
	for _, b := range ref.Buckets {
		want = append(want, [2]uint64{parseU(t, b[0]), parseU(t, b[1])})
	}
	if !slices.Equal(got, want) {
		t.Fatalf("%s %v buckets differ from the reference merge:\ngot  %v\nwant %v", ref.Metric, ref.Labels, got, want)
	}
	checks := []struct {
		name      string
		got, want uint64
	}{
		{"count", h.Count(), parseU(t, ref.Count)},
		{"min", h.Min(), parseU(t, ref.Min)},
		{"max", h.Max(), parseU(t, ref.Max)},
		{"p50", h.Quantile(0.5), parseU(t, ref.P50)},
		{"p90", h.Quantile(0.9), parseU(t, ref.P90)},
		{"p99", h.Quantile(0.99), parseU(t, ref.P99)},
		{"p999", h.Quantile(0.999), parseU(t, ref.P999)},
	}
	for _, c := range checks {
		if c.got != c.want {
			t.Errorf("%s %v %s = %d, reference %d", ref.Metric, ref.Labels, c.name, c.got, c.want)
		}
	}
	if math.Abs(h.Mean()-ref.Mean) > 1e-9*math.Max(1, ref.Mean) {
		t.Errorf("%s %v mean = %v, reference %v", ref.Metric, ref.Labels, h.Mean(), ref.Mean)
	}
}

// TestMergeMatchesReference merges the histograms of the three agent
// snapshots and compares the result with the merge of fps-stats.
func TestMergeMatchesReference(t *testing.T) {
	ref := loadReference(t)
	if len(ref.Merged.Histograms) == 0 {
		t.Fatal("reference holds no histograms")
	}
	parts := map[string][]Hist{}
	for _, s := range ref.Snapshots {
		snap := parseSnapshot(t, s.Snapshot)
		for _, hv := range snap.GetHistograms() {
			h, err := DecodeHist(hv.GetHdrV2())
			if err != nil {
				t.Fatalf("decode %s: %v", hv.GetMetric(), err)
			}
			if h.Count() != hv.GetCount() {
				t.Fatalf("decoded count %d differs from the snapshot count %d", h.Count(), hv.GetCount())
			}
			k := hv.GetMetric() + labelKey(hv.GetLabels())
			parts[k] = append(parts[k], h)
		}
	}
	mergedSnap := parseSnapshot(t, ref.Merged.Snapshot)
	for _, rh := range ref.Merged.Histograms {
		k := rh.Metric + labelKey(rh.Labels)
		if len(parts[k]) < 2 {
			t.Fatalf("%s: only %d parts, the fixture should merge several", k, len(parts[k]))
		}
		merged := Merge(parts[k]...)
		checkAgainst(t, merged, rh)
		// The merge in reverse order is the same.
		rev := slices.Clone(parts[k])
		slices.Reverse(rev)
		if got := Merge(rev...); !slices.Equal(got.buckets, merged.buckets) {
			t.Errorf("%s: merge depends on order", k)
		}
		// The encoding of the Rust merge decodes to the same buckets.
		for _, hv := range mergedSnap.GetHistograms() {
			if hv.GetMetric()+labelKey(hv.GetLabels()) != k {
				continue
			}
			h, err := DecodeHist(hv.GetHdrV2())
			if err != nil {
				t.Fatal(err)
			}
			if !slices.Equal(h.buckets, merged.buckets) || h.total != merged.total {
				t.Errorf("%s: decoded Rust merge differs from the Go merge", k)
			}
		}
	}
}

func labelKey(m map[string]string) string {
	var ls []Label
	for k, v := range m {
		ls = append(ls, Label{k, v})
	}
	slices.SortFunc(ls, func(a, b Label) int { return cmpString(a.Name, b.Name) })
	return seriesKey("", ls)
}

func cmpString(a, b string) int {
	switch {
	case a < b:
		return -1
	case a > b:
		return 1
	}
	return 0
}

// TestRoundTrip records random values with the test encoder in both forms
// and checks counts, extremes, and that every value lands in its bucket.
func TestRoundTrip(t *testing.T) {
	rng := rand.New(rand.NewPCG(1, 2)) //nolint:gosec // G404: seeded test data
	for trial := 0; trial < 50; trial++ {
		n := 1 + rng.Uint64N(500)
		values := make([]uint64, n)
		for i := range values {
			values[i] = rng.Uint64N(uint64(math.Pow10(1 + rng.IntN(9))))
		}
		enc := record(values...)
		h, err := DecodeHist(enc)
		if err != nil {
			t.Fatal(err)
		}
		if h.Count() != n {
			t.Fatalf("count %d, want %d", h.Count(), n)
		}
		lo, hi := slices.Min(values), slices.Max(values)
		if h.Min() != canonical.lowestEquivalent(lo) || h.Max() != canonical.highestEquivalent(hi) {
			t.Fatalf("min %d max %d, want %d and %d", h.Min(), h.Max(), canonical.lowestEquivalent(lo), canonical.highestEquivalent(hi))
		}
		if q := h.Quantile(1); q != h.Max() {
			t.Fatalf("p100 %d differs from max %d", q, h.Max())
		}
		if q := h.Quantile(0); q != h.Min() {
			t.Fatalf("p0 %d differs from min %d", q, h.Min())
		}
		// The relative bucket width stays within the precision of 3 digits.
		for _, v := range values {
			if v < 2000 {
				continue
			}
			if w := float64(canonical.rangeSize(v)) / float64(v); w > 1e-3 {
				t.Fatalf("bucket of %d is %v wide relative to the value", v, w)
			}
		}
	}
}

func TestLayoutIndexInverse(t *testing.T) {
	for _, v := range []uint64{0, 1, 2047, 2048, 2049, 4095, 4096, 1 << 20, 3_600_000_000, 1 << 50} {
		i := canonical.index(v)
		lo := canonical.valueAt(i)
		if lo != canonical.lowestEquivalent(v) || canonical.index(lo) != i {
			t.Fatalf("value %d: index %d maps back to %d", v, i, lo)
		}
		if hi := canonical.highestEquivalent(v); canonical.index(hi) != i || canonical.index(hi+1) != i+1 {
			t.Fatalf("value %d: highest equivalent %d leaves the bucket", v, hi)
		}
	}
}

// TestRebucket decodes a histogram of another layout and checks that each
// bucket lands in the canonical bucket of its lowest value.
func TestRebucket(t *testing.T) {
	src, err := newLayout(1, 2)
	if err != nil {
		t.Fatal(err)
	}
	counts := make([]uint64, src.index(1_000_000)+1)
	for _, v := range []uint64{5, 150, 151, 999, 123_456, 1_000_000} {
		counts[src.index(v)] += 2
	}
	h, err := DecodeHist(encodeV2(1, 2_000_000, 2, counts))
	if err != nil {
		t.Fatal(err)
	}
	if h.Count() != 12 {
		t.Fatalf("count %d", h.Count())
	}
	var want []uint64
	for i, c := range counts {
		if c > 0 {
			want = append(want, canonical.index(src.valueAt(uint64(i)))) //nolint:gosec // G115: i is a slice index
		}
	}
	var got []uint64
	for _, b := range h.buckets {
		got = append(got, b.index)
	}
	if !slices.Equal(got, want) {
		t.Fatalf("buckets %v, want %v", got, want)
	}
}

func TestDecodeRejectsMalformed(t *testing.T) {
	good := encodeV2(1, 1000, 3, []uint64{0, 1, 2})
	mut := func(f func([]byte) []byte) []byte { return f(slices.Clone(good)) }
	cases := map[string][]byte{
		"short":         {0x1c, 0x84},
		"cookie":        {1, 2, 3, 4, 5, 6, 7, 8},
		"short header":  good[:30],
		"payload len":   mut(func(b []byte) []byte { binary.BigEndian.PutUint32(b[4:], 99); return b }),
		"offset":        mut(func(b []byte) []byte { binary.BigEndian.PutUint32(b[8:], 1); return b }),
		"sigfig":        mut(func(b []byte) []byte { binary.BigEndian.PutUint32(b[12:], 6); return b }),
		"lowest zero":   mut(func(b []byte) []byte { binary.BigEndian.PutUint64(b[16:], 0); return b }),
		"highest":       mut(func(b []byte) []byte { binary.BigEndian.PutUint64(b[24:], 1); return b }),
		"truncated":     append(slices.Clone(good[:len(good)-1]), 0x80),
		"past buckets":  encodeV2(1, 2, 3, append(make([]uint64, 2048), 1)),
		"zero run":      append(encodeV2(1, 2, 3, nil)[:headerLen], 0x81, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0xff),
		"compressed":    {0x1c, 0x84, 0x93, 0x14, 0, 0, 0, 9, 1, 2},
		"bad zlib":      {0x1c, 0x84, 0x93, 0x14, 0, 0, 0, 2, 1, 2},
		"inner cookie":  compress([]byte("0123456789012345678901234567890123456789")),
		"huge sigfig 5": encodeV2(1<<62, 1<<62+1, 5, nil),
	}
	fixPayload(cases["zero run"])
	for name, data := range cases {
		t.Run(name, func(t *testing.T) {
			if _, err := DecodeHist(data); !errors.Is(err, ErrHistogram) {
				t.Fatalf("err = %v, want ErrHistogram", err)
			}
		})
	}
}

// fixPayload sets the payload length of an uncompressed encoding to its
// actual length.
func fixPayload(b []byte) {
	binary.BigEndian.PutUint32(b[4:], uint32(len(b)-headerLen)) //nolint:gosec // G115: test payloads are small
}

func TestDecodeBomb(t *testing.T) {
	// Zeros inflate far beyond the limit.
	raw := make([]byte, maxInflatedLen+headerLen+1)
	copy(raw, encodeV2(1, 2, 3, nil))
	if _, err := DecodeHist(compress(raw)); !errors.Is(err, ErrHistogram) {
		t.Fatalf("err = %v, want ErrHistogram", err)
	}
}

func TestEmptyHistogram(t *testing.T) {
	h, err := DecodeHist(nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, s := range Stats {
		if v := h.Value(s); v != 0 {
			t.Errorf("%s of an empty histogram = %v", s.Name, v)
		}
	}
	if got := Merge(h, Hist{}); got.Count() != 0 {
		t.Fatal("merge of empty histograms is not empty")
	}
}

func FuzzDecodeHist(f *testing.F) {
	f.Add(record(1, 2, 3, 1000, 1_000_000))
	f.Add(encodeV2(1, 1000, 3, []uint64{0, 1, 0, 0, 2}))
	f.Add([]byte{0x1c, 0x84, 0x93, 0x14, 0, 0, 0, 0})
	f.Fuzz(func(t *testing.T, data []byte) {
		h, err := DecodeHist(data)
		if err != nil {
			return
		}
		var sum uint64
		for _, b := range h.buckets {
			sum += b.count
		}
		if sum != h.total {
			t.Fatalf("bucket sum %d differs from total %d", sum, h.total)
		}
		for _, s := range Stats {
			_ = h.Value(s)
		}
	})
}
