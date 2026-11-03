// SPDX-License-Identifier: GPL-3.0-only

package store

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"mime"
	"strconv"
	"strings"

	"go.yaml.in/yaml/v3"
)

// Sentinel errors of the body decoder.
var (
	// ErrUnsupportedMediaType reports a body whose Content-Type is neither
	// JSON nor YAML.
	ErrUnsupportedMediaType = errors.New("unsupported media type")
	// ErrMalformedBody reports a body that is not one well formed JSON or YAML document.
	ErrMalformedBody = errors.New("malformed body")
)

// maxYAMLNodes bounds the nodes that one YAML document may expand to, alias
// expansions included, so that a small body cannot build a huge value.
const maxYAMLNodes = 100_000

// Format is the syntax of a request body.
type Format int

// Body formats that the configuration operations accept.
const (
	FormatJSON Format = iota + 1
	FormatYAML
)

// FormatOf returns the format of the Content-Type value ct. It accepts
// application/json, any +json type, application/yaml, any +yaml type, and the
// legacy names application/x-yaml, text/yaml, and text/x-yaml.
func FormatOf(ct string) (Format, error) {
	mt, _, err := mime.ParseMediaType(ct)
	if err != nil {
		return 0, fmt.Errorf("%w: content type %q", ErrUnsupportedMediaType, ct)
	}
	switch {
	case mt == "application/json" || strings.HasSuffix(mt, "+json"):
		return FormatJSON, nil
	case mt == "application/yaml" || mt == "application/x-yaml" || mt == "text/yaml" ||
		mt == "text/x-yaml" || strings.HasSuffix(mt, "+yaml"):
		return FormatYAML, nil
	}
	return 0, fmt.Errorf("%w: content type %q", ErrUnsupportedMediaType, mt)
}

// Canonical decodes one document in format f and returns it as canonical
// JSON, whose object keys are sorted and whose numbers keep their digits.
func Canonical(data []byte, f Format) ([]byte, error) {
	var v any
	var err error
	switch f {
	case FormatJSON:
		v, err = decodeJSON(data)
	case FormatYAML:
		v, err = decodeYAML(data)
	default:
		return nil, fmt.Errorf("decode body: unknown format %d", f)
	}
	if err != nil {
		return nil, err
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(v); err != nil {
		return nil, fmt.Errorf("%w: encode as JSON: %w", ErrMalformedBody, err)
	}
	return bytes.TrimSuffix(buf.Bytes(), []byte("\n")), nil
}

func decodeJSON(data []byte) (any, error) {
	dec := json.NewDecoder(bytes.NewReader(data))
	dec.UseNumber()
	var v any
	if err := dec.Decode(&v); err != nil {
		if errors.Is(err, io.EOF) {
			return nil, fmt.Errorf("%w: empty JSON document", ErrMalformedBody)
		}
		return nil, fmt.Errorf("%w: JSON: %w", ErrMalformedBody, err)
	}
	if _, err := dec.Token(); !errors.Is(err, io.EOF) {
		return nil, fmt.Errorf("%w: JSON: data after the document at offset %d", ErrMalformedBody, dec.InputOffset())
	}
	return v, nil
}

func decodeYAML(data []byte) (any, error) {
	dec := yaml.NewDecoder(bytes.NewReader(data))
	var doc yaml.Node
	if err := dec.Decode(&doc); err != nil {
		if errors.Is(err, io.EOF) {
			return nil, fmt.Errorf("%w: empty YAML document", ErrMalformedBody)
		}
		return nil, fmt.Errorf("%w: YAML: %w", ErrMalformedBody, err)
	}
	var extra yaml.Node
	if err := dec.Decode(&extra); !errors.Is(err, io.EOF) {
		return nil, fmt.Errorf("%w: YAML: the body holds more than one document", ErrMalformedBody)
	}
	budget := maxYAMLNodes
	return yamlValue(&doc, &budget, 0)
}

// yamlValue converts a YAML node to the value that encoding/json would decode
// from the same document. Every mapping key must be a string, and timestamps
// and other tagged scalars stay strings, as the JSON Schema expects.
func yamlValue(n *yaml.Node, budget *int, depth int) (any, error) {
	*budget--
	if *budget < 0 || depth > 1000 {
		return nil, fmt.Errorf("%w: YAML: document is too large or too deep", ErrMalformedBody)
	}
	switch n.Kind {
	case yaml.DocumentNode:
		if len(n.Content) != 1 {
			return nil, fmt.Errorf("%w: YAML: empty document", ErrMalformedBody)
		}
		return yamlValue(n.Content[0], budget, depth+1)
	case yaml.AliasNode:
		if n.Alias == nil {
			return nil, fmt.Errorf("%w: YAML line %d: unknown alias", ErrMalformedBody, n.Line)
		}
		return yamlValue(n.Alias, budget, depth+1)
	case yaml.SequenceNode:
		out := make([]any, 0, len(n.Content))
		for _, c := range n.Content {
			v, err := yamlValue(c, budget, depth+1)
			if err != nil {
				return nil, err
			}
			out = append(out, v)
		}
		return out, nil
	case yaml.MappingNode:
		if len(n.Content)%2 != 0 {
			return nil, fmt.Errorf("%w: YAML line %d: odd mapping", ErrMalformedBody, n.Line)
		}
		out := make(map[string]any, len(n.Content)/2)
		for i := 0; i < len(n.Content); i += 2 {
			k := n.Content[i]
			if k.Kind != yaml.ScalarNode || (k.ShortTag() != "!!str" && k.ShortTag() != "!!merge") {
				return nil, fmt.Errorf("%w: YAML line %d: mapping key is not a string", ErrMalformedBody, k.Line)
			}
			if _, dup := out[k.Value]; dup {
				return nil, fmt.Errorf("%w: YAML line %d: duplicate key %q", ErrMalformedBody, k.Line, k.Value)
			}
			v, err := yamlValue(n.Content[i+1], budget, depth+1)
			if err != nil {
				return nil, err
			}
			out[k.Value] = v
		}
		return out, nil
	case yaml.ScalarNode:
		return yamlScalar(n)
	}
	return nil, fmt.Errorf("%w: YAML line %d: unknown node kind %d", ErrMalformedBody, n.Line, n.Kind)
}

func yamlScalar(n *yaml.Node) (any, error) {
	switch n.ShortTag() {
	case "!!null":
		return nil, nil
	case "!!bool":
		var b bool
		if err := n.Decode(&b); err != nil {
			return nil, fmt.Errorf("%w: YAML line %d: %w", ErrMalformedBody, n.Line, err)
		}
		return b, nil
	case "!!int":
		var i int64
		if err := n.Decode(&i); err == nil {
			return json.Number(strconv.FormatInt(i, 10)), nil
		}
		var u uint64
		if err := n.Decode(&u); err != nil {
			return nil, fmt.Errorf("%w: YAML line %d: integer %s is out of range", ErrMalformedBody, n.Line, n.Value)
		}
		return json.Number(strconv.FormatUint(u, 10)), nil
	case "!!float":
		var f float64
		if err := n.Decode(&f); err != nil {
			return nil, fmt.Errorf("%w: YAML line %d: %w", ErrMalformedBody, n.Line, err)
		}
		if math.IsNaN(f) || math.IsInf(f, 0) {
			return nil, fmt.Errorf("%w: YAML line %d: %s is not a JSON number", ErrMalformedBody, n.Line, n.Value)
		}
		return json.Number(strconv.FormatFloat(f, 'g', -1, 64)), nil
	}
	return n.Value, nil
}
