// SPDX-License-Identifier: GPL-3.0-only

package api

import (
	"log/slog"
	"net/http"
	"regexp"
	"slices"
	"strings"
)

// BasePath is the path prefix of the API, from the server URL of the contract.
const BasePath = "/v1"

// operation is one row of the generated operation table.
type operation struct {
	method  string
	path    string
	id      string
	scope   Scope
	handler http.HandlerFunc
}

// Mux registers handlers for net/http patterns. *http.ServeMux and the
// controller's *app.App both satisfy it.
type Mux interface {
	Handle(pattern string, h http.Handler)
}

// Server is the REST layer. It routes each operation of the contract to a
// ServerInterface after the token check.
type Server struct {
	ops    []operation
	auth   Authenticator
	logger *slog.Logger
}

// New returns a Server that calls impl for every operation and checks tokens
// with auth. Pass Unimplemented, or a type that embeds it, as impl.
func New(impl ServerInterface, auth Authenticator, logger *slog.Logger) *Server {
	s := &Server{auth: auth, logger: logger}
	w := &ServerInterfaceWrapper{Handler: impl, ErrorHandlerFunc: s.paramError}
	s.ops = operations(w)
	return s
}

// Handler returns a handler that serves only the API, for tests and for
// callers that do not share a mux.
func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	s.Mount(mux)
	return mux
}

// Mount registers every operation on m under BasePath, plus a catch all that
// answers unknown paths with 404 and wrong methods with 405 problems.
//
// The contract names actions with a colon after the last path parameter, as
// in POST /configs/{id}:validate. A net/http wildcard must fill a whole
// segment, so Mount registers POST /v1/configs/{id} and picks the operation by
// the suffix of the segment, then strips the suffix from the path value. A
// colon thus always marks an action, and a plain parameter never matches a
// segment that contains one.
func (s *Server) Mount(m Mux) {
	type candidate struct {
		params []string // plain parameters
		action string   // name of the action parameter, or empty
		suffix string   // colon and action name
		h      http.Handler
	}
	groups := make(map[string][]candidate)
	var patterns []string
	for _, op := range s.ops {
		segs := strings.Split(op.path, "/")
		var c candidate
		for i, seg := range segs {
			if mm := actionRE.FindStringSubmatch(seg); mm != nil && i == len(segs)-1 {
				segs[i] = "{" + mm[1] + "}"
				c.action, c.suffix = mm[1], ":"+mm[2]
			} else if strings.HasPrefix(seg, "{") && strings.HasSuffix(seg, "}") {
				c.params = append(c.params, seg[1:len(seg)-1])
			}
		}
		c.h = requireScope(s.auth, op.scope, op.handler)
		pattern := op.method + " " + BasePath + strings.Join(segs, "/")
		if _, seen := groups[pattern]; !seen {
			patterns = append(patterns, pattern)
		}
		groups[pattern] = append(groups[pattern], c)
	}
	for _, pattern := range patterns {
		candidates := groups[pattern]
		m.Handle(pattern, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		next:
			for _, c := range candidates {
				for _, p := range c.params {
					if strings.Contains(r.PathValue(p), ":") {
						continue next
					}
				}
				if c.action != "" {
					base, ok := strings.CutSuffix(r.PathValue(c.action), c.suffix)
					if !ok || base == "" || strings.Contains(base, ":") {
						continue
					}
					r.SetPathValue(c.action, base)
				}
				c.h.ServeHTTP(w, r)
				return
			}
			s.unrouted(w, r)
		}))
	}
	m.Handle(BasePath+"/", http.HandlerFunc(s.unrouted))
}

// actionRE matches a last path segment such as {id}:validate.
var actionRE = regexp.MustCompile(`^\{([A-Za-z0-9_]+)\}:([A-Za-z0-9_]+)$`)

// unrouted answers a request that no operation matched. When the path matches
// an operation under another method it answers 405 with an Allow header, and
// otherwise 404.
func (s *Server) unrouted(w http.ResponseWriter, r *http.Request) {
	path, ok := strings.CutPrefix(r.URL.Path, BasePath)
	var allow []string
	if ok {
		for _, op := range s.ops {
			if matchTemplate(op.path, path) && !slices.Contains(allow, op.method) {
				allow = append(allow, op.method)
			}
		}
	}
	if len(allow) == 0 {
		WriteProblem(w, r, Problemf(http.StatusNotFound, "no operation matches %s %s", r.Method, r.URL.Path))
		return
	}
	slices.Sort(allow)
	w.Header().Set("Allow", strings.Join(allow, ", "))
	WriteProblem(w, r, Problemf(http.StatusMethodNotAllowed, "%s %s is not allowed, use %s", r.Method, r.URL.Path, strings.Join(allow, " or ")))
}

// matchTemplate reports whether path matches the contract path template tmpl.
// A parameter matches one non empty segment without a colon, and a parameter
// with an action suffix matches a segment that ends in that suffix.
func matchTemplate(tmpl, path string) bool {
	ts, ps := strings.Split(tmpl, "/"), strings.Split(path, "/")
	if len(ts) != len(ps) {
		return false
	}
	for i, t := range ts {
		p := ps[i]
		switch mm := actionRE.FindStringSubmatch(t); {
		case mm != nil:
			base, ok := strings.CutSuffix(p, ":"+mm[2])
			if !ok || base == "" || strings.Contains(base, ":") {
				return false
			}
		case strings.HasPrefix(t, "{") && strings.HasSuffix(t, "}"):
			if p == "" || strings.Contains(p, ":") {
				return false
			}
		case t != p:
			return false
		}
	}
	return true
}

// paramError answers a request whose parameters the generated wrapper could
// not bind, such as a missing required query parameter.
func (s *Server) paramError(w http.ResponseWriter, r *http.Request, err error) {
	if s.logger != nil {
		s.logger.Debug("bad request parameter", "method", r.Method, "path", r.URL.Path, "error", err)
	}
	WriteProblem(w, r, NewProblem(http.StatusBadRequest, err.Error()))
}
