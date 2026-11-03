// SPDX-License-Identifier: GPL-3.0-only

package metrics

import (
	"bytes"
	"compress/zlib"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"math"
	"math/bits"
)

// The HdrHistogram V2 encoding, which HistogramValue.hdr_v2 of
// contracts/proto/fps/agent/v1/agent.proto names, written by the Rust crate
// hdrhistogram in engine/fps-stats. We decode it here rather than through a
// library, because the Go library panics on truncated input, allocates the
// dense counts array that the header asks for, and cannot bound the inflated
// size. The decoder keeps the counts sparse, so a merge or a quantile costs
// time in the number of occupied buckets.

// Cookies of the V2 encoding. Writers set bits 4 to 7 to the word size, so
// readers compare the cookie with those bits cleared, as every HdrHistogram
// implementation does.
const (
	cookieV2           = 0x1c849303
	cookieV2Compressed = 0x1c849304
	cookieMask         = ^uint32(0xf0)
	headerLen          = 40
)

// Limits on untrusted encodings.
const (
	// maxEncodedLen bounds one compressed histogram.
	maxEncodedLen = 4 << 20
	// maxInflatedLen bounds the inflated V2 form, which holds a 40 byte
	// header and at most 9 bytes per bucket.
	maxInflatedLen = 16 << 20
	// maxSigFig is the largest precision HdrHistogram supports.
	maxSigFig = 5
)

// CanonicalLowest and CanonicalSigFig name the bucket layout of every merged
// histogram. They equal the layout of engine/fps-stats, which records with a
// lowest discernible value of 1 and 3 significant digits, so a merge of fps
// agent histograms is exact. A histogram of another layout is re-bucketed by
// the value of each bucket, as HdrHistogram's own add does.
const (
	CanonicalLowest = 1
	CanonicalSigFig = 3
)

// ErrHistogram reports a malformed HdrHistogram encoding.
var ErrHistogram = errors.New("malformed hdr histogram")

// layout is the bucket structure of an HdrHistogram with a given lowest
// discernible value and precision.
type layout struct {
	unitMagnitude    uint
	halfCountMag     uint
	subBucketCount   uint64
	subBucketHalf    uint64
	subBucketMask    uint64
	leadingZeroCount uint
}

func newLayout(lowest uint64, sigfig uint) (layout, error) {
	if lowest < 1 || sigfig > maxSigFig {
		return layout{}, fmt.Errorf("%w: lowest %d and precision %d are out of range", ErrHistogram, lowest, sigfig)
	}
	largestSingle := 2 * uint64(math.Pow10(int(sigfig)))
	countMag := uint(bits.Len64(largestSingle - 1))
	halfMag := uint(0)
	if countMag > 1 {
		halfMag = countMag - 1
	}
	unitMag := uint(bits.Len64(lowest) - 1)
	if unitMag+halfMag+1 > 62 {
		return layout{}, fmt.Errorf("%w: lowest %d is too large for precision %d", ErrHistogram, lowest, sigfig)
	}
	l := layout{unitMagnitude: unitMag, halfCountMag: halfMag}
	l.subBucketCount = 1 << (halfMag + 1)
	l.subBucketHalf = l.subBucketCount / 2
	l.subBucketMask = (l.subBucketCount - 1) << unitMag
	l.leadingZeroCount = 64 - unitMag - halfMag - 1
	return l, nil
}

var canonical = func() layout {
	l, err := newLayout(CanonicalLowest, CanonicalSigFig)
	if err != nil {
		panic(err) // constant arguments, covered by the tests
	}
	return l
}()

// countsLen returns the length of the counts array that covers highest.
func (l layout) countsLen(highest uint64) uint64 {
	smallestUntrackable := l.subBucketCount << l.unitMagnitude
	buckets := uint64(1)
	for smallestUntrackable <= highest {
		if smallestUntrackable > math.MaxUint64/2 {
			buckets++
			break
		}
		smallestUntrackable <<= 1
		buckets++
	}
	return (buckets + 1) * l.subBucketHalf
}

