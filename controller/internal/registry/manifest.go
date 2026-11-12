// SPDX-License-Identifier: GPL-3.0-only

package registry

import (
	"bytes"
	_ "embed" // the manifest schema is embedded below
	"encoding/json"
	"errors"
	"fmt"
	"sort"
	"strings"

	"github.com/dlclark/regexp2"
	"github.com/santhosh-tekuri/jsonschema/v6"
	"go.yaml.in/yaml/v3"
)

// ManifestFile is the name of the manifest at the root of every module tree
// and module archive.
const ManifestFile = "manifest.yaml"

// manifestSchemaJSON is a copy of contracts/schemas/module-manifest.schema.json.
// TestEmbeddedSchemaMatchesContract fails when the two differ, and
// go generate refreshes the copy.
//
//go:generate cp ../../../contracts/schemas/module-manifest.schema.json schema/module-manifest.schema.json
//go:embed schema/module-manifest.schema.json
var manifestSchemaJSON []byte

// manifestSchema is the compiled manifest schema.
var manifestSchema = mustCompileManifestSchema()

func mustCompileManifestSchema() *jsonschema.Schema {
	doc, err := jsonschema.UnmarshalJSON(bytes.NewReader(manifestSchemaJSON))
	if err != nil {
		panic("invariant: embedded manifest schema is valid JSON: " + err.Error())
	}
	c := jsonschema.NewCompiler()
	// JSON Schema patterns follow ECMA-262, and the relpath pattern of the
	// manifest uses lookahead, which the RE2 engine of package regexp lacks.
	c.UseRegexpEngine(compileECMARegexp)
	const url = "module-manifest.schema.json"
	if err := c.AddResource(url, doc); err != nil {
		panic("invariant: embedded manifest schema loads: " + err.Error())
	}
	s, err := c.Compile(url)
	if err != nil {
		panic("invariant: embedded manifest schema compiles: " + err.Error())
	}
	return s
}

type ecmaRegexp regexp2.Regexp

func (re *ecmaRegexp) MatchString(s string) bool {
	ok, err := (*regexp2.Regexp)(re).MatchString(s)
	return err == nil && ok
}

func (re *ecmaRegexp) String() string { return (*regexp2.Regexp)(re).String() }

func compileECMARegexp(s string) (jsonschema.Regexp, error) {
	re, err := regexp2.Compile(s, regexp2.ECMAScript)
	if err != nil {
		return nil, fmt.Errorf("compile pattern %q: %w", s, err)
	}
	return (*ecmaRegexp)(re), nil
}

// Manifest is a validated module manifest. The typed fields hold what the
// controller reads, and Raw holds the whole manifest as JSON, which getModule
// returns and which validates against the manifest schema. ExpectedBytes holds
// the estimated application payload bytes per session of each scenario with
// default parameters, keyed by scenario name, and the scenario catalog passes
// it to the compiler as the bytes per session.
type Manifest struct {
	Name          string           `json:"name"`
	Version       string           `json:"version"`
	Type          string           `json:"type"`
	Summary       string           `json:"summary,omitempty"`
	EngineAPI     string           `json:"engine_api"`
	License       string           `json:"license"`
	Scenarios     []string         `json:"scenarios,omitempty"`
	ExpectedBytes map[string]int64 `json:"expected_bytes,omitempty"`
	Requires      []Require        `json:"requires,omitempty"`
	Functions     []Function       `json:"functions,omitempty"`
	Assets        []Asset          `json:"assets,omitempty"`
	Extension     *Extension       `json:"extension,omitempty"`
	Fixtures      []Fixture        `json:"fixtures,omitempty"`
	Parameters    json.RawMessage  `json:"parameters,omitempty"`

	// Raw is the manifest as canonical JSON with sorted keys.
	Raw json.RawMessage `json:"-"`
}

// Require names a module that this module's scenarios call.
type Require struct {
	Module  string `json:"module"`
	Version string `json:"version"`
}

