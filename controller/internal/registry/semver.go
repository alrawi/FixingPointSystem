// SPDX-License-Identifier: GPL-3.0-only

package registry

import (
	"strconv"
	"strings"
)

// compareVersions orders two semantic versions by the precedence rules of
// semver 2.0.0. Build metadata does not affect precedence, so ties break on the
// plain string to keep the order total. Versions that the manifest schema
// accepted always parse.
func compareVersions(a, b string) int {
	if c := comparePrecedence(a, b); c != 0 {
		return c
	}
	return strings.Compare(a, b)
}

func comparePrecedence(a, b string) int {
	a, _, _ = strings.Cut(a, "+")
	b, _, _ = strings.Cut(b, "+")
	ac, apre, _ := strings.Cut(a, "-")
	bc, bpre, _ := strings.Cut(b, "-")
	as, bs := strings.Split(ac, "."), strings.Split(bc, ".")
	for i := 0; i < len(as) && i < len(bs); i++ {
		if c := compareNumeric(as[i], bs[i]); c != 0 {
			return c
		}
	}
	switch {
	case apre == "" && bpre == "":
		return 0
	case apre == "":
		return 1
	case bpre == "":
		return -1
	}
	ap, bp := strings.Split(apre, "."), strings.Split(bpre, ".")
	for i := 0; i < len(ap) && i < len(bp); i++ {
		an, aerr := strconv.ParseUint(ap[i], 10, 64)
		bn, berr := strconv.ParseUint(bp[i], 10, 64)
		var c int
		switch {
		case aerr == nil && berr == nil:
			c = cmpUint(an, bn)
		case aerr == nil:
			c = -1
		case berr == nil:
			c = 1
		default:
			c = strings.Compare(ap[i], bp[i])
		}
		if c != 0 {
			return c
		}
	}
	return len(ap) - len(bp)
}

// compareNumeric compares two decimal strings without leading zeros of any
// length, so versions above the range of uint64 still order correctly.
func compareNumeric(a, b string) int {
	if len(a) != len(b) {
		if len(a) < len(b) {
			return -1
		}
		return 1
	}
	return strings.Compare(a, b)
}

func cmpUint(a, b uint64) int {
	switch {
	case a < b:
		return -1
	case a > b:
		return 1
	}
	return 0
}
