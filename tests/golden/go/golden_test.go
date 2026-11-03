// SPDX-License-Identifier: GPL-3.0-only

package golden

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// recorder is a testing.TB that records a fatal failure instead of ending the test.
type recorder struct {
	testing.TB
	failed bool
	msg    string
}

func (r *recorder) Helper() {}

func (r *recorder) Fatal(args ...any) {
	r.failed = true
	r.msg = fmt.Sprint(args...)
}

func TestModeUpdatesOnlyForOne(t *testing.T) {
	if got := ModeFromValue("1", true); got != Update {
		t.Fatalf("ModeFromValue(1) = %v, want Update", got)
	}
	for _, v := range []string{"", "0", "true", "yes"} {
		if got := ModeFromValue(v, true); got != Compare {
			t.Errorf("ModeFromValue(%q) = %v, want Compare", v, got)
		}
	}
	if got := ModeFromValue("1", false); got != Compare {
		t.Errorf("ModeFromValue unset = %v, want Compare", got)
	}
}

func TestMatchPassesAgainstCommittedTestdata(t *testing.T) {
	AssertString(t, "report.txt", "sessions 4\nerrors 0\n")
}

func TestMatchLeavesTheFileUntouchedInUpdateMode(t *testing.T) {
	path := filepath.Join(t.TempDir(), "out.txt")
	writeFile(t, path, "same\n")
	old := time.Unix(1_000_000, 0)
	if err := os.Chtimes(path, old, old); err != nil {
		t.Fatal(err)
	}
	out, err := Check(path, []byte("same\n"), Update)
	if err != nil || out != Matched {
		t.Fatalf("Check = %v, %v, want Matched", out, err)
	}
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if !info.ModTime().Equal(old) {
		t.Errorf("modification time changed to %v", info.ModTime())
	}
}

func TestMismatchNamesTheGoldenFileAndShowsADiff(t *testing.T) {
	path := filepath.Join(t.TempDir(), "out.txt")
	writeFile(t, path, "sessions 4\nerrors 0\n")
	_, err := Check(path, []byte("sessions 4\nerrors 1\n"), Compare)
	if !errors.Is(err, ErrMismatch) {
		t.Fatalf("Check error = %v, want ErrMismatch", err)
	}
	var me *MismatchError
	if !errors.As(err, &me) || me.Path != path {
		t.Fatalf("Check error = %#v, want MismatchError for %s", err, path)
	}
	msg := err.Error()
	for _, want := range []string{"golden file " + path, "UPDATE_GOLDEN=1", "-errors 0\n+errors 1\n"} {
		if !strings.Contains(msg, want) {
			t.Errorf("error lacks %q:\n%s", want, msg)
		}
	}
	if got := readFile(t, path); got != "sessions 4\nerrors 0\n" {
		t.Errorf("golden file changed to %q", got)
	}
}

func TestMismatchFailsTheTestInAssertFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "out.txt")
	writeFile(t, path, "a\n")
	t.Setenv(UpdateEnv, "")
	r := &recorder{TB: t}
	AssertFile(r, path, []byte("b\n"))
	if !r.failed || !strings.Contains(r.msg, path) || !strings.Contains(r.msg, "-a\n+b\n") {
		t.Fatalf("recorder = %+v, want a failure that names %s with a diff", r, path)
	}
}

func TestMissingFileFailsInCompareMode(t *testing.T) {
	path := filepath.Join(t.TempDir(), "out.txt")
	_, err := Check(path, []byte("x\n"), Compare)
	if !errors.Is(err, ErrMissing) || !strings.Contains(err.Error(), path) {
		t.Fatalf("Check error = %v, want ErrMissing naming %s", err, path)
	}
	if _, err := os.Stat(path); !errors.Is(err, os.ErrNotExist) {
		t.Errorf("golden file was created: %v", err)
	}
}

func TestUpdateRewritesADifferingFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "out.txt")
	writeFile(t, path, "old\n")
	if out, err := Check(path, []byte("new\n"), Update); err != nil || out != Written {
		t.Fatalf("Check = %v, %v, want Written", out, err)
	}
	if got := readFile(t, path); got != "new\n" {
		t.Errorf("golden file = %q, want new", got)
	}
	if out, err := Check(path, []byte("new\n"), Compare); err != nil || out != Matched {
		t.Errorf("Check after update = %v, %v, want Matched", out, err)
	}
}

func TestUpdateThroughTheEnvironment(t *testing.T) {
	path := filepath.Join(t.TempDir(), "nested", "dir", "out.bin")
	t.Setenv(UpdateEnv, "1")
	r := &recorder{TB: t}
	AssertFile(r, path, []byte{0, 1, 2})
	if r.failed {
		t.Fatalf("AssertFile failed in update mode: %s", r.msg)
	}
	if got := readFile(t, path); got != "\x00\x01\x02" {
		t.Errorf("golden file = %q", got)
	}
}

func TestDiff(t *testing.T) {
	lines := func(n int) string {
		var b strings.Builder
		for i := 0; i < n; i++ {
			fmt.Fprintf(&b, "line %d\n", i)
		}
		return b.String()
	}
	twenty := lines(20)
	tests := []struct {
		name, golden, got string
		want              []string
	}{
		{"one replaced line", "a\nb\nc\nd\ne\nf\ng\n", "a\nb\nc\nX\ne\nf\ng\n",
			[]string{"--- golden f\n+++ actual\n@@ -1,7 +1,7 @@\n a\n b\n c\n-d\n+X\n e\n f\n g\n"}},
		{"distant changes", twenty,
			strings.Replace(strings.Replace(twenty, "line 2\n", "two\n", 1), "line 17\n", "seventeen\n", 1),
			[]string{"@@ -1,6 +1,6 @@\n", "@@ -15,6 +15,6 @@\n"}},
		{"missing final newline", "a\n", "a", []string{"-a\n+a\n\\ No newline at end of file\n"}},
		{"insertion into empty", "", "x\n", []string{"--- golden f\n+++ actual\n@@ -0,0 +1 @@\n+x\n"}},
		{"binary", "\x00\x01\x02\xff", "\x00\x01\x03\xff\x04",
			[]string{"golden has 4 bytes, actual has 5 bytes", "first difference at offset 2", "\n-02 ff\n+03 ff 04\n"}},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			d := Diff("f", []byte(tc.golden), []byte(tc.got))
			for _, w := range tc.want {
				if !strings.Contains(d, w) {
					t.Errorf("diff lacks %q:\n%s", w, d)
				}
			}
		})
	}
}

func writeFile(t *testing.T, path, s string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(s), 0o600); err != nil {
		t.Fatal(err)
	}
}

func readFile(t *testing.T, path string) string {
	t.Helper()
	b, err := os.ReadFile(path) //nolint:gosec // G304: test paths come from t.TempDir
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}
