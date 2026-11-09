// SPDX-License-Identifier: GPL-3.0-only

package wiring

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"log/slog"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"time"

	"github.com/alrawi/FixingPointSystem/controller/internal/agents"
)

// ErrNoPKI reports an operation that needs the certificate authority on a
// controller that runs with insecure_plain_grpc.
var ErrNoPKI = errors.New("the controller runs without a certificate authority (insecure_plain_grpc)")

// MintToken creates a one time enrollment token for the agent identity id that
// stays valid for ttl, or for the default of the authority when ttl is zero.
// It also adds the identity to the inventory when it is not there, because an
// operator who mints a token admits the agent. The token is a secret that the
// caller hands to the agent out of band, and it is not stored in clear.
func (c *Controller) MintToken(id string, ttl time.Duration) (token string, expires time.Time, err error) {
	if c.PKI == nil {
		return "", time.Time{}, ErrNoPKI
	}
	if token, expires, err = c.PKI.NewToken(id, ttl); err != nil {
		return "", time.Time{}, err
	}
	if _, err := c.Agents.Get(id); errors.Is(err, agents.ErrUnknownAgent) {
		if _, err := c.Agents.Enroll(id, nil); err != nil {
			return "", time.Time{}, err
		}
	}
	c.logger.Info("enrollment token minted", "agent_id", id, "expires", expires.UTC().Format(time.RFC3339))
	return token, expires, nil
}

// RevokeIdentity revokes every certificate and every unused token of the agent
// identity id and ends its open session. It returns the number of
// certificates that it revoked. The inventory entry stays, so an operator can
// delete it with the REST API.
func (c *Controller) RevokeIdentity(id string) (int, error) {
	if c.PKI == nil {
		return 0, ErrNoPKI
	}
	n, err := c.PKI.Revoke(id)
	if err != nil {
		return n, err
	}
	c.logger.Warn("agent identity revoked", "agent_id", id, "certificates", n)
	return n, nil
}

// ListenEnroll opens the TCP listener of the enrollment service. It returns
// nil without an error when the controller runs without a certificate
// authority, because there is nothing to serve.
func (c *Controller) ListenEnroll(ctx context.Context) (net.Listener, error) {
	if c.PKI == nil {
		return nil, nil
	}
	var lc net.ListenConfig
	ln, err := lc.Listen(ctx, "tcp", c.cfg.EnrollListen)
	if err != nil {
		return nil, fmt.Errorf("listen on %s: %w", c.cfg.EnrollListen, err)
	}
	return ln, nil
}

// ListenAdmin opens the admin socket. It returns nil without an error when no
// socket is configured or when the controller runs without a certificate
// authority. It refuses a socket that another controller serves, so two
// controllers never write one PKI directory, and it replaces the stale socket
// file of a controller that died. The socket accepts only its owner.
func (c *Controller) ListenAdmin(ctx context.Context) (net.Listener, error) {
	path := c.cfg.ResolvedAdminSocket()
	if path == "" || c.PKI == nil {
		return nil, nil
	}
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return nil, fmt.Errorf("admin socket: %w", err)
	}
	if info, err := os.Stat(dir); err != nil {
		return nil, fmt.Errorf("admin socket: %w", err)
	} else if info.Mode().Perm()&0o002 != 0 {
		return nil, fmt.Errorf("admin socket: directory %s is writable by everyone; use a private directory", dir)
	}
	if err := clearStaleSocket(ctx, path); err != nil {
		return nil, err
	}
	var lc net.ListenConfig
	ln, err := lc.Listen(ctx, "unix", path)
	if err != nil {
		return nil, fmt.Errorf("admin socket: %w", err)
	}
	if err := os.Chmod(path, 0o600); err != nil {
		_ = ln.Close()
		return nil, fmt.Errorf("admin socket: %w", err)
	}
	return ln, nil
}

// clearStaleSocket removes the socket file at path when no process answers on
// it, and fails when a process does or when the file is not a socket.
func clearStaleSocket(ctx context.Context, path string) error {
	info, err := os.Lstat(path)
	if errors.Is(err, fs.ErrNotExist) {
		return nil
	}
	if err != nil {
		return fmt.Errorf("admin socket: %w", err)
	}
	if info.Mode().Type() != fs.ModeSocket {
		return fmt.Errorf("admin socket: %s exists and is not a socket", path)
	}
	dctx, cancel := context.WithTimeout(ctx, time.Second)
	defer cancel()
	var d net.Dialer
	if conn, err := d.DialContext(dctx, "unix", path); err == nil {
		_ = conn.Close()
		return fmt.Errorf("admin socket: another controller already serves %s; two controllers must not share one PKI directory", path)
	}
	if err := os.Remove(path); err != nil {
		return fmt.Errorf("admin socket: remove stale socket: %w", err)
	}
	return nil
}

