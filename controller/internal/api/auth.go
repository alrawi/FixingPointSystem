// SPDX-License-Identifier: GPL-3.0-only

package api

import (
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"net/http"
	"strings"
)

// Scope is the permission level of an API token. The levels nest, so a run
// token also passes every read operation and an admin token passes everything.
type Scope int

// The scopes that the contract names in x-required-scope, from lowest to highest.
const (
	ScopeRead Scope = iota + 1
	ScopeRun
	ScopeAdmin
)

// String returns the name of s as the contract writes it.
func (s Scope) String() string {
	switch s {
	case ScopeRead:
		return "read"
	case ScopeRun:
		return "run"
	case ScopeAdmin:
		return "admin"
	default:
		return fmt.Sprintf("Scope(%d)", int(s))
	}
}

// Allows reports whether a token with scope s may call an operation that
// requires scope required.
func (s Scope) Allows(required Scope) bool {
	return s.valid() && required.valid() && s >= required
}

func (s Scope) valid() bool { return s >= ScopeRead && s <= ScopeAdmin }

// ParseScope returns the scope named read, run, or admin.
func ParseScope(name string) (Scope, error) {
	for _, s := range []Scope{ScopeRead, ScopeRun, ScopeAdmin} {
		if s.String() == name {
			return s, nil
		}
	}
	return 0, fmt.Errorf("unknown scope %q, want read, run, or admin", name)
}

// Authenticator maps a bearer token to its scope. It returns false for an
// unknown token.
type Authenticator interface {
	Authenticate(token string) (Scope, bool)
}

// StaticTokens is an Authenticator over a fixed set of tokens. It keeps only
// SHA-256 digests of the tokens, so a memory dump does not reveal them, and a
// lookup by digest takes the same time whichever byte of a token differs.
type StaticTokens struct {
	scopes map[[sha256.Size]byte]Scope
}

// NewStaticTokens returns an Authenticator for tokens, which maps each token to
// its scope. It rejects an empty token and an invalid scope.
func NewStaticTokens(tokens map[string]Scope) (*StaticTokens, error) {
	st := &StaticTokens{scopes: make(map[[sha256.Size]byte]Scope, len(tokens))}
	for tok, scope := range tokens {
		if tok == "" {
			return nil, errors.New("empty API token")
		}
		if !scope.valid() {
			return nil, fmt.Errorf("API token has invalid scope %v", scope)
		}
		st.scopes[sha256.Sum256([]byte(tok))] = scope
	}
	return st, nil
}

// Authenticate implements Authenticator.
func (st *StaticTokens) Authenticate(token string) (Scope, bool) {
	if st == nil || token == "" {
		return 0, false
	}
	s, ok := st.scopes[sha256.Sum256([]byte(token))]
	return s, ok
}

// Realm is the realm that WWW-Authenticate challenges name.
const Realm = "fps"

type scopeKey struct{}

// ScopeFromContext returns the scope of the token that authenticated the
// request whose context is ctx.
func ScopeFromContext(ctx context.Context) (Scope, bool) {
	s, ok := ctx.Value(scopeKey{}).(Scope)
	return s, ok
}

// bearerToken returns the token of an Authorization header that uses the
// Bearer scheme, which RFC 7235 matches without regard to case.
func bearerToken(r *http.Request) (string, bool) {
	scheme, token, ok := strings.Cut(r.Header.Get("Authorization"), " ")
	if !ok || !strings.EqualFold(scheme, "Bearer") {
		return "", false
	}
	token = strings.TrimSpace(token)
	return token, token != ""
}

// requireScope wraps next so that it runs only for a token whose scope allows
// required. It answers 401 without a valid token and 403 for a token whose
// scope is too low, with the challenges of RFC 6750 section 3.
func requireScope(auth Authenticator, required Scope, next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		token, ok := bearerToken(r)
		if !ok {
			w.Header().Set("WWW-Authenticate", fmt.Sprintf("Bearer realm=%q", Realm))
			WriteProblem(w, r, NewProblem(http.StatusUnauthorized, "this operation needs a bearer token"))
			return
		}
		scope, ok := auth.Authenticate(token)
		if !ok {
			w.Header().Set("WWW-Authenticate", fmt.Sprintf("Bearer realm=%q, error=\"invalid_token\"", Realm))
			WriteProblem(w, r, NewProblem(http.StatusUnauthorized, "the bearer token is not valid"))
			return
		}
		if !scope.Allows(required) {
			w.Header().Set("WWW-Authenticate", fmt.Sprintf("Bearer realm=%q, error=\"insufficient_scope\", scope=%q", Realm, required.String()))
			WriteProblem(w, r, Problemf(http.StatusForbidden, "this operation needs scope %s, and the token has scope %s", required, scope))
			return
		}
		next.ServeHTTP(w, r.WithContext(context.WithValue(r.Context(), scopeKey{}, scope)))
	})
}
