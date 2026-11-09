// SPDX-License-Identifier: GPL-3.0-only

// Package app wires the controller process together. It loads the configuration,
// builds the logger, serves the HTTP endpoints, and shuts the server down when its
// context ends.
package app

import (
	"bytes"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"go.yaml.in/yaml/v3"
)

// ErrInvalidConfig reports that a configuration value is missing or out of range.
var ErrInvalidConfig = errors.New("invalid config")

// Default values of the configuration. The listen address matches the default
// server of contracts/openapi/fps-api-v1.yaml.
const (
	DefaultListen            = "localhost:8443"
	DefaultLogLevel          = "info"
	DefaultShutdownTimeout   = 15 * time.Second
	DefaultReadHeaderTimeout = 10 * time.Second
	// DefaultGRPCListen is the TCP address of the agent gRPC service.
	DefaultGRPCListen = "localhost:8444"
	// DefaultEnrollListen is the TCP address of the HTTPS enrollment and
	// renewal service for agent certificates.
	DefaultEnrollListen = "localhost:8445"
	// PKIDirName is the name of the certificate authority state under DataDir.
	PKIDirName = "pki"
	// AdminSocketFile is the name of the local admin socket under the PKI
	// directory.
	AdminSocketFile = "admin.sock"
	// maxSocketPath is the longest unix socket path that every supported
	// system accepts, which is the limit of sockaddr_un on macOS and BSD.
	maxSocketPath = 100
	// MemoryStorePath keeps the configuration store in memory.
	MemoryStorePath = ":memory:"
	// StoreFile is the name of the configuration store under DataDir.
	StoreFile = "configs.db"
	// ModulesDir is the name of the module registry under DataDir.
	ModulesDir = "modules"
	// MinTokenLength is the shortest API token that the controller accepts.
	MinTokenLength = 16
)

// Scopes lists the API token scopes from lowest to highest, as the REST
// contract names them in x-required-scope.
var Scopes = []string{"read", "run", "admin"}

// Config holds the settings of one controller process. The YAML keys appear in
// the struct tags, and the wiki page Controller documents them.
type Config struct {
	// Listen is the TCP address of the HTTP server, as host:port.
	Listen string `yaml:"listen"`
	// LogLevel is one of debug, info, warn, or error.
	LogLevel string `yaml:"log_level"`
	// ShutdownTimeout bounds how long in flight requests may run after a stop signal.
	ShutdownTimeout Duration `yaml:"shutdown_timeout"`
	// ReadHeaderTimeout bounds how long a client may take to send request headers.
	ReadHeaderTimeout Duration `yaml:"read_header_timeout"`
	// DataDir holds the module registry in DataDir/modules and, unless
	// StorePath names another file, the configuration store in
	// DataDir/configs.db. Empty keeps the store in memory and the registry in
	// a temporary directory, so all state ends with the process.
	DataDir string `yaml:"data_dir"`
	// StorePath is the SQLite file of the configuration store, or ":memory:".
	// Empty means DataDir/configs.db, or ":memory:" without DataDir.
	StorePath string `yaml:"store_path"`
	// GRPCListen is the TCP address of the agent gRPC service, as host:port.
	// The service requires mutual TLS unless InsecurePlainGRPC is set.
	GRPCListen string `yaml:"grpc_listen"`
	// EnrollListen is the TCP address of the HTTPS service where agents
	// exchange an enrollment token for a certificate and renew it, as
	// host:port. It is unused with InsecurePlainGRPC.
	EnrollListen string `yaml:"enroll_listen"`
	// PKIDir holds the controller CA and the PKI state. Empty means
	// DataDir/pki, or an in memory CA that ends with the process when DataDir
	// is empty too.
	PKIDir string `yaml:"pki_dir"`
	// TLSNames lists the host names and IP addresses that the controller
	// certificate carries, which agents check against the address they dial.
	// Empty means localhost, 127.0.0.1, ::1, and the specific hosts of
	// grpc_listen and enroll_listen. See ServerNames.
	TLSNames []string `yaml:"tls_names"`
	// AdminSocket is the unix socket where the token and revoke subcommands
	// reach the running controller. Empty means PKIDir/admin.sock, or no
	// socket when the PKI directory is empty.
	AdminSocket string `yaml:"admin_socket"`
	// InsecurePlainGRPC serves the agent gRPC service without TLS and trusts
	// the agent_id of each request. It exists for development and tests until
	// every agent speaks mutual TLS, and the controller logs a warning at
	// start up and on every registration. Never set it on a shared network.
	InsecurePlainGRPC bool `yaml:"insecure_plain_grpc"`
	// AgentAutoEnroll admits an agent whose identity is not in the inventory,
	// which POST /v1/agents or a minted enrollment token fills. With mutual TLS
	// the identity is the one in the client certificate.
	AgentAutoEnroll bool `yaml:"agent_auto_enroll"`
	// Tokens lists the bearer tokens of the REST API. Without tokens every
	// /v1 operation answers 401.
	Tokens []Token `yaml:"tokens"`
	// MetricsTokenFile names a file whose content, without surrounding white
	// space, is the bearer token that GET /metrics requires. Empty keeps the
	// endpoint open. The setting takes a file and not the secret itself, so
	// the token never appears in the configuration, in flags, or in logs.
	MetricsTokenFile string `yaml:"metrics_token_file"`
}