// Function is one template function of the module's function catalog.
type Function struct {
	Name    string  `json:"name"`
	Summary string  `json:"summary"`
	Params  []Param `json:"params"`
}

// Param is one positional parameter of a template function. A parameter
// without a default is required.
type Param struct {
	Name    string `json:"name"`
	Type    string `json:"type"`
	Default any    `json:"default,omitempty"`
}

// Asset is a named content asset, either a file or a generator.
type Asset struct {
	Name      string `json:"name"`
	Path      string `json:"path,omitempty"`
	Generator string `json:"generator,omitempty"`
}

// Extension names the WebAssembly component and the hooks it exports.
type Extension struct {
	Wasm  string   `json:"wasm"`
	Hooks []string `json:"hooks"`
}

// Fixture is a realism fixture capture and its expected classifications.
type Fixture struct {
	Capture string `json:"capture"`
}

// Files returns every file path relative to the manifest that the manifest
// names, sorted and without duplicates.
func (m Manifest) Files() []string {
	set := map[string]struct{}{}
	for _, s := range m.Scenarios {
		set[s] = struct{}{}
	}
	for _, a := range m.Assets {
		if a.Path != "" {
			set[a.Path] = struct{}{}
		}
	}
	if m.Extension != nil {
		set[m.Extension.Wasm] = struct{}{}
	}
	for _, f := range m.Fixtures {
		set[f.Capture] = struct{}{}
	}
	out := make([]string, 0, len(set))
	for p := range set {
		out = append(out, p)
	}
	sort.Strings(out)
	return out
}

// ValidationError lists every problem that a manifest or a module tree has.
// Each finding names a JSON Pointer into the manifest, or a path in the module
// tree prefixed with "file:".
type ValidationError struct {
	Module   string
	Findings []Finding
}

// Finding is one problem of a manifest.
type Finding struct {
	Path    string
	Message string
}

func (e *ValidationError) Error() string {
	parts := make([]string, len(e.Findings))
	for i, f := range e.Findings {
		parts[i] = f.Path + ": " + f.Message
	}
	name := e.Module
	if name == "" {
		name = "module"
	}
	return fmt.Sprintf("%s is invalid: %s", name, strings.Join(parts, "; "))
}

// ParseManifest decodes a manifest in YAML or JSON, validates it against the
// manifest schema, and checks its license with CheckLicense and the semantic
// rules of contracts/README.md that the schema cannot express. Every failure is
// a *ValidationError, whose findings name the field.
func ParseManifest(data []byte) (Manifest, error) {
	var doc any
	if err := yaml.Unmarshal(data, &doc); err != nil {
		return Manifest{}, invalid("", "", "manifest is not valid YAML: "+err.Error())
	}
	doc, err := jsonValue(doc, "")
	if err != nil {
		return Manifest{}, invalid("", "", err.Error())
	}
	raw, err := json.Marshal(doc)
	if err != nil {
		return Manifest{}, invalid("", "", "manifest cannot be encoded as JSON: "+err.Error())
	}
	inst, err := jsonschema.UnmarshalJSON(bytes.NewReader(raw))
	if err != nil {
		return Manifest{}, invalid("", "", "manifest cannot be decoded as JSON: "+err.Error())
	}
	var m Manifest
	if err := json.Unmarshal(raw, &m); err != nil {
		// A manifest whose fields have the wrong JSON types still reaches
		// schema validation below, which names the field.
		m = Manifest{}
	}
	m.Raw = raw
	verr := &ValidationError{Module: moduleLabel(m)}
	if err := manifestSchema.Validate(inst); err != nil {
		var se *jsonschema.ValidationError
		if !errors.As(err, &se) {
			return Manifest{}, fmt.Errorf("validate manifest: %w", err)
		}
		verr.Findings = append(verr.Findings, schemaFindings(se)...)
		return Manifest{}, verr
	}
	if err := CheckLicense(m.License); err != nil {
		verr.Findings = append(verr.Findings, Finding{Path: "/license", Message: err.Error()})
	}
	if len(m.Functions) > 0 && m.Extension != nil && !contains(m.Extension.Hooks, "render") {
		verr.Findings = append(verr.Findings, Finding{
			Path:    "/extension/hooks",
			Message: "a manifest that declares functions and an extension must list the render hook",
		})
	}
	seen := map[string]bool{}
	for i, f := range m.Functions {
		if seen[f.Name] {
			verr.Findings = append(verr.Findings, Finding{
				Path:    fmt.Sprintf("/functions/%d/name", i),
				Message: fmt.Sprintf("function %s is declared twice", f.Name),
			})
		}
		seen[f.Name] = true
	}
	if reservedModuleNames[m.Name] {
		verr.Findings = append(verr.Findings, Finding{
			Path:    "/name",
			Message: fmt.Sprintf("module name %s is reserved by the template grammar", m.Name),
		})
	}
	if len(verr.Findings) > 0 {
		return Manifest{}, verr
	}
	return m, nil
}