// position returns the bucket and sub-bucket of value.
func (l layout) position(value uint64) (bucket, sub uint64) {
	bucket = uint64(l.leadingZeroCount) - uint64(bits.LeadingZeros64(value|l.subBucketMask))
	sub = value >> (bucket + uint64(l.unitMagnitude))
	return bucket, sub
}

// index returns the counts index of value.
func (l layout) index(value uint64) uint64 {
	b, s := l.position(value)
	return ((b + 1) << l.halfCountMag) + s - l.subBucketHalf
}

// valueAt returns the lowest value of the bucket at counts index i.
func (l layout) valueAt(i uint64) uint64 {
	b := int64(i>>l.halfCountMag) - 1
	s := (i & (l.subBucketHalf - 1)) + l.subBucketHalf
	if b < 0 {
		s -= l.subBucketHalf
		b = 0
	}
	return s << (uint64(b) + uint64(l.unitMagnitude))
}

// rangeSize returns the number of values that share the bucket of value.
func (l layout) rangeSize(value uint64) uint64 {
	b, s := l.position(value)
	if s >= l.subBucketCount {
		b++
	}
	return 1 << (uint64(l.unitMagnitude) + b)
}

// lowestEquivalent returns the smallest value in the bucket of value.
func (l layout) lowestEquivalent(value uint64) uint64 {
	b, s := l.position(value)
	return s << (b + uint64(l.unitMagnitude))
}

// highestEquivalent returns the largest value in the bucket of value.
func (l layout) highestEquivalent(value uint64) uint64 {
	lo, size := l.lowestEquivalent(value), l.rangeSize(value)
	if lo > math.MaxUint64-size+1 {
		return math.MaxUint64
	}
	return lo + size - 1
}

// medianEquivalent returns the middle value of the bucket of value.
func (l layout) medianEquivalent(value uint64) uint64 {
	return l.lowestEquivalent(value) + l.rangeSize(value)>>1
}

// bucket is one occupied bucket of a histogram.
type bucket struct {
	index uint64
	count uint64
}

// Hist is an HDR distribution in the canonical layout. The zero value is an
// empty histogram. Hist values are immutable once built, so readers share
// them without copies.
type Hist struct {
	buckets []bucket // ordered by index, every count above zero
	total   uint64
}

// DecodeHist decodes a histogram in the HdrHistogram V2 encoding, compressed
// or not, and re-buckets it into the canonical layout. An empty input is an
// empty histogram. Malformed input returns an error that wraps ErrHistogram
// and never panics.
func DecodeHist(data []byte) (Hist, error) {
	if len(data) == 0 {
		return Hist{}, nil
	}
	if len(data) < 8 {
		return Hist{}, fmt.Errorf("%w: %d bytes are too short for a cookie", ErrHistogram, len(data))
	}
	switch binary.BigEndian.Uint32(data) & cookieMask {
	case cookieV2Compressed:
		raw, err := inflate(data)
		if err != nil {
			return Hist{}, err
		}
		return decodeV2(raw)
	case cookieV2:
		return decodeV2(data)
	}
	return Hist{}, fmt.Errorf("%w: unknown cookie %#08x", ErrHistogram, binary.BigEndian.Uint32(data))
}

func inflate(data []byte) ([]byte, error) {
	if len(data) > maxEncodedLen {
		return nil, fmt.Errorf("%w: %d compressed bytes exceed the limit of %d", ErrHistogram, len(data), maxEncodedLen)
	}
	n := binary.BigEndian.Uint32(data[4:])
	if uint64(n) > uint64(len(data)-8) {
		return nil, fmt.Errorf("%w: compressed length %d exceeds the %d bytes present", ErrHistogram, n, len(data)-8)
	}
	z, err := zlib.NewReader(bytes.NewReader(data[8 : 8+int(n)]))
	if err != nil {
		return nil, fmt.Errorf("%w: inflate: %w", ErrHistogram, err)
	}
	defer func() { _ = z.Close() }()
	raw, err := io.ReadAll(io.LimitReader(z, maxInflatedLen+1))
	if err != nil {
		return nil, fmt.Errorf("%w: inflate: %w", ErrHistogram, err)
	}
	if len(raw) > maxInflatedLen {
		return nil, fmt.Errorf("%w: inflated form exceeds %d bytes", ErrHistogram, maxInflatedLen)
	}
	return raw, nil
}

