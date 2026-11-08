// SPDX-License-Identifier: GPL-3.0-only

package pki

import (
	"encoding/json"
	"encoding/pem"
	"errors"
	"net/http"
	"time"
)

// maxBody caps a request body. A CSR is about 250 bytes.
const maxBody = 16 << 10

// EnrollRequest is the JSON body of POST /enroll.
type EnrollRequest struct {
	Token  string `json:"token"`
	CSRPEM string `json:"csr_pem"`
}

// RenewRequest is the JSON body of POST /renew.
type RenewRequest struct {
	CSRPEM string `json:"csr_pem"`
}

// IssuedResponse is the JSON answer of both endpoints.
type IssuedResponse struct {
	AgentID        string    `json:"agent_id"`
	CertificatePEM string    `json:"certificate_pem"`
	CAPEM          string    `json:"ca_pem"`
	NotBefore      time.Time `json:"not_before"`
	NotAfter       time.Time `json:"not_after"`
	RenewAt        time.Time `json:"renew_at"`
}

// Handler serves the two endpoints that issue certificates. It belongs behind
// a listener that uses EnrollTLSConfig.
//
//	POST /enroll  {"token": "...", "csr_pem": "..."}   no client certificate
//	POST /renew   {"csr_pem": "..."}                   presents the current certificate
//
// Both answer 200 with an IssuedResponse. A bad token, or a missing, expired,
// foreign, or revoked certificate answers 401 with one fixed message, a CSR
// that the CA cannot use answers 400, and any other failure answers 500. The
// handler never logs or echoes a token.
func (a *Authority) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("POST /enroll", a.serveEnroll)
	mux.HandleFunc("POST /renew", a.serveRenew)
	return mux
}

func (a *Authority) serveEnroll(w http.ResponseWriter, r *http.Request) {
	var req EnrollRequest
	if !decode(w, r, &req) {
		return
	}
	csr, ok := csrDER(req.CSRPEM)
	if !ok {
		fail(w, http.StatusBadRequest, "csr_pem must hold one PEM CERTIFICATE REQUEST")
		return
	}
	issued, err := a.Enroll(req.Token, csr)
	reply(w, issued, err)
}

func (a *Authority) serveRenew(w http.ResponseWriter, r *http.Request) {
	if r.TLS == nil || len(r.TLS.PeerCertificates) == 0 {
		fail(w, http.StatusUnauthorized, "a valid client certificate is required")
		return
	}
	var req RenewRequest
	if !decode(w, r, &req) {
		return
	}
	csr, ok := csrDER(req.CSRPEM)
	if !ok {
		fail(w, http.StatusBadRequest, "csr_pem must hold one PEM CERTIFICATE REQUEST")
		return
	}
	issued, err := a.Renew(r.TLS.PeerCertificates[0], csr)
	reply(w, issued, err)
}

func reply(w http.ResponseWriter, issued *Issued, err error) {
	switch {
	case err == nil:
		w.Header().Set("Content-Type", "application/json")
		w.Header().Set("Cache-Control", "no-store")
		_ = json.NewEncoder(w).Encode(IssuedResponse{
			AgentID:        issued.AgentID,
			CertificatePEM: string(issued.CertPEM),
			CAPEM:          string(issued.CAPEM),
			NotBefore:      issued.NotBefore,
			NotAfter:       issued.NotAfter,
			RenewAt:        issued.RenewAt(),
		})
	case errors.Is(err, ErrInvalidToken), errors.Is(err, ErrTokenUsed), errors.Is(err, ErrTokenExpired),
		errors.Is(err, ErrCertificateInvalid), errors.Is(err, ErrRevoked):
		fail(w, http.StatusUnauthorized, "credentials not accepted")
	case errors.Is(err, ErrInvalidCSR):
		fail(w, http.StatusBadRequest, "the certificate signing request is not acceptable")
	default:
		fail(w, http.StatusInternalServerError, "the certificate could not be issued")
	}
}

func decode(w http.ResponseWriter, r *http.Request, v any) bool {
	r.Body = http.MaxBytesReader(w, r.Body, maxBody)
	dec := json.NewDecoder(r.Body)
	dec.DisallowUnknownFields()
	if err := dec.Decode(v); err != nil {
		fail(w, http.StatusBadRequest, "the body must be one JSON object of the documented form")
		return false
	}
	return true
}

func csrDER(p string) ([]byte, bool) {
	b, rest := pem.Decode([]byte(p))
	if b == nil || b.Type != "CERTIFICATE REQUEST" || len(rest) > 0 {
		return nil, false
	}
	return b.Bytes, true
}

func fail(w http.ResponseWriter, code int, msg string) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	if code == http.StatusUnauthorized {
		w.Header().Set("WWW-Authenticate", `Bearer realm="fps-enroll"`)
	}
	w.WriteHeader(code)
	_ = json.NewEncoder(w).Encode(map[string]string{"error": msg})
}
