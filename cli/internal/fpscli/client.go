// SPDX-License-Identifier: GPL-3.0-only

package fpscli

import (
	"bytes"
	"context"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"regexp"
	"strings"
	"text/tabwriter"
	"time"
)

// maxBody bounds the response bodies that the client reads into memory.
const maxBody = 64 << 20

// idPattern is the pattern of identifiers in the path of the API contract.
var idPattern = regexp.MustCompile(`^[A-Za-z0-9_-]{1,64}$`)

func checkID(kind, id string) error {
	if !idPattern.MatchString(id) {
		return usagef("%s %q is not a valid identifier, want 1 to 64 letters, digits, underscores, or dashes", kind, id)
	}
	return nil
}

// Finding is one validation finding of the API contract.
type Finding struct {
	Severity string `json:"severity"`
	Path     string `json:"path"`
	Message  string `json:"message"`
}

// APIError is an RFC 9457 problem answer of the controller.
type APIError struct {
	Status   int
	Method   string
	Path     string
	Title    string
	Detail   string
	Findings []Finding
	// Scope is the scope that a 403 names in WWW-Authenticate.
	Scope     string
	sentToken bool
}

func (e *APIError) Error() string {
	msg := e.Title
	if msg == "" {
		msg = statusText(e.Status)
	}
	s := fmt.Sprintf("%s %s: HTTP %d %s", e.Method, e.Path, e.Status, msg)
	if e.Detail != "" && e.Detail != msg {
		s += ": " + e.Detail
	}
	return s
}

func statusText(code int) string {
	if t := http.StatusText(code); t != "" {
		return t
	}
	return "error"
}

func (e *APIError) exitCode() int {
	switch {
	case e.Status == http.StatusUnauthorized || e.Status == http.StatusForbidden:
		return ExitAuth
	case e.Status == http.StatusNotFound:
		return ExitNotFound
	default:
		return ExitAPI
	}
}

// hint returns advice for the errors that have a usual cause.
func (e *APIError) hint() string {
	switch e.Status {
	case http.StatusUnauthorized:
		if !e.sentToken {
			return "no API token was sent, set FPS_TOKEN_FILE, FPS_TOKEN, or --token-file"
		}
		return "the controller does not accept the API token"
	case http.StatusForbidden:
		if e.Scope != "" {
			return fmt.Sprintf("the token needs the scope %q for this operation", e.Scope)
		}
		return "the token lacks the scope that this operation needs"
	case http.StatusNotImplemented:
		return "this controller does not implement the operation yet"
	}
	return ""
}

func (e *APIError) printFindings(w io.Writer) {
	if len(e.Findings) == 0 {
		return
	}
	printFindings(w, e.Findings)
}

// NetError is a failure to talk to the controller at all.
type NetError struct {
	Err  error
	Hint string
}

func (e *NetError) Error() string {
	if e.Hint != "" {
		return e.Err.Error() + " (" + e.Hint + ")"
	}
	return e.Err.Error()
}

// Unwrap returns the underlying error.
func (e *NetError) Unwrap() error { return e.Err }

// Client talks to the controller. It never follows a redirect, so the token
// never reaches another host.
type Client struct {
	base    *url.URL
	token   string
	http    *http.Client
	timeout time.Duration
}