func decodeV2(raw []byte) (Hist, error) {
	if len(raw) < headerLen {
		return Hist{}, fmt.Errorf("%w: %d bytes are too short for a V2 header", ErrHistogram, len(raw))
	}
	if binary.BigEndian.Uint32(raw)&cookieMask != cookieV2 {
		return Hist{}, fmt.Errorf("%w: inflated cookie %#08x is not V2", ErrHistogram, binary.BigEndian.Uint32(raw))
	}
	payloadLen := binary.BigEndian.Uint32(raw[4:])
	offset := binary.BigEndian.Uint32(raw[8:])
	sigfig := binary.BigEndian.Uint32(raw[12:])
	lowest := binary.BigEndian.Uint64(raw[16:])
	highest := binary.BigEndian.Uint64(raw[24:])
	if uint64(payloadLen) != uint64(len(raw)-headerLen) {
		return Hist{}, fmt.Errorf("%w: payload length %d differs from the %d bytes present", ErrHistogram, payloadLen, len(raw)-headerLen)
	}
	if offset != 0 {
		return Hist{}, fmt.Errorf("%w: normalizing index offset %d is not supported", ErrHistogram, int32(offset))
	}
	if sigfig > maxSigFig || lowest < 1 || lowest > math.MaxInt64 || highest > math.MaxInt64 || highest < 2*lowest {
		return Hist{}, fmt.Errorf("%w: header with lowest %d, highest %d, and precision %d is invalid", ErrHistogram, lowest, highest, sigfig)
	}
	src, err := newLayout(lowest, uint(sigfig))
	if err != nil {
		return Hist{}, err
	}
	limit := src.countsLen(highest)
	same := src == canonical
	var h Hist
	payload := raw[headerLen:]
	var idx uint64
	for len(payload) > 0 {
		v, n, err := zigzag(payload)
		if err != nil {
			return Hist{}, err
		}
		payload = payload[n:]
		if v < 0 {
			zeros := uint64(-(v + 1)) + 1
			if zeros > limit-idx {
				return Hist{}, fmt.Errorf("%w: zero run passes the %d buckets of the header", ErrHistogram, limit)
			}
			idx += zeros
			continue
		}
		if idx >= limit {
			return Hist{}, fmt.Errorf("%w: counts pass the %d buckets of the header", ErrHistogram, limit)
		}
		if v > 0 {
			c := uint64(v)
			if h.total > math.MaxUint64-c {
				return Hist{}, fmt.Errorf("%w: total count overflows", ErrHistogram)
			}
			h.total += c
			target := idx
			if !same {
				target = canonical.index(src.valueAt(idx))
			}
			h.add(target, c)
		}
		idx++
	}
	return h, nil
}

// add adds c to the bucket at index i. Decoding appends in index order, and
// re-bucketing keeps the order because the bucket value grows with the index,
// so add only ever touches the last bucket or appends.
func (h *Hist) add(i, c uint64) {
	if n := len(h.buckets); n > 0 && h.buckets[n-1].index == i {
		h.buckets[n-1].count += c
		return
	}
	h.buckets = append(h.buckets, bucket{index: i, count: c})
}

// zigzag reads one ZigZag LEB128 value of at most 9 bytes, whose last byte
// carries 8 bits.
func zigzag(b []byte) (int64, int, error) {
	var u uint64
	for i := 0; i < 9; i++ {
		if i >= len(b) {
			return 0, 0, fmt.Errorf("%w: truncated count", ErrHistogram)
		}
		if i == 8 {
			u |= uint64(b[i]) << 56
			return int64(u>>1) ^ -int64(u&1), 9, nil
		}
		u |= uint64(b[i]&0x7f) << (7 * uint(i))
		if b[i]&0x80 == 0 {
			return int64(u>>1) ^ -int64(u&1), i + 1, nil
		}
	}
	return 0, 0, fmt.Errorf("%w: truncated count", ErrHistogram)
}

