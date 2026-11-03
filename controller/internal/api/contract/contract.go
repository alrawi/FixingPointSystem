// SPDX-License-Identifier: GPL-3.0-only

// Package contract loads the controller's OpenAPI contract for the code
// generator and the contract tests.
//
// The contract is an OpenAPI 3.1 document, and two request and response bodies
// point at JSON Schema 2020-12 files under contracts/schemas/. kin-openapi reads
// the OpenAPI 3.0 schema dialect, which differs from 2020-12 in keywords such as
// exclusiveMinimum. Load therefore builds a view of the contract in which every
// reference to a file outside the document becomes an open object schema that
// names the file in the x-fps-external-schema extension. The view also adds
// type string to each enum whose values are all strings and which names no
// type, because 3.0 tools treat a schema without a type as any value and would
// generate an untyped Go alias. Both rewrites keep the set of valid documents
// the same. Everything else stays as written, so operations, parameters,
// responses, and the document's own schemas reach the generator and the
// validator unchanged.
package contract

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/getkin/kin-openapi/openapi3"
	"go.yaml.in/yaml/v3"
)

// ExternalSchemaExtension names the extension that records the file an external
// reference pointed at before View replaced it.
const ExternalSchemaExtension = "x-fps-external-schema"

// Load reads the contract at path, applies View, and returns the parsed
// document. It does not validate the document, so callers decide how strict to be.
func Load(path string) (*openapi3.T, error) {
	data, err := os.ReadFile(filepath.Clean(path))
	if err != nil {
		return nil, fmt.Errorf("read contract: %w", err)
	}
	doc, err := Parse(data)
	if err != nil {
		return nil, fmt.Errorf("contract %s: %w", path, err)
	}
	return doc, nil
}

// Parse applies View to the contract in data and returns the parsed document.
func Parse(data []byte) (*openapi3.T, error) {
	view, err := View(data)
	if err != nil {
		return nil, err
	}
	doc, err := openapi3.NewLoader().LoadFromData(view)
	if err != nil {
		return nil, fmt.Errorf("parse OpenAPI document: %w", err)
	}
	return doc, nil
}

// extraFields are the OpenAPI 3.1 fields that the contract uses and that the
// 3.0 rules of kin-openapi do not know.
var extraFields = []string{
	"examples",         // a list of schema examples
	"contentMediaType", // the media type of a binary string
	"identifier",       // the SPDX identifier of the license
	"summary",          // the short summary of the API in info
}

// Validate checks doc with the kin-openapi rules, allowing the 3.1 fields that
// the contract uses. Schema examples are not checked, because 3.1 writes them
// as a list.
func Validate(ctx context.Context, doc *openapi3.T) error {
	if err := doc.Validate(ctx, openapi3.DisableExamplesValidation(), openapi3.AllowExtraSiblingFields(extraFields...)); err != nil {
		return fmt.Errorf("validate contract: %w", err)
	}
	return nil
}

// View returns the YAML of the contract with every external reference replaced
// by an open object schema and every untyped string enum typed as string.
// References inside the document, which start with "#", stay unchanged.
func View(data []byte) ([]byte, error) {
	var root yaml.Node
	if err := yaml.Unmarshal(data, &root); err != nil {
		return nil, fmt.Errorf("parse YAML: %w", err)
	}
	rewrite(&root)
	var buf bytes.Buffer
	enc := yaml.NewEncoder(&buf)
	enc.SetIndent(2)
	if err := enc.Encode(&root); err != nil {
		return nil, fmt.Errorf("encode YAML: %w", err)
	}
	if err := enc.Close(); err != nil {
		return nil, fmt.Errorf("encode YAML: %w", err)
	}
	return buf.Bytes(), nil
}

// rewrite walks n, replaces each mapping whose $ref points outside the
// document, and types each untyped string enum.
func rewrite(n *yaml.Node) {
	if n.Kind == yaml.MappingNode {
		if ref := value(n, "$ref"); ref != nil && ref.Kind == yaml.ScalarNode && !strings.HasPrefix(ref.Value, "#") {
			n.Content = []*yaml.Node{
				scalar("type"), scalar("object"),
				scalar(ExternalSchemaExtension), scalar(ref.Value),
			}
			return
		}
		if enum := value(n, "enum"); enum != nil && value(n, "type") == nil && stringSequence(enum) {
			n.Content = append(n.Content, scalar("type"), scalar("string"))
		}
	}
	for _, c := range n.Content {
		rewrite(c)
	}
}

// value returns the value of key in the mapping n, or nil.
func value(n *yaml.Node, key string) *yaml.Node {
	for i := 0; i+1 < len(n.Content); i += 2 {
		if n.Content[i].Value == key {
			return n.Content[i+1]
		}
	}
	return nil
}

// stringSequence reports whether n is a non-empty sequence of string scalars.
func stringSequence(n *yaml.Node) bool {
	if n.Kind != yaml.SequenceNode || len(n.Content) == 0 {
		return false
	}
	for _, c := range n.Content {
		if c.Kind != yaml.ScalarNode || c.Tag != "!!str" {
			return false
		}
	}
	return true
}

func scalar(s string) *yaml.Node {
	return &yaml.Node{Kind: yaml.ScalarNode, Tag: "!!str", Value: s}
}
