// SPDX-License-Identifier: GPL-3.0-only

package compiler

import (
	"bytes"
	"errors"
	"fmt"
	"io"

	"go.yaml.in/yaml/v3"
)

// Config mirrors contracts/schemas/test-config.schema.json. The compiler
// checks the rules that it depends on, and schema validation of the whole
// document belongs to the configuration store.
type Config struct {
	APIVersion string            `yaml:"apiVersion" json:"apiVersion"`
	Kind       string            `yaml:"kind" json:"kind"`
	Metadata   Metadata          `yaml:"metadata" json:"metadata"`
	Topology   string            `yaml:"topology" json:"topology"`
	Network    Network           `yaml:"network" json:"network"`
	Profile    Profile           `yaml:"profile" json:"profile"`
	Load       Load              `yaml:"load" json:"load"`
	Timeline   Timeline          `yaml:"timeline" json:"timeline"`
	Criteria   []CriterionConfig `yaml:"criteria" json:"criteria"`
}

// Metadata names the test.
type Metadata struct {
	Name        string            `yaml:"name" json:"name"`
	Description string            `yaml:"description" json:"description"`
	Labels      map[string]string `yaml:"labels" json:"labels"`
}

// Network holds the ports, host pools, and external servers.
type Network struct {
	Ports           []Port           `yaml:"ports" json:"ports"`
	HostPools       []HostPool       `yaml:"host_pools" json:"host_pools"`
	ExternalServers []ExternalConfig `yaml:"external_servers" json:"external_servers"`
}

// Port is one traffic port at <agent>/<interface>.
type Port struct {
	Name     string `yaml:"name" json:"name"`
	Location string `yaml:"location" json:"location"`
	MTU      int64  `yaml:"mtu" json:"mtu"`
}

// HostPool is one range of emulated hosts on a port.
type HostPool struct {
	Name      string `yaml:"name" json:"name"`
	Port      string `yaml:"port" json:"port"`
	Role      string `yaml:"role" json:"role"`
	IPv4      string `yaml:"ipv4" json:"ipv4"`
	IPv6      string `yaml:"ipv6" json:"ipv6"`
	Count     int64  `yaml:"count" json:"count"`
	Gateway   string `yaml:"gateway" json:"gateway"`
	VLAN      int64  `yaml:"vlan" json:"vlan"`
	MACBase   string `yaml:"mac_base" json:"mac_base"`
	OSProfile string `yaml:"os_profile" json:"os_profile"`
}

// ExternalConfig is a real server for the client_sim topology.
type ExternalConfig struct {
	Name    string `yaml:"name" json:"name"`
	Address string `yaml:"address" json:"address"`
	Port    int64  `yaml:"port" json:"port"`
}

// Profile holds the weight basis and the mix.
type Profile struct {
	Basis string      `yaml:"basis" json:"basis"`
	Mix   []MixConfig `yaml:"mix" json:"mix"`
}

// MixConfig is one entry of the profile mix.
type MixConfig struct {
	Scenario   string         `yaml:"scenario" json:"scenario"`
	Weight     float64        `yaml:"weight" json:"weight"`
	ClientPool string         `yaml:"client_pool" json:"client_pool"`
	ServerPool string         `yaml:"server_pool" json:"server_pool"`
	Params     map[string]any `yaml:"params" json:"params"`
}

// Load holds the objective and the phases.
type Load struct {
	Objective Objective     `yaml:"objective" json:"objective"`
	Phases    []PhaseConfig `yaml:"phases" json:"phases"`
}

// Objective is the load target of the whole test.
type Objective struct {
	Type  string  `yaml:"type" json:"type"`
	Value float64 `yaml:"value" json:"value"`
}

// PhaseConfig is one load phase.
type PhaseConfig struct {
	Name      string  `yaml:"name" json:"name"`
	DurationS float64 `yaml:"duration_s" json:"duration_s"`
	Shape     string  `yaml:"shape" json:"shape"`
}

// Timeline holds the seed and the warm up period.
type Timeline struct {
	Seed    *uint64 `yaml:"seed" json:"seed"`
	WarmupS float64 `yaml:"warmup_s" json:"warmup_s"`
}

// CriterionConfig is one pass and fail rule.
type CriterionConfig struct {
	Metric string  `yaml:"metric" json:"metric"`
	Op     string  `yaml:"op" json:"op"`
	Value  float64 `yaml:"value" json:"value"`
}

// ParseConfig decodes one YAML test configuration and rejects unknown keys.
func ParseConfig(data []byte) (Config, error) {
	var cfg Config
	dec := yaml.NewDecoder(bytes.NewReader(data))
	dec.KnownFields(true)
	if err := dec.Decode(&cfg); err != nil {
		if errors.Is(err, io.EOF) {
			return Config{}, fmt.Errorf("parse test configuration: %w: empty document", ErrInvalidConfig)
		}
		return Config{}, fmt.Errorf("parse test configuration: %w: %w", ErrInvalidConfig, err)
	}
	return cfg, nil
}