// Token is one bearer token of the REST API. Exactly one of Token and
// TokenFile is set.
type Token struct {
	// Name labels the token in logs. It never appears in a response.
	Name string `yaml:"name"`
	// Scope is read, run, or admin.
	Scope string `yaml:"scope"`
	// Token is the secret itself.
	Token string `yaml:"token"`
	// TokenFile names a file whose content, without surrounding white space,
	// is the secret.
	TokenFile string `yaml:"token_file"`
}

// Duration is a time.Duration that YAML reads from a Go duration string such as "15s".
type Duration time.Duration

// UnmarshalYAML parses a duration string such as "500ms" or "1m30s".
func (d *Duration) UnmarshalYAML(node *yaml.Node) error {
	var s string
	if err := node.Decode(&s); err != nil {
		return fmt.Errorf("line %d: duration must be a string such as \"15s\": %w", node.Line, err)
	}
	v, err := time.ParseDuration(s)
	if err != nil {
		return fmt.Errorf("line %d: %w", node.Line, err)
	}
	*d = Duration(v)
	return nil
}

// MarshalYAML writes the duration in the form that UnmarshalYAML reads.
func (d Duration) MarshalYAML() (any, error) {
	return time.Duration(d).String(), nil
}

// DefaultConfig returns the configuration that applies when no file and no flag
// set a value.
func DefaultConfig() Config {
	return Config{
		Listen:            DefaultListen,
		LogLevel:          DefaultLogLevel,
		ShutdownTimeout:   Duration(DefaultShutdownTimeout),
		ReadHeaderTimeout: Duration(DefaultReadHeaderTimeout),
		GRPCListen:        DefaultGRPCListen,
		EnrollListen:      DefaultEnrollListen,
	}
}

// LoadConfigFile reads the YAML file at path over the defaults and validates the
// result. A missing file yields an error that wraps fs.ErrNotExist.
func LoadConfigFile(path string) (Config, error) {
	data, err := os.ReadFile(filepath.Clean(path))
	if err != nil {
		return Config{}, fmt.Errorf("read config %s: %w", path, err)
	}
	cfg, err := ParseConfig(data)
	if err != nil {
		return Config{}, fmt.Errorf("config %s: %w", path, err)
	}
	return cfg, nil
}

// ParseConfig decodes one YAML document over the defaults and validates the
// result. Unknown keys are errors, so a misspelled key never passes silently. An
// empty document yields the defaults.
func ParseConfig(data []byte) (Config, error) {
	cfg := DefaultConfig()
	dec := yaml.NewDecoder(bytes.NewReader(data))
	dec.KnownFields(true)
	if err := dec.Decode(&cfg); err != nil && !errors.Is(err, io.EOF) {
		return Config{}, fmt.Errorf("decode: %w", err)
	}
	if err := cfg.Validate(); err != nil {
		return Config{}, err
	}
	return cfg, nil
}

