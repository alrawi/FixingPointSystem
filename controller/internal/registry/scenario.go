// SPDX-License-Identifier: GPL-3.0-only

package registry

import (
	"bytes"
	_ "embed" // the scenario schema is embedded below
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"regexp"
	"strconv"
	"strings"

	"github.com/santhosh-tekuri/jsonschema/v6"
	"go.yaml.in/yaml/v3"
)

// scenarioSchemaJSON is a copy of contracts/schemas/scenario.schema.json, which
// is scenario contract v0.3. TestEmbeddedScenarioSchemaMatchesContract fails
// when the two differ, and go generate refreshes the copy.
//
//go:generate cp ../../../contracts/schemas/scenario.schema.json schema/scenario.schema.json
//go:embed schema/scenario.schema.json
var scenarioSchemaJSON []byte

// scenarioSchema is the compiled scenario schema.
var scenarioSchema = mustCompileScenarioSchema()

func mustCompileScenarioSchema() *jsonschema.Schema {
	doc, err := jsonschema.UnmarshalJSON(bytes.NewReader(scenarioSchemaJSON))
	if err != nil {
		panic("invariant: embedded scenario schema is valid JSON: " + err.Error())
	}
	c := jsonschema.NewCompiler()
	c.UseRegexpEngine(compileECMARegexp)
	const url = "scenario.schema.json"
	if err := c.AddResource(url, doc); err != nil {
		panic("invariant: embedded scenario schema loads: " + err.Error())
	}
	s, err := c.Compile(url)
	if err != nil {
		panic("invariant: embedded scenario schema compiles: " + err.Error())
	}
	return s
}

// pendingDir holds the scenarios of a module that use syntax the engine does
// not compile yet. The manifest does not list them, the registry checks them
// like the listed ones and does not index them.
const pendingDir = "scenarios-pending"

// checkScenarioFile validates one scenario file against the scenario schema and
// checks the v0.3 template rules that the schema cannot express. The findings
// carry paths of the form file:<path>#<JSON Pointer>. A file that cannot be
// read as a regular file is a finding too, except that a missing file returns
// the error from fs.ReadFile, because the caller reports it already.
func checkScenarioFile(fsys fs.FS, p string) ([]Finding, error) {
	data, err := fs.ReadFile(fsys, p)
	if err != nil {
		return nil, err
	}
	at := func(ptr string) string { return "file:" + p + "#" + ptr }
	var doc any
	if err := yaml.Unmarshal(data, &doc); err != nil {
		return []Finding{{Path: at(""), Message: "scenario file is not valid YAML: " + err.Error()}}, nil
	}
	doc, err = jsonValue(doc, "")
	if err != nil {
		return []Finding{{Path: at(""), Message: err.Error()}}, nil
	}
	raw, err := json.Marshal(doc)
	if err != nil {
		return []Finding{{Path: at(""), Message: "scenario cannot be encoded as JSON: " + err.Error()}}, nil
	}
	inst, err := jsonschema.UnmarshalJSON(bytes.NewReader(raw))
	if err != nil {
		return []Finding{{Path: at(""), Message: "scenario cannot be decoded as JSON: " + err.Error()}}, nil
	}
	if err := scenarioSchema.Validate(inst); err != nil {
		var se *jsonschema.ValidationError
		if !errors.As(err, &se) {
			return nil, fmt.Errorf("validate scenario %s: %w", p, err)
		}
		found := schemaFindings(se)
		for i := range found {
			found[i].Path = at(found[i].Path)
		}
		// The template rules assume a document of the right shape.
		return found, nil
	}
	root, _ := doc.(map[string]any)
	var out []Finding
	for _, is := range checkScenarioTemplates(root) {
		out = append(out, Finding{Path: at(is.at), Message: is.msg})
	}
	return out, nil
}

// tplIssue is a template problem with the JSON Pointer of its field.
type tplIssue struct {
	at, msg string
}

// scenarioScope is what the template checks need to know about a scenario.
type scenarioScope struct {
	profiles map[string]bool
	// vars maps a variable to its value type, or to any for a variable that a
	// set action binds.
	vars map[string]string
}