// newClient builds a client from the global settings. It reads the token from
// the token file or the environment.
func (a *app) newClient() (*Client, error) {
	u, err := url.Parse(strings.TrimRight(a.g.URL, "/"))
	if err != nil || u.Host == "" || (u.Scheme != "http" && u.Scheme != "https") {
		return nil, usagef("--url %q is not an http or https URL", a.g.URL)
	}
	u.User = nil
	u.RawQuery, u.Fragment = "", ""
	token, err := a.readToken()
	if err != nil {
		return nil, err
	}
	if token != "" && u.Scheme == "http" && !isLoopbackHost(u.Hostname()) && !a.g.InsecureHTTP {
		return nil, usagef("refusing to send the API token over plain http to %s; use https, or pass --insecure-http", u.Host)
	}
	tr := http.DefaultTransport.(*http.Transport).Clone() //nolint:errcheck // the default transport has this type
	tr.ResponseHeaderTimeout = a.g.Timeout
	tlsCfg := &tls.Config{MinVersion: tls.VersionTLS12}
	if a.g.CAFile != "" {
		pool, err := x509.SystemCertPool()
		if err != nil || pool == nil {
			pool = x509.NewCertPool()
		}
		pem, err := os.ReadFile(a.g.CAFile) //nolint:gosec // G304: the user names the CA file
		if err != nil {
			return nil, usagef("read --ca-file: %v", err)
		}
		if !pool.AppendCertsFromPEM(pem) {
			return nil, usagef("--ca-file %s holds no PEM certificate", a.g.CAFile)
		}
		tlsCfg.RootCAs = pool
	}
	tr.TLSClientConfig = tlsCfg
	return &Client{
		base:    u,
		token:   token,
		timeout: a.g.Timeout,
		http: &http.Client{
			Transport: tr,
			CheckRedirect: func(*http.Request, []*http.Request) error {
				return http.ErrUseLastResponse
			},
		},
	}, nil
}

// readToken returns the token of the token file flag, of FPS_TOKEN_FILE, or of
// FPS_TOKEN, in that order, or "" when none is set.
func (a *app) readToken() (string, error) {
	file := firstNonEmpty(a.g.TokenFile, a.env.Getenv("FPS_TOKEN_FILE"))
	token := ""
	if file != "" {
		data, err := os.ReadFile(file) //nolint:gosec // G304: the user names the token file
		if err != nil {
			return "", usagef("read the token file: %v", err)
		}
		if st, err := os.Stat(file); err == nil && st.Mode().Perm()&0o077 != 0 {
			a.eprintf("fps: warning: token file %s is readable by other users, run chmod 600 on it\n", file)
		}
		token = strings.TrimSpace(string(data))
		if token == "" {
			return "", usagef("the token file %s is empty", file)
		}
	} else {
		token = strings.TrimSpace(a.env.Getenv("FPS_TOKEN"))
	}
	if strings.ContainsAny(token, "\r\n\x00") {
		return "", usagef("the API token holds a control character")
	}
	return token, nil
}

func isLoopbackHost(host string) bool {
	if strings.EqualFold(host, "localhost") {
		return true
	}
	ip := net.ParseIP(host)
	return ip != nil && ip.IsLoopback()
}

// request describes one call.
type request struct {
	method string
	path   string // path below /v1, such as /runs/r1:stop
	query  url.Values
	body   []byte
	ctype  string
	accept string
}

func (c *Client) newRequest(ctx context.Context, r request) (*http.Request, error) {
	u := *c.base
	u.Path = strings.TrimRight(c.base.Path, "/") + "/v1" + r.path
	u.RawPath = ""
	if len(r.query) > 0 {
		u.RawQuery = r.query.Encode()
	}
	var body io.Reader
	if r.body != nil {
		body = bytes.NewReader(r.body)
	}
	req, err := http.NewRequestWithContext(ctx, r.method, u.String(), body)
	if err != nil {
		return nil, err
	}
	if c.token != "" {
		req.Header.Set("Authorization", "Bearer "+c.token)
	}
	if r.ctype != "" {
		req.Header.Set("Content-Type", r.ctype)
	}
	accept := r.accept
	if accept == "" {
		accept = "application/json"
	}
	req.Header.Set("Accept", accept+", application/problem+json")
	req.Header.Set("User-Agent", "fps-cli")
	return req, nil
}

// do sends r and returns the response of a 2xx status. Any other status
// becomes an *APIError and a transport failure a *NetError.
func (c *Client) do(ctx context.Context, r request) (*http.Response, error) {
	req, err := c.newRequest(ctx, r)
	if err != nil {
		return nil, &NetError{Err: c.scrub(err)}
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return nil, c.netError(err)
	}
	if resp.StatusCode >= 200 && resp.StatusCode < 300 {
		return resp, nil
	}
	defer func() { _ = resp.Body.Close() }()
	return nil, c.problem(r, resp)
}

