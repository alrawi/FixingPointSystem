// SPDX-License-Identifier: GPL-3.0-only

// Package golden compares test output with golden files under testdata/.
//
// A test renders its output and calls [Assert] with a file name under the
// package's testdata/ directory. The helper compares the output with that file
// and fails the test with a unified diff that names the file on mismatch. It
// rewrites the file only when the environment variable UPDATE_GOLDEN equals 1.
package golden

import (
	"bytes"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"
	"unicode/utf8"
)

// UpdateEnv is the environment variable that switches the helper to update mode.
const UpdateEnv = "UPDATE_GOLDEN"

const (
	// contextLines is the number of unchanged lines that the diff shows around each change.
	contextLines = 3
	// maxAlignCells bounds the product of line counts that the diff aligns line
	// by line. Larger inputs show their differing middle as one replaced block.
	maxAlignCells = 4_000_000
)

// Mode says whether a comparison may rewrite the golden file.
type Mode int

const (
	// Compare compares the output with the golden file and never writes it.
	Compare Mode = iota
	// Update writes the output to the golden file when the two differ.
	Update
)

// ModeFromEnv reads the mode from UPDATE_GOLDEN in the process environment.
func ModeFromEnv() Mode {
	return ModeFromValue(os.LookupEnv(UpdateEnv))
}

// ModeFromValue returns Update only for the value 1, so a typo or an empty
// value never rewrites golden files. The second argument reports whether the
// variable is set, as [os.LookupEnv] does.
func ModeFromValue(value string, ok bool) Mode {
	if ok && value == "1" {
		return Update
	}
	return Compare
}

// Outcome is the result of a comparison that did not fail.
type Outcome int

const (
	// Matched means that the golden file already held the output.
	Matched Outcome = iota
	// Written means that update mode created or rewrote the golden file.
	Written
)

// ErrMissing reports that the golden file does not exist in compare mode.
var ErrMissing = errors.New("golden file does not exist")

// ErrMismatch reports that the output differs from the golden file.
var ErrMismatch = errors.New("golden file does not match the output")

// MismatchError carries the golden file path and the diff of a mismatch.
// It matches [ErrMismatch] with [errors.Is].
type MismatchError struct {
	// Path is the golden file.
	Path string
	// Diff is the unified diff from the golden file to the output.
	Diff string
}

// Error names the golden file, the update switch, and the diff.
func (e *MismatchError) Error() string {
	return fmt.Sprintf("golden file %s does not match the output; run with %s=1 to accept "+
		"the output and review the change with git diff\n%s", e.Path, UpdateEnv, e.Diff)
}

// Is reports whether target is [ErrMismatch].
func (e *MismatchError) Is(target error) bool { return target == ErrMismatch }

// Check compares got with the golden file at path in the given mode.
//
// In Update mode the function writes the file, and its missing parent
// directories, only when the content differs, so unchanged files keep their
// modification time.
func Check(path string, got []byte, mode Mode) (Outcome, error) {
	want, err := os.ReadFile(path) //nolint:gosec // G304: tests choose their own golden paths
	missing := errors.Is(err, fs.ErrNotExist)
	if err != nil && !missing {
		return Matched, fmt.Errorf("golden file %s: read: %w", path, err)
	}
	if !missing && bytes.Equal(want, got) {
		return Matched, nil
	}
	switch {
	case mode == Update:
		if err := write(path, got); err != nil {
			return Matched, err
		}
		return Written, nil
	case missing:
		return Matched, fmt.Errorf("golden file %s: %w; run with %s=1 to create it", path, ErrMissing, UpdateEnv)
	default:
		return Matched, &MismatchError{Path: path, Diff: Diff(path, want, got)}
	}
}