// checkScenarioTemplates applies the template rules of scenario contract v0.3
// to a scenario that already passed the schema. It checks what the controller
// can judge without the module manifests, which are references to profiles and
// the distribution functions dist.<name>(...). Module calls and the other
// references are left to the contract validator.
func checkScenarioTemplates(doc map[string]any) []tplIssue {
	sc := scenarioScope{profiles: map[string]bool{}, vars: map[string]string{}}
	if ps, ok := doc["profiles"].(map[string]any); ok {
		for name := range ps {
			sc.profiles[name] = true
		}
	}
	if vs, ok := doc["vars"].(map[string]any); ok {
		for name, v := range vs {
			sc.vars[name] = valueType(v)
		}
	}
	flows, _ := doc["flows"].([]any)
	for _, f := range flows {
		fm, _ := f.(map[string]any)
		for _, side := range []string{"client", "server"} {
			collectSetNames(fm[side], sc.vars)
		}
	}
	var out []tplIssue
	for i, f := range flows {
		fm, _ := f.(map[string]any)
		for _, side := range []string{"client", "server"} {
			acts, _ := fm[side].([]any)
			out = append(out, checkActions(acts, []string{"flows", strconv.Itoa(i), side}, sc)...)
		}
	}
	return out
}

// collectSetNames records the variables that set actions bind, at any depth.
func collectSetNames(actions any, vars map[string]string) {
	acts, _ := actions.([]any)
	for _, a := range acts {
		kind, body, ok := singleAction(a)
		if !ok {
			continue
		}
		switch kind {
		case "set":
			if m, ok := body.(map[string]any); ok {
				for name := range m {
					vars[name] = "any"
				}
			}
		case "loop":
			if m, ok := body.(map[string]any); ok {
				collectSetNames(m["do"], vars)
			}
		case "branch":
			arms, _ := body.([]any)
			for _, arm := range arms {
				if m, ok := arm.(map[string]any); ok {
					collectSetNames(m["do"], vars)
				}
			}
		}
	}
}

// singleAction returns the kind and body of an action, which is a mapping with
// exactly one key.
func singleAction(a any) (string, any, bool) {
	m, ok := a.(map[string]any)
	if !ok || len(m) != 1 {
		return "", nil, false
	}
	for k, v := range m {
		return k, v, true
	}
	return "", nil, false
}

// templateKeys are the fields of an action body that hold a template, as in
// the contract validator.
var templateKeys = []string{"host", "port", "bytes", "min_bytes", "count", "ms"}

func checkActions(actions []any, path []string, sc scenarioScope) []tplIssue {
	var out []tplIssue
	for i, a := range actions {
		kind, body, ok := singleAction(a)
		if !ok {
			continue
		}
		here := append(append([]string(nil), path...), strconv.Itoa(i), kind)
		field := func(key, text string) {
			p := here
			if key != "" {
				p = append(append([]string(nil), here...), key)
			}
			for _, msg := range checkTemplate(text, sc) {
				out = append(out, tplIssue{at: pointer(p), msg: msg})
			}
		}
		switch kind {
		case "send":
			if s, ok := body.(string); ok {
				field("", s)
			}
		case "connect", "accept", "expect", "loop", "delay":
			if m, ok := body.(map[string]any); ok {
				for _, key := range templateKeys {
					if s, ok := m[key].(string); ok {
						field(key, s)
					}
				}
			}
		case "set":
			if m, ok := body.(map[string]any); ok {
				for key, v := range m {
					if s, ok := v.(string); ok {
						field(key, s)
					}
				}
			}
		}
		switch kind {
		case "loop":
			if m, ok := body.(map[string]any); ok {
				inner, _ := m["do"].([]any)
				out = append(out, checkActions(inner, append(append([]string(nil), here...), "do"), sc)...)
			}
		case "branch":
			arms, _ := body.([]any)
			for j, arm := range arms {
				if m, ok := arm.(map[string]any); ok {
					inner, _ := m["do"].([]any)
					out = append(out, checkActions(inner, append(append([]string(nil), here...), strconv.Itoa(j), "do"), sc)...)
				}
			}
		}
	}
	return out
}

// valueType is the template type of a scenario variable.
func valueType(v any) string {
	switch v.(type) {
	case bool:
		return "boolean"
	case int, int64, uint64:
		return "int"
	case float64:
		return "float"
	default:
		return "text"
	}
}

// placeholder matches one {{ expression }}, which ends at the first }}.
var placeholder = regexp.MustCompile(`(?s)\{\{(.*?)\}\}`)

