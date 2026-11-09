// SPDX-License-Identifier: GPL-3.0-only

package wiring

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"time"
)

// AdminClient calls the admin socket of a running controller. The token and
// revoke subcommands of fps-controller use it, so the PKI state never has a
// second writer.
type AdminClient struct {
	// Socket is the path of the admin socket.
	Socket string
}

// ErrControllerNotRunning reports that nothing answers on the admin socket.
var ErrControllerNotRunning = errors.New("no controller answers on the admin socket")

// MintToken asks the controller for an enrollment token for agentID. A zero
// ttl takes the default of the controller.
func (c AdminClient) MintToken(ctx context.Context, agentID string, ttl time.Duration) (TokenResponse, error) {
	req := TokenRequest{AgentID: agentID}
	if ttl > 0 {
		req.TTL = ttl.String()
	}
	var out TokenResponse
	err := c.post(ctx, AdminTokenPath, req, &out)
	return out, err
}

// Revoke asks the controller to revoke the identity agentID.
func (c AdminClient) Revoke(ctx context.Context, agentID string) (RevokeResponse, error) {
	var out RevokeResponse
	err := c.post(ctx, AdminRevokePath, RevokeRequest{AgentID: agentID}, &out)
	return out, err
}

func (c AdminClient) post(ctx context.Context, path string, body, out any) error {
	data, err := json.Marshal(body)
	if err != nil {
		return err
	}
	hc := &http.Client{
		Timeout: 30 * time.Second,
		Transport: &http.Transport{
			DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
				var d net.Dialer
				return d.DialContext(ctx, "unix", c.Socket)
			},
		},
	}
	defer hc.CloseIdleConnections()
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, "http://admin"+path, bytes.NewReader(data))
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := hc.Do(req)
	if err != nil {
		return fmt.Errorf("%w at %s: %w", ErrControllerNotRunning, c.Socket, err)
	}
	defer func() { _ = resp.Body.Close() }()
	reply, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if err != nil {
		return err
	}
	if resp.StatusCode != http.StatusOK {
		var e struct {
			Error string `json:"error"`
		}
		if json.Unmarshal(reply, &e) == nil && e.Error != "" {
			return fmt.Errorf("controller refused the request: %s", e.Error)
		}
		return fmt.Errorf("controller answered %s", resp.Status)
	}
	if err := json.Unmarshal(reply, out); err != nil {
		return fmt.Errorf("decode the controller reply: %w", err)
	}
	return nil
}