// Validate checks every field and returns an error that wraps ErrInvalidConfig
// for the first field that fails.
func (c Config) Validate() error {
	if _, _, err := net.SplitHostPort(c.Listen); err != nil {
		return fmt.Errorf("%w: listen %q: %w", ErrInvalidConfig, c.Listen, err)
	}
	if _, err := ParseLogLevel(c.LogLevel); err != nil {
		return err
	}
	if c.ShutdownTimeout <= 0 {
		return fmt.Errorf("%w: shutdown_timeout must be positive", ErrInvalidConfig)
	}
	if c.ReadHeaderTimeout <= 0 {
		return fmt.Errorf("%w: read_header_timeout must be positive", ErrInvalidConfig)
	}
	if _, _, err := net.SplitHostPort(c.GRPCListen); err != nil {
		return fmt.Errorf("%w: grpc_listen %q: %w", ErrInvalidConfig, c.GRPCListen, err)
	}
	if _, _, err := net.SplitHostPort(c.EnrollListen); err != nil {
		return fmt.Errorf("%w: enroll_listen %q: %w", ErrInvalidConfig, c.EnrollListen, err)
	}
	for i, n := range c.TLSNames {
		if strings.TrimSpace(n) == "" || strings.ContainsAny(n, " /\t") {
			return fmt.Errorf("%w: tls_names[%d] %q is not a host name or an IP address", ErrInvalidConfig, i, n)
		}
	}
	if sock := c.ResolvedAdminSocket(); len(sock) > maxSocketPath {
		return fmt.Errorf("%w: admin socket path %q is longer than %d bytes; set admin_socket to a shorter path", ErrInvalidConfig, sock, maxSocketPath)
	}
	names := make(map[string]bool, len(c.Tokens))
	for i, t := range c.Tokens {
		if err := t.validate(); err != nil {
			return fmt.Errorf("%w: tokens[%d]: %w", ErrInvalidConfig, i, err)
		}
		if names[t.Name] {
			return fmt.Errorf("%w: tokens[%d]: name %q repeats", ErrInvalidConfig, i, t.Name)
		}
		names[t.Name] = true
	}
	return nil
}

func (t Token) validate() error {
	if t.Name == "" {
		return errors.New("name is empty")
	}
	if !slices.Contains(Scopes, t.Scope) {
		return fmt.Errorf("scope %q is not read, run, or admin", t.Scope)
	}
	switch {
	case t.Token != "" && t.TokenFile != "":
		return errors.New("token and token_file are both set")
	case t.Token == "" && t.TokenFile == "":
		return errors.New("neither token nor token_file is set")
	case t.Token != "" && len(t.Token) < MinTokenLength:
		return fmt.Errorf("token is shorter than %d bytes", MinTokenLength)
	}
	return nil
}

// ResolvedStorePath returns the path of the configuration store that the
// settings select.
func (c Config) ResolvedStorePath() string {
	switch {
	case c.StorePath != "":
		return c.StorePath
	case c.DataDir != "":
		return filepath.Join(c.DataDir, StoreFile)
	default:
		return MemoryStorePath
	}
}

// RegistryDir returns the root of the module registry, or an empty string
// when DataDir is empty and the registry lives in a temporary directory.
func (c Config) RegistryDir() string {
	if c.DataDir == "" {
		return ""
	}
	return filepath.Join(c.DataDir, ModulesDir)
}

// ResolvedPKIDir returns the directory of the controller CA, or an empty string
// for an in memory CA.
func (c Config) ResolvedPKIDir() string {
	switch {
	case c.PKIDir != "":
		return c.PKIDir
	case c.DataDir != "":
		return filepath.Join(c.DataDir, PKIDirName)
	default:
		return ""
	}
}

// ResolvedAdminSocket returns the path of the admin socket, or an empty string
// when the controller serves none.
func (c Config) ResolvedAdminSocket() string {
	switch {
	case c.AdminSocket != "":
		return c.AdminSocket
	case c.ResolvedPKIDir() != "":
		return filepath.Join(c.ResolvedPKIDir(), AdminSocketFile)
	default:
		return ""
	}
}

