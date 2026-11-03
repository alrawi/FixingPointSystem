// SPDX-License-Identifier: GPL-3.0-only

package contract

import (
	"context"
	"reflect"
	"testing"

	"go.yaml.in/yaml/v3"
)

// view applies View to src and decodes the result.
func view(t *testing.T, src string) map[string]any {
	t.Helper()
	out, err := View([]byte(src))
	if err != nil {
		t.Fatal(err)
	}
	var m map[string]any
	if err := yaml.Unmarshal(out, &m); err != nil {
		t.Fatal(err)
	}
	return m
}

func TestViewReplacesExternalRefs(t *testing.T) {
	got := view(t, `
a: {$ref: "#/components/schemas/Local"}
b: {$ref: "../schemas/test-config.schema.json"}
c:
  - {$ref: "other.json#/x", description: dropped}
`)
	want := map[string]any{
		"a": map[string]any{"$ref": "#/components/schemas/Local"},
		"b": map[string]any{"type": "object", ExternalSchemaExtension: "../schemas/test-config.schema.json"},
		"c": []any{map[string]any{"type": "object", ExternalSchemaExtension: "other.json#/x"}},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("View = %v, want %v", got, want)
	}
}

func TestViewTypesStringEnums(t *testing.T) {
	got := view(t, `
s: {enum: [a, b]}
typed: {type: integer, enum: ["1"]}
mixed: {enum: [a, 1]}
`)
	want := map[string]any{
		"s":     map[string]any{"enum": []any{"a", "b"}, "type": "string"},
		"typed": map[string]any{"enum": []any{"1"}, "type": "integer"},
		"mixed": map[string]any{"enum": []any{"a", 1}},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("View = %v, want %v", got, want)
	}
}

func TestViewRejectsBadYAML(t *testing.T) {
	if _, err := View([]byte("a: [")); err == nil {
		t.Fatal("View accepted broken YAML")
	}
}

func TestLoadContract(t *testing.T) {
	doc, err := Load("../../../../contracts/openapi/fps-api-v1.yaml")
	if err != nil {
		t.Fatal(err)
	}
	if err := Validate(context.Background(), doc); err != nil {
		t.Fatal(err)
	}
	if doc.Paths.Len() == 0 {
		t.Fatal("contract has no paths")
	}
	body := doc.Paths.Value("/configs").Post.RequestBody.Value.Content["application/json"].Schema.Value
	if got := body.Extensions[ExternalSchemaExtension]; got != "../schemas/test-config.schema.json" {
		t.Errorf("createConfig body extension = %v", got)
	}
}

func TestLoadMissingFile(t *testing.T) {
	if _, err := Load("no-such-file.yaml"); err == nil {
		t.Fatal("Load accepted a missing file")
	}
}
