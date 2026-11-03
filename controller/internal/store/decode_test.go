// SPDX-License-Identifier: GPL-3.0-only

package store

import (
	"errors"
	"strings"
	"testing"

	"go.yaml.in/yaml/v3"
)

func TestFormatOf(t *testing.T) {
	tests := []struct {
		ct   string
		want Format
	}{
		{"application/json", FormatJSON},
		{"application/json; charset=utf-8", FormatJSON},
		{"application/merge-patch+json", FormatJSON},
		{"application/yaml", FormatYAML},
		{"Application/YAML", FormatYAML},
		{"application/x-yaml", FormatYAML},
		{"text/yaml", FormatYAML},
		{"text/x-yaml", FormatYAML},
	}
	for _, tc := range tests {
		got, err := FormatOf(tc.ct)
		if err != nil || got != tc.want {
			t.Errorf("FormatOf(%q) = %v, %v, want %v", tc.ct, got, err, tc.want)
		}
	}
	for _, ct := range []string{"", "text/plain", "application/xml", ";;"} {
		if _, err := FormatOf(ct); !errors.Is(err, ErrUnsupportedMediaType) {
			t.Errorf("FormatOf(%q) = %v, want ErrUnsupportedMediaType", ct, err)
		}
	}
}

func TestCanonicalYAMLMatchesJSON(t *testing.T) {
	for _, ex := range examples(t) {
		fromYAML := mustCanonical(t, ex.data)
		fromJSON, err := Canonical(fromYAML, FormatJSON)
		if err != nil {
			t.Fatalf("%s: %v", ex.name, err)
		}
		if string(fromJSON) != string(fromYAML) {
			t.Errorf("%s: canonical JSON differs\nyaml %s\njson %s", ex.name, fromYAML, fromJSON)
		}
	}
}

func TestCanonicalValues(t *testing.T) {
	tests := []struct {
		name, in, want string
		f              Format
	}{
		{"sorted keys", `{b: 1, a: 2}`, `{"a":2,"b":1}`, FormatYAML},
		{"big integers keep digits", `v: 18446744073709551615`, `{"v":18446744073709551615}`, FormatYAML},
		{"floats", `v: 0.1`, `{"v":0.1}`, FormatYAML},
		{"integers beyond 64 bits become floats", `v: 999999999999999999999999`, `{"v":1e+24}`, FormatYAML},
		{"timestamps stay strings", `v: 2026-11-03`, `{"v":"2026-11-03"}`, FormatYAML},
		{"null and bool", `{a: null, b: true}`, `{"a":null,"b":true}`, FormatYAML},
		{"aliases expand", "a: &x {k: 1}\nb: *x", `{"a":{"k":1},"b":{"k":1}}`, FormatYAML},
		{"no HTML escaping", `v: "<a&b>"`, `{"v":"<a&b>"}`, FormatYAML},
		{"json numbers keep digits", `{"v": 1.50, "w": 10000000000}`, `{"v":1.50,"w":10000000000}`, FormatJSON},
	}
	for _, tc := range tests {
		got, err := Canonical([]byte(tc.in), tc.f)
		if err != nil || string(got) != tc.want {
			t.Errorf("%s: Canonical = %s, %v, want %s", tc.name, got, err, tc.want)
		}
	}
}

func TestCanonicalRejectsMalformedBodies(t *testing.T) {
	bomb := "a: &a [x, x, x, x, x, x, x, x, x, x]\n"
	for i := 'b'; i <= 'h'; i++ {
		prev := string(i - 1)
		bomb += string(i) + ": &" + string(i) + " [*" + prev + ", *" + prev + ", *" + prev + ", *" + prev + ", *" + prev + ", *" + prev + ", *" + prev + ", *" + prev + ", *" + prev + ", *" + prev + "]\n"
	}
	tests := []struct {
		name, in string
		f        Format
	}{
		{"empty json", ``, FormatJSON},
		{"broken json", `{"a":`, FormatJSON},
		{"trailing json", `{} {}`, FormatJSON},
		{"empty yaml", ``, FormatYAML},
		{"broken yaml", "a: [", FormatYAML},
		{"two documents", "a: 1\n---\nb: 2\n", FormatYAML},
		{"integer key", "1: a", FormatYAML},
		{"sequence key", "? [a]\n: b", FormatYAML},
		{"duplicate key", "a: 1\na: 2", FormatYAML},
		{"nan", "v: .nan", FormatYAML},
		{"infinity", "v: -.inf", FormatYAML},
		{"alias bomb", bomb, FormatYAML},
		{"deep nesting", strings.Repeat("[", 2000) + strings.Repeat("]", 2000), FormatYAML},
		{"unknown format", `{}`, Format(9)},
	}
	for _, tc := range tests {
		if _, err := Canonical([]byte(tc.in), tc.f); err == nil {
			t.Errorf("%s: Canonical succeeded", tc.name)
		}
	}
}

func TestYAMLValueRejectsOddNodes(t *testing.T) {
	budget := 10
	for _, n := range []*yaml.Node{
		{Kind: yaml.DocumentNode},
		{Kind: yaml.AliasNode},
		{Kind: yaml.MappingNode, Content: []*yaml.Node{{Kind: yaml.ScalarNode, Tag: "!!str", Value: "k"}}},
		{Kind: 0},
		{Kind: yaml.ScalarNode, Tag: "!!bool", Value: "maybe"},
		{Kind: yaml.ScalarNode, Tag: "!!float", Value: "x"},
	} {
		if _, err := yamlValue(n, &budget, 0); !errors.Is(err, ErrMalformedBody) {
			t.Errorf("yamlValue(kind %d) = %v, want ErrMalformedBody", n.Kind, err)
		}
	}
}