// checkTemplate checks every placeholder of a template string.
func checkTemplate(text string, sc scenarioScope) []string {
	var out []string
	rest := placeholder.ReplaceAllString(text, "")
	if strings.Contains(rest, "{{") || strings.Contains(rest, "}}") {
		out = append(out, "unbalanced '{{' or '}}'")
	}
	for _, m := range placeholder.FindAllStringSubmatch(text, -1) {
		out = append(out, checkExpression(m[1], sc)...)
	}
	return out
}

func checkExpression(expr string, sc scenarioScope) []string {
	if strings.TrimSpace(expr) == "" {
		return []string{"empty placeholder"}
	}
	n, err := parseExpression(expr)
	if err != nil {
		return []string{fmt.Sprintf("cannot parse expression '%s': %s", strings.TrimSpace(expr), err.Error())}
	}
	var issues []string
	checkNode(n, sc, &issues)
	return issues
}

// Expression nodes of the template grammar.
type (
	exprNode any
	litNode  struct {
		typ string // int, float, text, or boolean
		num float64
		str string // the value of a text literal
	}
	refNode  struct{ parts []string }
	callNode struct {
		module, name string // module is empty for a core function
		pos          []exprNode
		kw           []kwArg
	}
	kwArg struct {
		name string
		val  exprNode
	}
)

func (c callNode) qualified() string {
	if c.module != "" {
		return c.module + "." + c.name
	}
	return c.name
}

// maxCallDepth is the nesting limit of calls, counting the outermost as 1.
const maxCallDepth = 8

type token struct{ kind, text string }

var (
	numberRE = regexp.MustCompile(`^-?[0-9]+(\.[0-9]+)?`)
	identRE  = regexp.MustCompile(`^[A-Za-z_][A-Za-z0-9_]*`)
)

func tokenize(text string) ([]token, error) {
	var toks []token
	for i := 0; i < len(text); {
		ch := text[i]
		switch {
		case ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r' || ch == '\f' || ch == '\v':
			i++
		case strings.IndexByte("().,=", ch) >= 0:
			toks = append(toks, token{string(ch), string(ch)})
			i++
		case ch == '\'':
			end := strings.IndexByte(text[i+1:], '\'')
			if end < 0 {
				return nil, errors.New("unterminated string literal")
			}
			toks = append(toks, token{"str", text[i+1 : i+1+end]})
			i += end + 2
		default:
			if m := numberRE.FindString(text[i:]); m != "" {
				kind := "int"
				if strings.Contains(m, ".") {
					kind = "float"
				}
				toks = append(toks, token{kind, m})
				i += len(m)
			} else if m := identRE.FindString(text[i:]); m != "" {
				toks = append(toks, token{"ident", m})
				i += len(m)
			} else {
				return nil, fmt.Errorf("unexpected character '%c'", ch)
			}
		}
	}
	return toks, nil
}

type parser struct {
	toks []token
	pos  int
}

func (p *parser) peek(off int) string {
	if j := p.pos + off; j < len(p.toks) {
		return p.toks[j].kind
	}
	return ""
}

func (p *parser) take(kind string) (string, error) {
	if p.peek(0) != kind {
		got := "end of expression"
		if p.pos < len(p.toks) {
			got = p.toks[p.pos].text
		}
		return "", fmt.Errorf("expected '%s', got '%s'", kind, got)
	}
	v := p.toks[p.pos].text
	p.pos++
	return v, nil
}

func parseExpression(text string) (exprNode, error) {
	toks, err := tokenize(text)
	if err != nil {
		return nil, err
	}
	p := &parser{toks: toks}
	n, err := p.expression(1)
	if err != nil {
		return nil, err
	}
	if p.pos < len(p.toks) {
		return nil, fmt.Errorf("unexpected '%s'", p.toks[p.pos].text)
	}
	return n, nil
}

