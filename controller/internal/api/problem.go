// SPDX-License-Identifier: GPL-3.0-only

package api

import (
	"encoding/json"
	"fmt"
	"net/http"
)

// ProblemContentType is the media type of RFC 9457 problem details in JSON.
const ProblemContentType = "application/problem+json"

// ProblemTypeBlank is the problem type that RFC 9457 defines for problems
// that the HTTP status code describes fully.
const ProblemTypeBlank = "about:blank"

// NewProblem returns a problem of type about:blank for status, whose title is
// the status text, as RFC 9457 section 4.2.1 asks.
func NewProblem(status int, detail string) Problem {
	return Problem{
		Type:   ptr(ProblemTypeBlank),
		Title:  ptr(http.StatusText(status)),
		Status: ptr(status),
		Detail: ptr(detail),
	}
}

// Problemf returns NewProblem with a formatted detail.
func Problemf(status int, format string, args ...any) Problem {
	return NewProblem(status, fmt.Sprintf(format, args...))
}

// WriteProblem writes p as the response. The HTTP status is p.Status, or 500
// when p has none. When p has no instance, the request path becomes the
// instance, so a client can tell which request failed.
func WriteProblem(w http.ResponseWriter, r *http.Request, p Problem) {
	status := http.StatusInternalServerError
	if p.Status != nil {
		status = *p.Status
	} else {
		p.Status = ptr(status)
	}
	if p.Instance == nil && r != nil {
		p.Instance = ptr(r.URL.EscapedPath())
	}
	body, err := json.Marshal(p)
	if err != nil {
		// Problem holds only strings, numbers, and findings, so encoding
		// cannot fail. Fall back to a fixed body to stay within the contract.
		body = []byte(`{"type":"about:blank","status":500}`)
		status = http.StatusInternalServerError
	}
	h := w.Header()
	h.Set("Content-Type", ProblemContentType)
	h.Set("Cache-Control", "no-store")
	h.Set("X-Content-Type-Options", "nosniff")
	w.WriteHeader(status)
	_, _ = w.Write(append(body, '\n'))
}

func ptr[T any](v T) *T { return &v }