// RunPKI serves the enrollment service on enroll and the admin API on admin
// until ctx ends, and then shuts both down within the shutdown timeout. Either
// listener may be nil. It closes both listeners in every case.
func (c *Controller) RunPKI(ctx context.Context, enroll, admin net.Listener) error {
	type service struct {
		name string
		srv  *http.Server
		ln   net.Listener
	}
	var services []service
	if enroll != nil {
		services = append(services, service{"enrollment", &http.Server{
			Handler:           c.logEnroll(c.PKI.Handler()),
			ReadHeaderTimeout: time.Duration(c.cfg.ReadHeaderTimeout),
			ErrorLog:          slog.NewLogLogger(c.logger.Handler(), slog.LevelWarn),
		}, tls.NewListener(enroll, c.enrollTLS)})
		c.logger.Info("enrollment service listening", "addr", enroll.Addr().String(), "transport", "tls")
	}
	if admin != nil {
		services = append(services, service{"admin", &http.Server{
			Handler:           c.AdminHandler(),
			ReadHeaderTimeout: time.Duration(c.cfg.ReadHeaderTimeout),
			ErrorLog:          slog.NewLogLogger(c.logger.Handler(), slog.LevelWarn),
		}, admin})
		c.logger.Info("admin socket listening", "addr", admin.Addr().String())
	}
	if len(services) == 0 {
		return nil
	}
	errc := make(chan error, len(services))
	for _, s := range services {
		go func() {
			if err := s.srv.Serve(s.ln); err != nil && !errors.Is(err, http.ErrServerClosed) {
				errc <- fmt.Errorf("serve %s service: %w", s.name, err)
				return
			}
			errc <- nil
		}()
	}
	var err error
	select {
	case err = <-errc:
	case <-ctx.Done():
	}
	sctx, cancel := context.WithTimeout(context.Background(), time.Duration(c.cfg.ShutdownTimeout))
	defer cancel()
	for _, s := range services {
		if serr := s.srv.Shutdown(sctx); serr != nil {
			_ = s.srv.Close()
		}
		_ = s.ln.Close()
	}
	return err
}

// logEnroll logs the outcome of every enrollment and renewal request. It never
// logs the body, which holds the token.
func (c *Controller) logEnroll(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		rec := &statusRecorder{ResponseWriter: w, status: http.StatusOK}
		next.ServeHTTP(rec, r)
		level := c.logger.Info
		if rec.status >= http.StatusBadRequest {
			level = c.logger.Warn
		}
		level("enrollment request", "path", r.URL.Path, "status", rec.status, "remote", r.RemoteAddr)
	})
}

type statusRecorder struct {
	http.ResponseWriter
	status int
}

func (r *statusRecorder) WriteHeader(code int) {
	r.status = code
	r.ResponseWriter.WriteHeader(code)
}

// AdminPaths are the operations of the admin socket.
const (
	// AdminTokenPath mints an enrollment token.
	AdminTokenPath = "/admin/tokens"
	// AdminRevokePath revokes an agent identity.
	AdminRevokePath = "/admin/revoke"
)

// TokenRequest is the body of POST /admin/tokens.
type TokenRequest struct {
	// AgentID is the identity that the token enrolls.
	AgentID string `json:"agent_id"`
	// TTL is a Go duration such as "1h". Empty takes the default of one hour.
	TTL string `json:"ttl,omitempty"`
}

// TokenResponse is the body of a successful POST /admin/tokens.
type TokenResponse struct {
	// AgentID echoes the identity.
	AgentID string `json:"agent_id"`
	// Token is the secret. It appears once, and the controller keeps only its digest.
	Token string `json:"token"`
	// Expires is the end of the token's validity.
	Expires time.Time `json:"expires"`
}

// RevokeRequest is the body of POST /admin/revoke.
type RevokeRequest struct {
	// AgentID is the identity to revoke.
	AgentID string `json:"agent_id"`
}

// RevokeResponse is the body of a successful POST /admin/revoke.
type RevokeResponse struct {
	// AgentID echoes the identity.
	AgentID string `json:"agent_id"`
	// Certificates counts the revoked certificates.
	Certificates int `json:"certificates"`
}

// AdminHandler returns the handler of the admin socket. It has no
// authentication of its own: the unix socket file, which only its owner can
// open, is the credential. Never serve it on a TCP listener. It is the only
// way besides the enrollment service to change the PKI state, and it runs in
// the controller process, so the process stays the single writer.
func (c *Controller) AdminHandler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("POST "+AdminTokenPath, func(w http.ResponseWriter, r *http.Request) {
		var req TokenRequest
		if !decodeAdmin(w, r, &req) {
			return
		}
		var ttl time.Duration
		if req.TTL != "" {
			d, err := time.ParseDuration(req.TTL)
			if err != nil || d <= 0 {
				adminError(w, http.StatusBadRequest, fmt.Sprintf("ttl %q is not a positive duration such as 1h", req.TTL))
				return
			}
			ttl = d
		}
		token, expires, err := c.MintToken(req.AgentID, ttl)
		if err != nil {
			adminError(w, adminStatus(err), err.Error())
			return
		}
		adminReply(w, TokenResponse{AgentID: req.AgentID, Token: token, Expires: expires})
	})
	mux.HandleFunc("POST "+AdminRevokePath, func(w http.ResponseWriter, r *http.Request) {
		var req RevokeRequest
		if !decodeAdmin(w, r, &req) {
			return
		}
		if !agents.ValidID(req.AgentID) {
			adminError(w, http.StatusBadRequest, fmt.Sprintf("agent id %q is not valid", req.AgentID))
			return
		}
		n, err := c.RevokeIdentity(req.AgentID)
		if err != nil {
			adminError(w, adminStatus(err), err.Error())
			return
		}
		adminReply(w, RevokeResponse{AgentID: req.AgentID, Certificates: n})
	})
	return mux
}

func adminStatus(err error) int {
	if errors.Is(err, ErrNoPKI) {
		return http.StatusConflict
	}
	return http.StatusBadRequest
}

func decodeAdmin(w http.ResponseWriter, r *http.Request, v any) bool {
	r.Body = http.MaxBytesReader(w, r.Body, 4<<10)
	dec := json.NewDecoder(r.Body)
	dec.DisallowUnknownFields()
	if err := dec.Decode(v); err != nil {
		adminError(w, http.StatusBadRequest, "the body must be one JSON object of the documented form")
		return false
	}
	return true
}

func adminReply(w http.ResponseWriter, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	_ = json.NewEncoder(w).Encode(v)
}

func adminError(w http.ResponseWriter, code int, msg string) {
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(code)
	_ = json.NewEncoder(w).Encode(map[string]string{"error": msg})
}
