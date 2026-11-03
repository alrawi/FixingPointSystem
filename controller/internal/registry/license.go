// SPDX-License-Identifier: GPL-3.0-only

package registry

import (
	"errors"
	"fmt"
	"strings"
)

// ErrLicenseUnknown reports a license expression that is malformed or names a
// license or exception that the allow list does not know.
var ErrLicenseUnknown = errors.New("unknown license")

// ErrLicenseIncompatible reports a well formed license expression that does not
// allow distribution together with GPL-3.0 code.
var ErrLicenseIncompatible = errors.New("license is not compatible with GPL-3.0")

// compatibleLicenses maps the lower case SPDX identifier of each license that
// may be combined with GPL-3.0 code to its canonical spelling. The list follows
// the GPL-3.0 compatibility notes of the Free Software Foundation.
var compatibleLicenses = canonical(
	"GPL-3.0-only", "GPL-3.0-or-later", "GPL-2.0-or-later",
	"LGPL-3.0-only", "LGPL-3.0-or-later", "LGPL-2.1-only", "LGPL-2.1-or-later",
	"AGPL-3.0-only", "AGPL-3.0-or-later",
	"Apache-2.0", "MIT", "MIT-0", "X11", "ISC", "0BSD", "Zlib", "BSL-1.0",
	"BSD-2-Clause", "BSD-3-Clause", "MPL-2.0", "Artistic-2.0",
	"Unlicense", "CC0-1.0", "CC-BY-4.0", "CC-BY-SA-4.0",
	"Unicode-3.0", "Unicode-DFS-2016", "CDLA-Permissive-2.0",
)

// incompatibleLicenses names well known licenses that cannot be combined with
// GPL-3.0 code, so that the error says incompatible rather than unknown.
var incompatibleLicenses = canonical(
	"GPL-2.0-only", "LGPL-2.0-only", "Apache-1.0", "Apache-1.1", "BSD-4-Clause",
	"CDDL-1.0", "CDDL-1.1", "EPL-1.0", "EPL-2.0", "MPL-1.0", "MPL-1.1",
	"MPL-2.0-no-copyleft-exception", "SSPL-1.0", "BUSL-1.1", "JSON", "OpenSSL",
	"CC-BY-NC-4.0", "CC-BY-ND-4.0", "CC-BY-NC-SA-4.0", "CC-BY-NC-ND-4.0",
	"Commons-Clause", "Elastic-2.0",
)

// licenseExceptions names the SPDX exceptions that only add permissions, so a
// compatible license with one of them stays compatible.
var licenseExceptions = canonical(
	"LLVM-exception", "Classpath-exception-2.0", "GCC-exception-3.1",
)

func canonical(ids ...string) map[string]string {
	m := make(map[string]string, len(ids))
	for _, id := range ids {
		m[strings.ToLower(id)] = id
	}
	return m
}

// CheckLicense parses expr as an SPDX license expression and reports whether
// module content under it may be distributed with GPL-3.0 code. An expression
// is compatible when every AND operand is compatible and at least one operand
// of each OR is compatible. Every license and exception in the expression must
// be known, even in an OR operand that is not needed, so a typo never passes.
// The deprecated "+" suffix and the bare GPL identifiers are unknown, because
// SPDX replaced them with the -only and -or-later forms.
//
// The error wraps ErrLicenseUnknown or ErrLicenseIncompatible.
func CheckLicense(expr string) error {
	p := &licenseParser{toks: tokenizeLicense(expr)}
	if len(p.toks) == 0 {
		return fmt.Errorf("license %q: empty expression: %w", expr, ErrLicenseUnknown)
	}
	n, err := p.or()
	if err == nil && p.pos < len(p.toks) {
		err = fmt.Errorf("unexpected %q", p.toks[p.pos])
	}
	if err != nil {
		return fmt.Errorf("license %q: %s: %w", expr, err.Error(), ErrLicenseUnknown)
	}
	if bad := n.incompatible(); bad != "" {
		return fmt.Errorf("license %q: %s: %w", expr, bad, ErrLicenseIncompatible)
	}
	return nil
}

// licenseNode is one node of a parsed expression. A leaf has an id, and an
// inner node has an operator and operands.
type licenseNode struct {
	op   string // "AND", "OR", or empty for a leaf
	args []licenseNode
	id   string // canonical license id of a leaf
	ok   bool   // whether the leaf license is compatible
}

// incompatible returns the license that makes n incompatible, or an empty
// string when n is compatible.
func (n licenseNode) incompatible() string {
	switch n.op {
	case "AND":
		for _, a := range n.args {
			if bad := a.incompatible(); bad != "" {
				return bad
			}
		}
		return ""
	case "OR":
		var first string
		for _, a := range n.args {
			bad := a.incompatible()
			if bad == "" {
				return ""
			}
			if first == "" {
				first = bad
			}
		}
		return first
	default:
		if n.ok {
			return ""
		}
		return n.id
	}
}

func tokenizeLicense(expr string) []string {
	var toks []string
	var cur strings.Builder
	flush := func() {
		if cur.Len() > 0 {
			toks = append(toks, cur.String())
			cur.Reset()
		}
	}
	for _, r := range expr {
		switch r {
		case '(', ')':
			flush()
			toks = append(toks, string(r))
		case ' ', '\t', '\n', '\r':
			flush()
		default:
			cur.WriteRune(r)
		}
	}
	flush()
	return toks
}

type licenseParser struct {
	toks []string
	pos  int
}

func (p *licenseParser) peek() string {
	if p.pos < len(p.toks) {
		return p.toks[p.pos]
	}
	return ""
}

func (p *licenseParser) or() (licenseNode, error) {
	return p.binary("OR", p.and)
}

func (p *licenseParser) and() (licenseNode, error) {
	return p.binary("AND", p.term)
}

func (p *licenseParser) binary(op string, next func() (licenseNode, error)) (licenseNode, error) {
	first, err := next()
	if err != nil {
		return licenseNode{}, err
	}
	n := licenseNode{op: op, args: []licenseNode{first}}
	for p.peek() == op {
		p.pos++
		a, err := next()
		if err != nil {
			return licenseNode{}, err
		}
		n.args = append(n.args, a)
	}
	if len(n.args) == 1 {
		return first, nil
	}
	return n, nil
}

// term parses a parenthesized expression or a license with an optional WITH
// exception.
func (p *licenseParser) term() (licenseNode, error) {
	tok := p.peek()
	switch tok {
	case "":
		return licenseNode{}, errors.New("expression ends early")
	case "(":
		p.pos++
		n, err := p.or()
		if err != nil {
			return licenseNode{}, err
		}
		if p.peek() != ")" {
			return licenseNode{}, errors.New("missing closing parenthesis")
		}
		p.pos++
		return n, nil
	case ")", "AND", "OR", "WITH":
		return licenseNode{}, fmt.Errorf("unexpected %q", tok)
	}
	p.pos++
	key := strings.ToLower(tok)
	var n licenseNode
	if id, ok := compatibleLicenses[key]; ok {
		n = licenseNode{id: id, ok: true}
	} else if id, ok := incompatibleLicenses[key]; ok {
		n = licenseNode{id: id}
	} else {
		return licenseNode{}, fmt.Errorf("license %q is not on the allow list", tok)
	}
	if p.peek() == "WITH" {
		p.pos++
		exc := p.peek()
		id, ok := licenseExceptions[strings.ToLower(exc)]
		if !ok {
			return licenseNode{}, fmt.Errorf("license exception %q is not on the allow list", exc)
		}
		p.pos++
		n.id += " WITH " + id
	}
	return n, nil
}
