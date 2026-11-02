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
)

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
	return nil
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