func (c *Client) netError(err error) error {
	err = c.scrub(err)
	ne := &NetError{Err: err}
	var unknown x509.UnknownAuthorityError
	var invalid x509.CertificateInvalidError
	var host x509.HostnameError
	if errors.As(err, &unknown) || errors.As(err, &invalid) || errors.As(err, &host) {
		ne.Hint = "TLS verification failed, pass the controller CA with --ca-file or FPS_CA_FILE"
	}
	var ctxErr interface{ Timeout() bool }
	if errors.As(err, &ctxErr) && ctxErr.Timeout() {
		ne.Hint = "timed out, see --timeout"
	}
	return ne
}

// scrub removes the token from an error text, so no path can echo it.
func (c *Client) scrub(err error) error {
	if err == nil || c.token == "" || !strings.Contains(err.Error(), c.token) {
		return err
	}
	return errors.New(strings.ReplaceAll(err.Error(), c.token, "[redacted]"))
}

func (c *Client) problem(r request, resp *http.Response) error {
	e := &APIError{Status: resp.StatusCode, Method: r.method, Path: "/v1" + r.path, sentToken: c.token != ""}
	data, _ := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if resp.StatusCode >= 300 && resp.StatusCode < 400 {
		e.Title = "redirect refused"
		e.Detail = "the controller answered with a redirect, which the client never follows"
		return e
	}
	var p struct {
		Title  string    `json:"title"`
		Detail string    `json:"detail"`
		Errors []Finding `json:"errors"`
	}
	if json.Unmarshal(data, &p) == nil && (p.Title != "" || p.Detail != "" || len(p.Errors) > 0) {
		e.Title, e.Detail, e.Findings = p.Title, p.Detail, p.Errors
	} else if s := strings.TrimSpace(string(data)); s != "" {
		if len(s) > 200 {
			s = s[:200] + "..."
		}
		e.Detail = s
	}
	if c.token != "" {
		e.Title = strings.ReplaceAll(e.Title, c.token, "[redacted]")
		e.Detail = strings.ReplaceAll(e.Detail, c.token, "[redacted]")
	}
	if auth := resp.Header.Get("WWW-Authenticate"); auth != "" {
		e.Scope = authScope(auth)
	}
	return e
}

// authScope extracts scope="..." from a WWW-Authenticate header.
func authScope(h string) string {
	const key = `scope="`
	i := strings.Index(h, key)
	if i < 0 {
		return ""
	}
	rest := h[i+len(key):]
	if j := strings.IndexByte(rest, '"'); j >= 0 {
		return rest[:j]
	}
	return ""
}

// fetch sends r with the request timeout and returns the whole body.
func (c *Client) fetch(ctx context.Context, r request) ([]byte, error) {
	ctx, cancel := context.WithTimeout(ctx, c.timeout)
	defer cancel()
	resp, err := c.do(ctx, r)
	if err != nil {
		return nil, err
	}
	defer func() { _ = resp.Body.Close() }()
	data, err := io.ReadAll(io.LimitReader(resp.Body, maxBody))
	if err != nil {
		return nil, c.netError(err)
	}
	return data, nil
}

// fetchJSON sends r and decodes the JSON body into out when out is not nil.
// It returns the raw body, so the JSON output mode can print it unchanged.
func (c *Client) fetchJSON(ctx context.Context, r request, out any) ([]byte, error) {
	data, err := c.fetch(ctx, r)
	if err != nil {
		return nil, err
	}
	if out != nil {
		if err := json.Unmarshal(data, out); err != nil {
			return nil, fmt.Errorf("%s %s: decode the answer: %w", r.method, r.path, err)
		}
	}
	return data, nil
}

// Printing helpers shared by the commands.

func printFindings(w io.Writer, fs []Finding) {
	tw := tabwriter.NewWriter(w, 0, 4, 2, ' ', 0)
	_, _ = fmt.Fprintln(tw, "SEVERITY\tPATH\tMESSAGE")
	for _, f := range fs {
		_, _ = fmt.Fprintf(tw, "%s\t%s\t%s\n", f.Severity, f.Path, f.Message)
	}
	_ = tw.Flush()
}
