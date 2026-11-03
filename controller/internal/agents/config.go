// SPDX-License-Identifier: GPL-3.0-only

package agents

import (
	"context"
	"crypto/x509"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"regexp"
	"strings"
	"time"

	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/peer"
)

// Default session parameters that Register returns.
const (
	DefaultHeartbeatInterval = time.Second
	DefaultHeartbeatTimeout  = 5 * time.Second
)

// DefaultEngineAPIs lists the engine contract versions, as major.minor, that
// the controller drives when Config.EngineAPIs is empty.
var DefaultEngineAPIs = []string{"0.1", "0.2"}

// Clock tells the time. The manager reads time only through it.
type Clock interface {
	Now() time.Time
}

// SystemClock reads the wall clock.
type SystemClock struct{}

// Now returns the wall clock time.
func (SystemClock) Now() time.Time { return time.Now() }

// IdentityFunc returns the identity of the agent behind a call. claimed is
// the agent_id of the request, which the function may use only when the
// connection carries no identity of its own.
type IdentityFunc func(ctx context.Context, claimed string) (string, error)

// ErrNoIdentity reports that a connection carries no verified client
// certificate.
var ErrNoIdentity = errors.New("no verified client certificate")

// PeerCertIdentity returns the identity in the verified client certificate of
// the connection. It takes the subject common name, or the first DNS name
// when the common name is empty. The gRPC server must verify client
// certificates, for example with tls.RequireAndVerifyClientCert.
func PeerCertIdentity(ctx context.Context, _ string) (string, error) {
	p, ok := peer.FromContext(ctx)
	if !ok {
		return "", ErrNoIdentity
	}
	info, ok := p.AuthInfo.(credentials.TLSInfo)
	if !ok || len(info.State.VerifiedChains) == 0 || len(info.State.VerifiedChains[0]) == 0 {
		return "", ErrNoIdentity
	}
	if id := certIdentity(info.State.VerifiedChains[0][0]); id != "" {
		return id, nil
	}
	return "", fmt.Errorf("client certificate names no identity: %w", ErrNoIdentity)
}

func certIdentity(c *x509.Certificate) string {
	if c.Subject.CommonName != "" {
		return c.Subject.CommonName
	}
	if len(c.DNSNames) > 0 {
		return c.DNSNames[0]
	}
	return ""
}

// ClaimedIdentity trusts the agent_id of the request. It serves tests and
// local development without TLS and must not run in production.
func ClaimedIdentity(_ context.Context, claimed string) (string, error) {
	return claimed, nil
}

// Config configures a Manager. Zero fields take the defaults that New names.
type Config struct {
	// ControllerID names the controller instance in RegisterResponse.
	ControllerID string
	// HeartbeatInterval is the period at which agents send heartbeats.
	HeartbeatInterval time.Duration
	// HeartbeatTimeout is the silence after which an agent is offline. It
	// must exceed HeartbeatInterval.
	HeartbeatTimeout time.Duration
	// EngineAPIs lists the engine contract versions, as major.minor, that the
	// controller drives.
	EngineAPIs []string
	// AutoEnroll accepts agent identities that no one enrolled ahead of time.
	AutoEnroll bool
	// Identity returns the identity of the agent behind a call. The default
	// is PeerCertIdentity.
	Identity IdentityFunc
	// Clock tells the time. The default is SystemClock.
	Clock Clock
	// Logger receives structured logs. The default discards them.
	Logger *slog.Logger
	// Observer receives every Event. It runs on the goroutine that caused
	// the event, outside the manager lock, and must return quickly.
	Observer func(Event)
	// Rand supplies session tokens. The default is crypto/rand.Reader.
	Rand io.Reader
}

// idPattern is the Id parameter pattern of the REST contract.
var idPattern = regexp.MustCompile(`^[A-Za-z0-9_-]{1,64}$`)

// ValidID reports whether id is a valid agent identity for the REST contract.
func ValidID(id string) bool { return idPattern.MatchString(id) }

// engineMinor returns the major.minor part of a semantic version.
func engineMinor(v string) string {
	parts := strings.SplitN(v, ".", 3)
	if len(parts) < 2 || parts[0] == "" || parts[1] == "" {
		return ""
	}
	return parts[0] + "." + parts[1]
}
