// SPDX-License-Identifier: GPL-3.0-only

package store

import (
	"bytes"
	"os"
	"path/filepath"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
)

func TestEmbeddedSchemaMatchesContract(t *testing.T) {
	want, err := os.ReadFile(filepath.Join(contracts, "schemas", "test-config.schema.json"))
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(testConfigSchema, want) {
		t.Fatal("schema/test-config.schema.json differs from the contract, run go generate ./internal/store/")
	}
}

func newSchema(t *testing.T) *Schema {
	t.Helper()
	s, err := NewSchema()
	if err != nil {
		t.Fatal(err)
	}
	return s
}

func TestSchemaAcceptsEveryExample(t *testing.T) {
	s := newSchema(t)
	for _, ex := range examples(t) {
		fs, err := s.Check(mustCanonical(t, ex.data))
		if err != nil || len(fs) != 0 {
			t.Errorf("%s: Check = %+v, %v, want no findings", ex.name, fs, err)
		}
	}
}

func TestSchemaNamesTheFailingField(t *testing.T) {
	s := newSchema(t)
	for _, fx := range readFixtures(t, filepath.Join("testdata", "invalid")) {
		if fx.want == "" {
			t.Fatalf("%s: the first line names no wanted path", fx.name)
		}
		fs, err := s.Check(mustCanonical(t, fx.data))
		if err != nil {
			t.Fatalf("%s: %v", fx.name, err)
		}
		if !hasFinding(fs, api.FindingSeverityError, fx.want) {
			t.Errorf("%s: findings %+v name no error at %s", fx.name, fs, fx.want)
		}
		for _, f := range fs {
			if f.Message == "" {
				t.Errorf("%s: finding at %s has no message", fx.name, f.Path)
			}
		}
	}
}

func TestSchemaRootErrors(t *testing.T) {
	s := newSchema(t)
	fs, err := s.Check([]byte(`[1]`))
	if err != nil || !hasFinding(fs, api.FindingSeverityError, "") {
		t.Errorf("Check of an array = %+v, %v, want an error at the root", fs, err)
	}
	if _, err := s.Check([]byte(`{`)); err == nil {
		t.Error("Check of malformed JSON succeeded")
	}
}

func TestCompileSchemaErrors(t *testing.T) {
	for _, src := range []string{`{`, `{"type": 5}`} {
		if _, err := compileSchema([]byte(src)); err == nil {
			t.Errorf("compileSchema(%s) succeeded", src)
		}
	}
}

func TestPointerEscapes(t *testing.T) {
	if got := pointer([]string{"a/b", "c~d", "0"}); got != "/a~1b/c~0d/0" {
		t.Errorf("pointer = %q", got)
	}
}

func TestSortFindingsDropsDuplicates(t *testing.T) {
	in := []api.Finding{
		errorFinding("/b", "x"), infoFinding("/a", "y"), errorFinding("/a", "y"), errorFinding("/b", "x"),
	}
	got := sortFindings(in)
	want := []api.Finding{errorFinding("/a", "y"), infoFinding("/a", "y"), errorFinding("/b", "x")}
	if len(got) != len(want) {
		t.Fatalf("sortFindings = %+v, want %+v", got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Errorf("sortFindings[%d] = %+v, want %+v", i, got[i], want[i])
		}
	}
}
