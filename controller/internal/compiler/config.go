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
	// Modules holds the settings of each module by module name.
	Modules map[string]ModuleConfig `yaml:"modules" json:"modules"`
}

// ModuleConfig holds the settings that hold for every scenario of one module
// in the test. The settings schema of the module manifest describes them.
type ModuleConfig struct {
	Settings map[string]any `yaml:"settings" json:"settings"`
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

// Port is one traffic port. Its location is <agent>/<interface>, which pins
// the port to that agent, or <interface> or */<interface>, which spreads the
// port across every agent that has the interface.
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
	// Arrival and Size are the optional declarations of test configuration
	// v0.3 (CON-19). They are targets that the realism check follows and add
	// no command to a plan, see Declarations.
	Arrival *Arrival `yaml:"arrival,omitempty" json:"arrival,omitempty"`
	Size    *Size    `yaml:"size,omitempty" json:"size,omitempty"`
}

// MixConfig is one entry of the profile mix.
type MixConfig struct {
	Scenario   string         `yaml:"scenario" json:"scenario"`
	Weight     float64        `yaml:"weight" json:"weight"`
	ClientPool string         `yaml:"client_pool" json:"client_pool"`
	ServerPool string         `yaml:"server_pool" json:"server_pool"`
	Params     map[string]any `yaml:"params" json:"params"`
	// Arrival and Size override the profile declarations for this entry.
	Arrival *Arrival `yaml:"arrival,omitempty" json:"arrival,omitempty"`
	Size    *Size    `yaml:"size,omitempty" json:"size,omitempty"`
}

// Arrival is a distribution of inter arrival times, as the schema definition
// arrival writes it. Dist names the distribution, and each other field is
// the parameter of some distribution, so a field that the distribution does
// not use stays nil. The schema checks which parameters each distribution
// takes, and the compiler only carries the declaration.
type Arrival struct {
	Dist        string   `yaml:"dist" json:"dist"`
	Ms          *float64 `yaml:"ms,omitempty" json:"ms,omitempty"`
	ToleranceMs *float64 `yaml:"tolerance_ms,omitempty" json:"tolerance_ms,omitempty"`
	MeanMs      *float64 `yaml:"mean_ms,omitempty" json:"mean_ms,omitempty"`
	MinMs       *float64 `yaml:"min_ms,omitempty" json:"min_ms,omitempty"`
	MaxMs       *float64 `yaml:"max_ms,omitempty" json:"max_ms,omitempty"`
	Mu          *float64 `yaml:"mu,omitempty" json:"mu,omitempty"`
	Sigma       *float64 `yaml:"sigma,omitempty" json:"sigma,omitempty"`
	Unit        string   `yaml:"unit,omitempty" json:"unit,omitempty"`
	Alpha       *float64 `yaml:"alpha,omitempty" json:"alpha,omitempty"`
	ScaleMs     *float64 `yaml:"scale_ms,omitempty" json:"scale_ms,omitempty"`
}

// SizeDist is a distribution of payload sizes in bytes, as the schema
// definition size_distribution writes it.
type SizeDist struct {
	Dist      string   `yaml:"dist" json:"dist"`
	Value     *float64 `yaml:"value,omitempty" json:"value,omitempty"`
	Tolerance *float64 `yaml:"tolerance,omitempty" json:"tolerance,omitempty"`
	Mean      *float64 `yaml:"mean,omitempty" json:"mean,omitempty"`
	Min       *float64 `yaml:"min,omitempty" json:"min,omitempty"`
	Max       *float64 `yaml:"max,omitempty" json:"max,omitempty"`
	Mu        *float64 `yaml:"mu,omitempty" json:"mu,omitempty"`
	Sigma     *float64 `yaml:"sigma,omitempty" json:"sigma,omitempty"`
	Alpha     *float64 `yaml:"alpha,omitempty" json:"alpha,omitempty"`
	Scale     *float64 `yaml:"scale,omitempty" json:"scale,omitempty"`
}

// Size holds the payload size distributions by direction: Request is the
// client to server direction, Response the server to client direction, and
// Both the two pooled.
type Size struct {
	Request  *SizeDist `yaml:"request,omitempty" json:"request,omitempty"`
	Response *SizeDist `yaml:"response,omitempty" json:"response,omitempty"`
	Both     *SizeDist `yaml:"both,omitempty" json:"both,omitempty"`
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

// Timeline holds the seed and the warm up period. A missing seed makes the
// compiler choose a random nonzero one, and the minimum is 1.
type Timeline struct {
	Seed    *uint64 `yaml:"seed" json:"seed"`
	WarmupS float64 `yaml:"warmup_s" json:"warmup_s"`
}

// CriterionConfig is one pass and fail rule.
type CriterionConfig struct {
	Metric string  `yaml:"metric" json:"metric"`
	Op     string  `yaml:"op" json:"op"`
	Value  float64 `yaml:"value" json:"value"`
	// Where is the label filter of the criterion, for example
	// {application: dns}. The runs package applies it.
	Where map[string]string `yaml:"where,omitempty" json:"where,omitempty"`
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