func write(path string, got []byte) error {
	if dir := filepath.Dir(path); dir != "." {
		if err := os.MkdirAll(dir, 0o750); err != nil {
			return fmt.Errorf("golden file %s: create directory: %w", path, err)
		}
	}
	if err := os.WriteFile(path, got, 0o600); err != nil {
		return fmt.Errorf("golden file %s: write: %w", path, err)
	}
	return nil
}

// Assert compares got with testdata/<name> relative to the package directory,
// which is the working directory of go test, and fails t on mismatch.
func Assert(t testing.TB, name string, got []byte) {
	t.Helper()
	AssertFile(t, filepath.Join("testdata", name), got)
}

// AssertString is [Assert] for string output.
func AssertString(t testing.TB, name, got string) {
	t.Helper()
	Assert(t, name, []byte(got))
}

// AssertFile compares got with the golden file at path in the mode that
// UPDATE_GOLDEN selects, and fails t with a diff that names the file.
func AssertFile(t testing.TB, path string, got []byte) {
	t.Helper()
	if _, err := Check(path, got, ModeFromEnv()); err != nil {
		t.Fatal(err)
	}
}

type opKind int

const (
	opEqual opKind = iota
	opDelete
	opInsert
)

// op is one diff operation. g indexes the golden lines and a the actual lines;
// the index that the kind does not use is -1.
type op struct {
	kind opKind
	g, a int
}

// Diff returns a unified diff from golden to got. The header names the golden
// file with label. Content that is not UTF-8 gets a summary of the first
// differing byte instead of a line diff.
func Diff(label string, golden, got []byte) string {
	if !utf8.Valid(golden) || !utf8.Valid(got) {
		return binaryDiff(label, golden, got)
	}
	g := splitLines(string(golden))
	a := splitLines(string(got))
	ops := align(g, a)
	var b strings.Builder
	fmt.Fprintf(&b, "--- golden %s\n+++ actual\n", label)
	for _, h := range hunks(ops) {
		hunk := ops[h[0]:h[1]]
		g0, a0 := firstPositions(ops, h[0])
		gLen, aLen := 0, 0
		for _, o := range hunk {
			if o.kind != opInsert {
				gLen++
			}
			if o.kind != opDelete {
				aLen++
			}
		}
		fmt.Fprintf(&b, "@@ -%s +%s @@\n", hunkRange(g0, gLen), hunkRange(a0, aLen))
		for _, o := range hunk {
			sign, line := ' ', ""
			switch o.kind {
			case opEqual:
				line = g[o.g]
			case opDelete:
				sign, line = '-', g[o.g]
			case opInsert:
				sign, line = '+', a[o.a]
			}
			b.WriteRune(sign)
			b.WriteString(line)
			if !strings.HasSuffix(line, "\n") {
				b.WriteString("\n\\ No newline at end of file\n")
			}
		}
	}
	return b.String()
}

// splitLines splits s after each newline and keeps the newlines.
func splitLines(s string) []string {
	if s == "" {
		return nil
	}
	lines := strings.SplitAfter(s, "\n")
	if lines[len(lines)-1] == "" {
		lines = lines[:len(lines)-1]
	}
	return lines
}

// hunkRange formats a hunk range in unified diff notation. start is zero based.
func hunkRange(start, n int) string {
	switch n {
	case 0:
		return fmt.Sprintf("%d,0", start)
	case 1:
		return fmt.Sprintf("%d", start+1)
	default:
		return fmt.Sprintf("%d,%d", start+1, n)
	}
}

func firstPositions(ops []op, start int) (int, int) {
	g, a := 0, 0
	for _, o := range ops[:start] {
		if o.kind != opInsert {
			g++
		}
		if o.kind != opDelete {
			a++
		}
	}
	return g, a
}

// hunks groups changed operations with context lines around them and returns
// each hunk as a half open range of ops.
func hunks(ops []op) [][2]int {
	var out [][2]int
	for k, o := range ops {
		if o.kind == opEqual {
			continue
		}
		start := max(k-contextLines, 0)
		end := min(k+1+contextLines, len(ops))
		if n := len(out); n > 0 && start <= out[n-1][1] {
			out[n-1][1] = end
		} else {
			out = append(out, [2]int{start, end})
		}
	}
	return out
}

