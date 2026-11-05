// SPDX-License-Identifier: GPL-3.0-only

package app

import (
	"crypto/sha256"
	"crypto/subtle"
	"net/http"
	"strings"
)

// RequireBearer returns a handler that passes a request to next only when it
// carries the header "Authorization: Bearer <token>". Any other request
// answers 401 with a Bearer challenge and never reaches next. An empty token
// returns next unchanged, so the endpoint stays open.
//
// The comparison runs in constant time over SHA-256 digests, so neither the
// content nor the length of the token leaks through timing.
func RequireBearer(token string, next http.Handler) http.Handler {
	if token == "" {
		return next
	}
	want := sha256.Sum256([]byte(token))
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		got, ok := bearerToken(r.Header.Get("Authorization"))
		sum := sha256.Sum256([]byte(got))
		if !ok || subtle.ConstantTimeCompare(sum[:], want[:]) != 1 {
			w.Header().Set("WWW-Authenticate", `Bearer realm="fps-controller metrics"`)
			w.Header().Set("Cache-Control", "no-store")
			http.Error(w, "unauthorized", http.StatusUnauthorized)
			return
		}
		next.ServeHTTP(w, r)
	})
}

// bearerToken extracts the credentials of a Bearer authorization header. The
// scheme name is case insensitive.
func bearerToken(header string) (string, bool) {
	const scheme = "bearer "
	if len(header) <= len(scheme) || !strings.EqualFold(header[:len(scheme)], scheme) {
		return "", false
	}
	return strings.TrimSpace(header[len(scheme):]), true
}
