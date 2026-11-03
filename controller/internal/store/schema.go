// SPDX-License-Identifier: GPL-3.0-only

package store

import (
	"bytes"
	_ "embed" // the test configuration schema
	"errors"
	"fmt"
	"sort"
	"strings"

	"github.com/santhosh-tekuri/jsonschema/v6"
	"github.com/santhosh-tekuri/jsonschema/v6/kind"
	"golang.org/x/text/language"
	"golang.org/x/text/message"

	"github.com/alrawi/FixingPointSystem/controller/internal/api"
)

// testConfigSchema is a copy of contracts/schemas/test-config.schema.json,
// which go generate refreshes and a test compares with the contract.
//
//go:embed schema/test-config.schema.json
var testConfigSchema []byte

// SchemaURL is the $id of the test configuration schema.
const SchemaURL = "https://github.com/alrawi/FixingPointSystem/contracts/schemas/test-config.schema.json"

// Schema checks documents against the test configuration schema.
type Schema struct {
	sch     *jsonschema.Schema
	printer *message.Printer
}

// NewSchema compiles the embedded test configuration schema.
func NewSchema() (*Schema, error) {
	return compileSchema(testConfigSchema)
}

func compileSchema(src []byte) (*Schema, error) {
	doc, err := jsonschema.UnmarshalJSON(bytes.NewReader(src))
	if err != nil {
		return nil, fmt.Errorf("parse test configuration schema: %w", err)
	}
	c := jsonschema.NewCompiler()
	if err := c.AddResource(SchemaURL, doc); err != nil {
		return nil, fmt.Errorf("add test configuration schema: %w", err)
	}
	sch, err := c.Compile(SchemaURL)
	if err != nil {
		return nil, fmt.Errorf("compile test configuration schema: %w", err)
	}
	return &Schema{sch: sch, printer: message.NewPrinter(language.English)}, nil
}

// Check validates the canonical JSON document doc and returns one error
// finding per failing field, sorted by path. An empty result means the
// document conforms.
func (s *Schema) Check(doc []byte) ([]api.Finding, error) {
	inst, err := jsonschema.UnmarshalJSON(bytes.NewReader(doc))
	if err != nil {
		return nil, fmt.Errorf("%w: %w", ErrMalformedBody, err)
	}
	err = s.sch.Validate(inst)
	if err == nil {
		return nil, nil
	}
	var ve *jsonschema.ValidationError
	if !errors.As(err, &ve) {
		return nil, fmt.Errorf("validate test configuration: %w", err)
	}
	var out []api.Finding
	s.leaves(ve, &out)
	return sortFindings(out), nil
}

// leaves appends a finding for each leaf of the error tree. A missing or an
// unknown property becomes a finding at the property itself, so the path
// names the field rather than its parent object.
func (s *Schema) leaves(ve *jsonschema.ValidationError, out *[]api.Finding) {
	if len(ve.Causes) > 0 {
		for _, c := range ve.Causes {
			s.leaves(c, out)
		}
		return
	}
	at := pointer(ve.InstanceLocation)
	switch k := ve.ErrorKind.(type) {
	case *kind.Required:
		for _, name := range k.Missing {
			*out = append(*out, errorFinding(at+"/"+escapeToken(name), "required property is missing"))
		}
	case *kind.AdditionalProperties:
		for _, name := range k.Properties {
			*out = append(*out, errorFinding(at+"/"+escapeToken(name), "property is not allowed here"))
		}
	default:
		*out = append(*out, errorFinding(at, ve.ErrorKind.LocalizedString(s.printer)))
	}
}

// pointer returns the JSON Pointer of RFC 6901 for tokens.
func pointer(tokens []string) string {
	var sb strings.Builder
	for _, t := range tokens {
		sb.WriteByte('/')
		sb.WriteString(escapeToken(t))
	}
	return sb.String()
}

func escapeToken(t string) string {
	return strings.ReplaceAll(strings.ReplaceAll(t, "~", "~0"), "/", "~1")
}

func errorFinding(path, msg string) api.Finding {
	return api.Finding{Severity: api.FindingSeverityError, Path: path, Message: msg}
}

// sortFindings orders findings by path, then severity and message, and drops
// exact duplicates, so reports are stable.
func sortFindings(in []api.Finding) []api.Finding {
	sort.SliceStable(in, func(i, j int) bool {
		a, b := in[i], in[j]
		if a.Path != b.Path {
			return a.Path < b.Path
		}
		if a.Severity != b.Severity {
			return a.Severity < b.Severity
		}
		return a.Message < b.Message
	})
	out := in[:0]
	for _, f := range in {
		if len(out) > 0 && f == out[len(out)-1] {
			continue
		}
		out = append(out, f)
	}
	return out
}