func (p *parser) expression(depth int) (exprNode, error) {
	switch kind := p.peek(0); kind {
	case "int", "float":
		text, _ := p.take(kind)
		f, err := strconv.ParseFloat(text, 64)
		if err != nil && !errors.Is(err, strconv.ErrRange) {
			return nil, fmt.Errorf("bad number '%s'", text)
		}
		return litNode{typ: kind, num: f}, nil
	case "str":
		text, _ := p.take("str")
		return litNode{typ: "text", str: text}, nil
	case "ident":
	default:
		return nil, errors.New("expected a literal, a reference, or a call")
	}
	first, _ := p.take("ident")
	if (first == "true" || first == "false") && p.peek(0) != "." && p.peek(0) != "(" {
		return litNode{typ: "boolean"}, nil
	}
	parts := []string{first}
	for p.peek(0) == "." {
		_, _ = p.take(".")
		id, err := p.take("ident")
		if err != nil {
			return nil, err
		}
		parts = append(parts, id)
	}
	if p.peek(0) != "(" {
		return refNode{parts: parts}, nil
	}
	if len(parts) > 2 {
		return nil, fmt.Errorf("'%s' is not a function name; use <module>.<function>", strings.Join(parts, "."))
	}
	if depth > maxCallDepth {
		return nil, fmt.Errorf("calls nest deeper than %d levels", maxCallDepth)
	}
	_, _ = p.take("(")
	call := callNode{name: parts[len(parts)-1]}
	if len(parts) == 2 {
		call.module = parts[0]
	}
	if p.peek(0) != ")" {
		for {
			if p.peek(0) == "ident" && p.peek(1) == "=" {
				name, _ := p.take("ident")
				_, _ = p.take("=")
				v, err := p.expression(depth + 1)
				if err != nil {
					return nil, err
				}
				call.kw = append(call.kw, kwArg{name, v})
			} else if len(call.kw) > 0 {
				return nil, errors.New("a positional argument follows a keyword argument")
			} else {
				v, err := p.expression(depth + 1)
				if err != nil {
					return nil, err
				}
				call.pos = append(call.pos, v)
			}
			if p.peek(0) != "," {
				break
			}
			_, _ = p.take(",")
		}
	}
	if _, err := p.take(")"); err != nil {
		return nil, err
	}
	return call, nil
}

// bound is an argument that a call binds to a parameter.
type bound struct {
	typ  string
	node exprNode
}

func hasKey(m map[string]bound, k string) bool {
	_, ok := m[k]
	return ok
}

// distParam is one parameter of a distribution function.
type distParam struct {
	name     string
	typ      string
	required bool
}

// distFunctions is the catalog of scenario contract v0.3. Each function returns
// an int, and cap defaults to 2,147,483,647.
var distFunctions = map[string][]distParam{
	"pareto":      {{"alpha", "float", true}, {"scale", "float", true}, {"cap", "int", false}},
	"lognormal":   {{"mu", "float", true}, {"sigma", "float", true}, {"cap", "int", false}},
	"exponential": {{"mean", "float", true}, {"cap", "int", false}},
	"uniform":     {{"low", "float", true}, {"high", "float", true}},
}

// distNames lists the distribution functions for messages, sorted.
const distNames = "exponential, lognormal, pareto, uniform"

var (
	distPositive    = map[string]bool{"alpha": true, "scale": true, "sigma": true, "mean": true, "cap": true}
	distNonNegative = map[string]bool{"low": true, "high": true}
)

// coreResult is the result type of the core functions. now is not in the
// table, because its type follows its literal format, see nowResult.
var coreResult = map[string]string{"fill": "text", "hex": "bytes", "rand_int": "int", "int_bytes": "bytes"}

// nowResult is the type that now(format) renders for each literal format
// (scenario contract v0.6).
var nowResult = map[string]string{"http_date": "text", "epoch_s": "int", "epoch_ms": "int"}

// coreCallResult returns the type of a call of a core function. A call of now
// gets the type that its literal format fixes, and any for a format that is
// not a literal or not known, which the contract validator reports.
func coreCallResult(c callNode) string {
	if c.name == "now" {
		var format exprNode
		switch {
		case len(c.pos) > 0:
			format = c.pos[0]
		default:
			for _, k := range c.kw {
				if k.name == "format" {
					format = k.val
				}
			}
		}
		if lit, ok := format.(litNode); ok && lit.typ == "text" {
			if t, known := nowResult[lit.str]; known {
				return t
			}
		}
		return "any"
	}
	if t, ok := coreResult[c.name]; ok {
		return t
	}
	return "any"
}

var sessionFields = map[string]string{"id": "int", "flow": "text"}

func compatible(param, arg string) bool {
	if param == "any" || arg == "any" || param == arg {
		return true
	}
	return (param == "float" && arg == "int") || (param == "bytes" && arg == "text")
}

