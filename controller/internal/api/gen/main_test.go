// SPDX-License-Identifier: GPL-3.0-only

package main

import (
	"bytes"
	"os"
	"path/filepath"
	"regexp"
	"testing"

	"github.com/alrawi/FixingPointSystem/controller/internal/api/contract"
)

// TestGeneratedCodeIsCurrent regenerates package api from the contract and
// compares the result with the committed files.
func TestGeneratedCodeIsCurrent(t *testing.T) {
	files, err := Generate("../../../../contracts/openapi/fps-api-v1.yaml")
	if err != nil {
		t.Fatal(err)
	}
	if len(files) != 2 {
		t.Fatalf("Generate returned %d files, want 2", len(files))
	}
	for name, want := range files {
		got, err := os.ReadFile(filepath.Clean(filepath.Join("..", name)))
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(got, want) {
			t.Errorf("internal/api/%s is out of date with the contract; run go generate in controller/internal/api", name)
		}
	}
}

// TestCodegenVersionMatchesGoMod keeps the version in the generated header
// equal to the version that go.mod pins.
func TestCodegenVersionMatchesGoMod(t *testing.T) {
	mod, err := os.ReadFile("../../../go.mod")
	if err != nil {
		t.Fatal(err)
	}
	m := regexp.MustCompile(`(?m)^\s*` + regexp.QuoteMeta(CodegenModule) + ` (\S+)`).FindSubmatch(mod)
	if m == nil {
		t.Fatalf("go.mod does not require %s", CodegenModule)
	}
	if string(m[1]) != CodegenVersion {
		t.Fatalf("go.mod pins %s %s, but CodegenVersion is %s", CodegenModule, m[1], CodegenVersion)
	}
}

func TestRoutesRejectsBadScope(t *testing.T) {
	doc, err := contract.Parse([]byte(`
openapi: 3.0.3
info: {title: t, version: "1"}
paths:
  /a:
    get:
      operationId: getA
      x-required-scope: root
      responses: {"200": {description: ok}}
`))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := Routes(doc); err == nil {
		t.Fatal("Routes accepted scope root")
	}
}

func TestRoutesRejectsMissingOperationID(t *testing.T) {
	doc, err := contract.Parse([]byte(`
openapi: 3.0.3
info: {title: t, version: "1"}
paths:
  /a:
    get:
      x-required-scope: read
      responses: {"200": {description: ok}}
`))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := Routes(doc); err == nil {
		t.Fatal("Routes accepted an operation without operationId")
	}
}