// align aligns two line lists with a longest common subsequence after
// trimming the common prefix and suffix.
func align(g, a []string) []op {
	prefix := 0
	for prefix < len(g) && prefix < len(a) && g[prefix] == a[prefix] {
		prefix++
	}
	suffix := 0
	for suffix < len(g)-prefix && suffix < len(a)-prefix && g[len(g)-1-suffix] == a[len(a)-1-suffix] {
		suffix++
	}
	gm := g[prefix : len(g)-suffix]
	am := a[prefix : len(a)-suffix]

	ops := make([]op, 0, len(g)+len(a))
	for i := 0; i < prefix; i++ {
		ops = append(ops, op{opEqual, i, i})
	}
	if len(gm)*len(am) > maxAlignCells {
		for i := range gm {
			ops = append(ops, op{opDelete, prefix + i, -1})
		}
		for j := range am {
			ops = append(ops, op{opInsert, -1, prefix + j})
		}
	} else {
		// lcs[i*w+j] is the common subsequence length of gm[i:] and am[j:].
		w := len(am) + 1
		lcs := make([]int, (len(gm)+1)*w)
		for i := len(gm) - 1; i >= 0; i-- {
			for j := len(am) - 1; j >= 0; j-- {
				if gm[i] == am[j] {
					lcs[i*w+j] = lcs[(i+1)*w+j+1] + 1
				} else {
					lcs[i*w+j] = max(lcs[(i+1)*w+j], lcs[i*w+j+1])
				}
			}
		}
		i, j := 0, 0
		for i < len(gm) || j < len(am) {
			switch {
			case i < len(gm) && j < len(am) && gm[i] == am[j]:
				ops = append(ops, op{opEqual, prefix + i, prefix + j})
				i++
				j++
			case j < len(am) && (i == len(gm) || lcs[i*w+j+1] >= lcs[(i+1)*w+j]):
				ops = append(ops, op{opInsert, -1, prefix + j})
				j++
			default:
				ops = append(ops, op{opDelete, prefix + i, -1})
				i++
			}
		}
	}
	gs, as := len(g)-suffix, len(a)-suffix
	for k := 0; k < suffix; k++ {
		ops = append(ops, op{opEqual, gs + k, as + k})
	}
	reorderChanges(ops)
	return ops
}

// reorderChanges sorts each run of consecutive changes so that its deletions
// come first, as diff tools show them.
func reorderChanges(ops []op) {
	for k := 0; k < len(ops); {
		if ops[k].kind == opEqual {
			k++
			continue
		}
		end := k
		for end < len(ops) && ops[end].kind != opEqual {
			end++
		}
		run := ops[k:end]
		sort.SliceStable(run, func(x, y int) bool {
			if run[x].kind != run[y].kind {
				return run[x].kind == opDelete
			}
			return run[x].g+run[x].a < run[y].g+run[y].a
		})
		k = end
	}
}

func binaryDiff(label string, golden, got []byte) string {
	offset := min(len(golden), len(got))
	for i := 0; i < offset; i++ {
		if golden[i] != got[i] {
			offset = i
			break
		}
	}
	window := func(b []byte) string {
		if offset >= len(b) {
			return ""
		}
		end := min(offset+16, len(b))
		parts := make([]string, 0, end-offset)
		for _, x := range b[offset:end] {
			parts = append(parts, fmt.Sprintf("%02x", x))
		}
		return strings.Join(parts, " ")
	}
	return fmt.Sprintf("--- golden %s\n+++ actual\nbinary content differs: golden has %d bytes, "+
		"actual has %d bytes, first difference at offset %d\n-%s\n+%s\n",
		label, len(golden), len(got), offset, window(golden), window(got))
}