// checkNode checks the profile and dist rules inside n and returns its type.
func checkNode(n exprNode, sc scenarioScope, issues *[]string) string {
	switch x := n.(type) {
	case litNode:
		return x.typ
	case refNode:
		return checkRef(x, sc, issues)
	case callNode:
		return checkCall(x, sc, issues)
	}
	return "any"
}

func checkRef(r refNode, sc scenarioScope, issues *[]string) string {
	text := strings.Join(r.parts, ".")
	switch r.parts[0] {
	case "profiles":
		if len(r.parts) != 2 {
			*issues = append(*issues, fmt.Sprintf("'%s' must have the form profiles.<name>", text))
		} else if !sc.profiles[r.parts[1]] {
			*issues = append(*issues, fmt.Sprintf("undefined profile '%s'", r.parts[1]))
		}
		return "text"
	case "dist":
		*issues = append(*issues, fmt.Sprintf("'%s' names a distribution function; call it with parentheses", text))
		return "any"
	case "vars":
		if len(r.parts) == 2 {
			if t, ok := sc.vars[r.parts[1]]; ok {
				return t
			}
		}
		return "any"
	case "session":
		if len(r.parts) == 2 {
			if t, ok := sessionFields[r.parts[1]]; ok {
				return t
			}
		}
		return "any"
	case "last":
		return "bytes"
	}
	return "any"
}

func checkCall(c callNode, sc scenarioScope, issues *[]string) string {
	argTypes := make([]string, 0, len(c.pos)+len(c.kw))
	for _, a := range c.pos {
		argTypes = append(argTypes, checkNode(a, sc, issues))
	}
	for _, k := range c.kw {
		argTypes = append(argTypes, checkNode(k.val, sc, issues))
	}
	if c.module != "dist" {
		// Module and core calls belong to the contract validator, which has
		// the manifests. Their results only need a type for outer dist calls.
		if c.module != "" {
			return "bytes"
		}
		return coreCallResult(c)
	}
	params, ok := distFunctions[c.name]
	if !ok {
		*issues = append(*issues, fmt.Sprintf("unknown distribution '%s', expected one of %s", c.name, distNames))
		return "int"
	}
	q := c.qualified()
	got := map[string]bound{}
	if len(c.pos) > len(params) {
		*issues = append(*issues, fmt.Sprintf("function '%s' takes at most %d positional argument(s), got %d", q, len(params), len(c.pos)))
	}
	for i := range c.pos {
		if i < len(params) {
			got[params[i].name] = bound{argTypes[i], c.pos[i]}
		}
	}
	known := map[string]bool{}
	for _, p := range params {
		known[p.name] = true
	}
	for j, k := range c.kw {
		typ := argTypes[len(c.pos)+j]
		switch {
		case !known[k.name]:
			*issues = append(*issues, fmt.Sprintf("function '%s' has no parameter '%s'", q, k.name))
		case hasKey(got, k.name):
			*issues = append(*issues, fmt.Sprintf("function '%s' got parameter '%s' twice", q, k.name))
		default:
			got[k.name] = bound{typ, k.val}
		}
	}
	lits := map[string]float64{}
	for _, p := range params {
		b, ok := got[p.name]
		if !ok {
			if p.required {
				*issues = append(*issues, fmt.Sprintf("function '%s' misses required parameter '%s'", q, p.name))
			}
			continue
		}
		if !compatible(p.typ, b.typ) {
			*issues = append(*issues, fmt.Sprintf("parameter '%s' of '%s' takes %s, got %s", p.name, q, p.typ, b.typ))
		}
		if l, ok := b.node.(litNode); ok && (l.typ == "int" || l.typ == "float") {
			lits[p.name] = l.num
		}
	}
	for _, p := range params {
		v, ok := lits[p.name]
		if !ok {
			continue
		}
		if distPositive[p.name] && v <= 0 {
			*issues = append(*issues, fmt.Sprintf("parameter '%s' of '%s' must be positive", p.name, q))
		}
		if distNonNegative[p.name] && v < 0 {
			*issues = append(*issues, fmt.Sprintf("parameter '%s' of '%s' must not be negative", p.name, q))
		}
	}
	if low, ok := lits["low"]; ok {
		if high, ok := lits["high"]; ok && low > high {
			*issues = append(*issues, "uniform() has low greater than high")
		}
	}
	return "int"
}