// reservedModuleNames are the names that templates cannot tell apart from
// references and literals, rule 5 of the template grammar.
var reservedModuleNames = map[string]bool{"vars": true, "session": true, "last": true, "true": true, "false": true}

func moduleLabel(m Manifest) string {
	if m.Name == "" {
		return ""
	}
	if m.Version == "" {
		return "module " + m.Name
	}
	return "module " + m.Name + " " + m.Version
}

func invalid(module, path, msg string) *ValidationError {
	return &ValidationError{Module: module, Findings: []Finding{{Path: path, Message: msg}}}
}

// schemaFindings flattens a schema validation error into one finding per
// failed leaf, with the JSON Pointer of the instance location.
func schemaFindings(e *jsonschema.ValidationError) []Finding {
	var out []Finding
	var walk func(*jsonschema.ValidationError)
	walk = func(e *jsonschema.ValidationError) {
		if len(e.Causes) == 0 {
			msg := "is invalid"
			if u := e.BasicOutput(); u.Error != nil {
				msg = u.Error.String()
			}
			out = append(out, Finding{Path: pointer(e.InstanceLocation), Message: msg})
			return
		}
		for _, c := range e.Causes {
			walk(c)
		}
	}
	walk(e)
	sort.SliceStable(out, func(i, j int) bool { return out[i].Path < out[j].Path })
	return out
}

func pointer(tokens []string) string {
	var b strings.Builder
	for _, t := range tokens {
		b.WriteByte('/')
		b.WriteString(strings.NewReplacer("~", "~0", "/", "~1").Replace(t))
	}
	return b.String()
}

// jsonValue converts a decoded YAML document into the JSON data model. It
// rejects mappings whose keys are not strings, and it turns other YAML scalars,
// such as timestamps, into strings.
func jsonValue(v any, at string) (any, error) {
	switch x := v.(type) {
	case map[string]any:
		out := make(map[string]any, len(x))
		for k, e := range x {
			c, err := jsonValue(e, at+"/"+k)
			if err != nil {
				return nil, err
			}
			out[k] = c
		}
		return out, nil
	case map[any]any:
		return nil, fmt.Errorf("%s: mapping keys must be strings", orRoot(at))
	case []any:
		out := make([]any, len(x))
		for i, e := range x {
			c, err := jsonValue(e, fmt.Sprintf("%s/%d", at, i))
			if err != nil {
				return nil, err
			}
			out[i] = c
		}
		return out, nil
	case nil, string, bool, int, int64, uint64, float64:
		return x, nil
	default:
		return fmt.Sprint(x), nil
	}
}

func orRoot(at string) string {
	if at == "" {
		return "manifest"
	}
	return at
}

func contains(list []string, s string) bool {
	for _, e := range list {
		if e == s {
			return true
		}
	}
	return false
}