// Merge returns the union of the distributions. Each bucket count is the sum
// of the bucket counts, which is the add of HdrHistogram in one layout.
func Merge(hs ...Hist) Hist {
	var out Hist
	for _, h := range hs {
		out = merge2(out, h)
	}
	return out
}

func merge2(a, b Hist) Hist {
	if len(a.buckets) == 0 {
		return b
	}
	if len(b.buckets) == 0 {
		return a
	}
	out := Hist{buckets: make([]bucket, 0, max(len(a.buckets), len(b.buckets))), total: satAdd(a.total, b.total)}
	i, j := 0, 0
	for i < len(a.buckets) || j < len(b.buckets) {
		switch {
		case j == len(b.buckets) || (i < len(a.buckets) && a.buckets[i].index < b.buckets[j].index):
			out.buckets = append(out.buckets, a.buckets[i])
			i++
		case i == len(a.buckets) || b.buckets[j].index < a.buckets[i].index:
			out.buckets = append(out.buckets, b.buckets[j])
			j++
		default:
			out.buckets = append(out.buckets, bucket{index: a.buckets[i].index, count: satAdd(a.buckets[i].count, b.buckets[j].count)})
			i++
			j++
		}
	}
	return out
}

func satAdd(a, b uint64) uint64 {
	if a > math.MaxUint64-b {
		return math.MaxUint64
	}
	return a + b
}

// Count returns the number of recorded values.
func (h Hist) Count() uint64 { return h.total }

// Min returns the smallest recorded value at bucket resolution, or 0 when
// the histogram is empty.
func (h Hist) Min() uint64 {
	if len(h.buckets) == 0 {
		return 0
	}
	return canonical.valueAt(h.buckets[0].index)
}

// Max returns the largest recorded value at bucket resolution, which is the
// highest value of its bucket, or 0 when the histogram is empty.
func (h Hist) Max() uint64 {
	if len(h.buckets) == 0 {
		return 0
	}
	return canonical.highestEquivalent(canonical.valueAt(h.buckets[len(h.buckets)-1].index))
}

// Mean returns the mean of the median values of the buckets, or 0 when the
// histogram is empty. The sum runs in the order of the Rust implementation.
func (h Hist) Mean() float64 {
	if h.total == 0 {
		return 0
	}
	var mean float64
	for _, b := range h.buckets {
		mean += float64(canonical.medianEquivalent(canonical.valueAt(b.index))) * float64(b.count) / float64(h.total)
	}
	return mean
}

// Quantile returns the value below which the fraction q of the recorded
// values lies, as the highest value of its bucket. It follows
// value_at_quantile of HdrHistogram, so q of 0 returns the lowest value of
// the first bucket.
func (h Hist) Quantile(q float64) uint64 {
	if h.total == 0 {
		return 0
	}
	q = min(max(q, 0), 1)
	target := uint64(math.Ceil(q * float64(h.total)))
	if target == 0 {
		target = 1
	}
	var run uint64
	for _, b := range h.buckets {
		run += b.count
		if run >= target {
			v := canonical.valueAt(b.index)
			if q == 0 {
				return canonical.lowestEquivalent(v)
			}
			return canonical.highestEquivalent(v)
		}
	}
	return h.Max()
}

// Stat is a histogram statistic that a metric reference may append to a
// histogram name, as contracts/README.md lists.
type Stat struct {
	// Name is the suffix, such as p99.
	Name string
	// Quantile is the quantile of a percentile statistic, or negative.
	Quantile float64
}

// Stats lists the histogram statistics in the order of contracts/README.md.
var Stats = []Stat{
	{"p50", 0.5}, {"p90", 0.9}, {"p99", 0.99}, {"p999", 0.999},
	{"min", -1}, {"max", -1}, {"mean", -1}, {"count", -1},
}

// Value returns the statistic of h.
func (h Hist) Value(s Stat) float64 {
	switch s.Name {
	case "min":
		return float64(h.Min())
	case "max":
		return float64(h.Max())
	case "mean":
		return h.Mean()
	case "count":
		return float64(h.Count())
	}
	return float64(h.Quantile(s.Quantile))
}