// ServerNames returns the host names and IP addresses of the controller
// certificate. TLSNames wins when it is set. Otherwise the names are localhost,
// 127.0.0.1, ::1, and the host of grpc_listen and of enroll_listen when it
// names one address, so a controller that listens on 10.0.0.5 gets a
// certificate for 10.0.0.5 without more settings. A wildcard host such as
// 0.0.0.0 adds nothing, so a controller on every interface sets tls_names to
// the names that agents dial.
func (c Config) ServerNames() []string {
	if len(c.TLSNames) > 0 {
		return slices.Clone(c.TLSNames)
	}
	names := []string{"localhost", "127.0.0.1", "::1"}
	for _, addr := range []string{c.GRPCListen, c.EnrollListen} {
		host, _, err := net.SplitHostPort(addr)
		if err != nil || host == "" {
			continue
		}
		if ip := net.ParseIP(host); ip != nil && ip.IsUnspecified() {
			continue
		}
		if !slices.Contains(names, host) {
			names = append(names, host)
		}
	}
	return names
}

// LoadTokens returns every API token with its scope name, reading each
// token_file. It fails when a file is unreadable, holds a token shorter than
// MinTokenLength, or two entries hold the same secret.
func (c Config) LoadTokens() (map[string]string, error) {
	out := make(map[string]string, len(c.Tokens))
	for _, t := range c.Tokens {
		secret := t.Token
		if t.TokenFile != "" {
			data, err := os.ReadFile(filepath.Clean(t.TokenFile))
			if err != nil {
				return nil, fmt.Errorf("%w: token %s: %w", ErrInvalidConfig, t.Name, err)
			}
			secret = strings.TrimSpace(string(data))
			if len(secret) < MinTokenLength {
				return nil, fmt.Errorf("%w: token %s: token_file %s holds fewer than %d bytes", ErrInvalidConfig, t.Name, t.TokenFile, MinTokenLength)
			}
		}
		if _, dup := out[secret]; dup {
			return nil, fmt.Errorf("%w: token %s: another token has the same secret", ErrInvalidConfig, t.Name)
		}
		out[secret] = t.Scope
	}
	return out, nil
}

// LoadMetricsToken returns the bearer token that GET /metrics requires, read
// from MetricsTokenFile. It returns an empty string when the setting is empty,
// which leaves the endpoint open. It fails when the file is unreadable or holds
// fewer than MinTokenLength bytes.
func (c Config) LoadMetricsToken() (string, error) {
	if c.MetricsTokenFile == "" {
		return "", nil
	}
	data, err := os.ReadFile(filepath.Clean(c.MetricsTokenFile))
	if err != nil {
		return "", fmt.Errorf("%w: metrics_token_file: %w", ErrInvalidConfig, err)
	}
	secret := strings.TrimSpace(string(data))
	if len(secret) < MinTokenLength {
		return "", fmt.Errorf("%w: metrics_token_file %s holds fewer than %d bytes", ErrInvalidConfig, c.MetricsTokenFile, MinTokenLength)
	}
	return secret, nil
}

// ParseLogLevel maps debug, info, warn, or error to its slog level, ignoring case.
func ParseLogLevel(s string) (slog.Level, error) {
	switch strings.ToLower(s) {
	case "debug":
		return slog.LevelDebug, nil
	case "info":
		return slog.LevelInfo, nil
	case "warn":
		return slog.LevelWarn, nil
	case "error":
		return slog.LevelError, nil
	default:
		return 0, fmt.Errorf("%w: log_level %q is not debug, info, warn, or error", ErrInvalidConfig, s)
	}
}

// NewLogger returns a slog logger that writes JSON lines to w at the given level.
func NewLogger(w io.Writer, level string) (*slog.Logger, error) {
	l, err := ParseLogLevel(level)
	if err != nil {
		return nil, err
	}
	return slog.New(slog.NewJSONHandler(w, &slog.HandlerOptions{Level: l})), nil
}
