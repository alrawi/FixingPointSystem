// SPDX-License-Identifier: GPL-3.0-only

// Package pki is the public key infrastructure of the controller. It gives the
// control channel between the controller and its agents mutual TLS.
//
// An Authority owns a controller CA that Open generates on first start. The
// operator mints a single use enrollment token for an agent identity with
// NewToken. The agent generates its own key, sends a certificate signing
// request with the token, and receives a short lived client certificate whose
// subject common name is the agent identity. Before the certificate expires
// the agent renews it over a connection that presents the current certificate.
// Revoke ends an identity at once, and ServerTLSConfig builds the tls.Config
// for the gRPC server that requires and verifies client certificates.
//
// The package uses only the standard library, crypto/x509, crypto/tls, and
// crypto/ecdsa, and it imports no other package of the controller. The gRPC
// server wiring and the agent are outside its scope; docs/wiki/Controller.md
// describes what they must do.
package pki
