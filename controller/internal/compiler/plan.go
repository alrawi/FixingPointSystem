// SPDX-License-Identifier: GPL-3.0-only

package compiler

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math"
	"strconv"
	"strings"
)

// Plan mirrors the canonical proto3 JSON mapping of fps.agent.v1.Plan in
// contracts/proto/fps/agent/v1/agent.proto. Field names are lowerCamelCase,
// enums are written by name, the seed is a string, durations are strings,
// and fields that hold their default value are left out.
type Plan struct {
	PlanID          string           `json:"planId,omitempty"`
	RunID           string           `json:"runId,omitempty"`
	AgentID         string           `json:"agentId,omitempty"`
	TestName        string           `json:"testName,omitempty"`
	EngineAPI       string           `json:"engineApi,omitempty"`
	Seed            uint64           `json:"seed,omitempty,string"`
	Topology        string           `json:"topology,omitempty"`
	Backend         string           `json:"backend,omitempty"`
	Modules         []PlanModule     `json:"modules,omitempty"`
	Scenarios       []ScenarioSource `json:"scenarios,omitempty"`
	Mix             []MixEntry       `json:"mix,omitempty"`
	Ports           []PortBinding    `json:"ports,omitempty"`
	HostPools       []HostPoolSlice  `json:"hostPools,omitempty"`
	ExternalServers []ExternalServer `json:"externalServers,omitempty"`
	Workers         []WorkerPlan     `json:"workers,omitempty"`
	Load            *LoadPlan        `json:"load,omitempty"`
	Warmup          Duration         `json:"warmup,omitempty"`
	Criteria        []Criterion      `json:"criteria,omitempty"`
}

// ModuleRef pins one module by name, version, and digest, and carries the
// signature that the registry stored for it. The struct stays comparable, so
// the signature is the standard base64 text that proto3 JSON uses for bytes.
type ModuleRef struct {
	Name    string `json:"name,omitempty"`
	Version string `json:"version,omitempty"`
	Digest  string `json:"digest,omitempty"`
	// Signature is the base64 encoding of the Ed25519 signature over the UTF-8
	// bytes of Digest. It is empty for a module without a signature.
	Signature string `json:"signature,omitempty"`
	// SignatureKeyID names the key that made Signature, and is empty when
	// Signature is.
	SignatureKeyID string `json:"signatureKeyId,omitempty"`
}

// PlanModule is a ModuleRef of the plan together with the settings of the
// test for the module. The settings field is not part of
// fps.agent.v1.ModuleRef yet, so the plan carries it ahead of the contract
// and leaves it out when the test sets nothing for the module.
type PlanModule struct {
	Name    string `json:"name,omitempty"`
	Version string `json:"version,omitempty"`
	Digest  string `json:"digest,omitempty"`
	// Signature and SignatureKeyID are the fields of fps.agent.v1.ModuleRef of
	// the same names. The JSON text of the bytes field is base64, which is
	// what ModuleRef.Signature holds.
	Signature      string         `json:"signature,omitempty"`
	SignatureKeyID string         `json:"signatureKeyId,omitempty"`
	Settings       map[string]any `json:"settings,omitempty"`
}

// ScenarioSource carries one scenario file and the SHA-256 digest of its bytes.
type ScenarioSource struct {
	Ref    string `json:"ref,omitempty"`
	Module string `json:"module,omitempty"`
	Source string `json:"source,omitempty"`
	Digest string `json:"digest,omitempty"`
}

// MixEntry is one resolved entry of the profile mix with its normalized weight.
type MixEntry struct {
	ID         string         `json:"id,omitempty"`
	Scenario   string         `json:"scenario,omitempty"`
	Weight     float64        `json:"weight,omitempty"`
	Params     map[string]any `json:"params,omitempty"`
	ClientPool string         `json:"clientPool,omitempty"`
	ServerPool string         `json:"serverPool,omitempty"`
}

// PortBinding maps a test port to a local interface.
type PortBinding struct {
	Name          string `json:"name,omitempty"`
	InterfaceName string `json:"interfaceName,omitempty"`
	MTU           uint32 `json:"mtu,omitempty"`
}

// HostPoolSlice is the part of one host pool that an agent emulates.
type HostPoolSlice struct {
	Name       string `json:"name,omitempty"`
	Port       string `json:"port,omitempty"`
	Role       string `json:"role,omitempty"`
	IPv4Prefix string `json:"ipv4Prefix,omitempty"`
	IPv6Prefix string `json:"ipv6Prefix,omitempty"`
	PoolCount  uint32 `json:"poolCount,omitempty"`
	Offset     uint32 `json:"offset,omitempty"`
	Count      uint32 `json:"count,omitempty"`
	Gateway    string `json:"gateway,omitempty"`
	VLAN       uint32 `json:"vlan,omitempty"`
	MACBase    string `json:"macBase,omitempty"`
	OSProfile  string `json:"osProfile,omitempty"`
}

// ExternalServer is a real server for the client_sim topology.
type ExternalServer struct {
	Name    string `json:"name,omitempty"`
	Address string `json:"address,omitempty"`
	Port    uint32 `json:"port,omitempty"`
}

// WorkerPlan is the share of one worker, which pins to one core.
type WorkerPlan struct {
	WorkerID uint32       `json:"workerId,omitempty"`
	Core     uint32       `json:"core,omitempty"`
	Hosts    []HostRange  `json:"hosts,omitempty"`
	Rates    []WorkerRate `json:"rates,omitempty"`
}

// HostRange is a contiguous range of hosts of one pool.
type HostRange struct {
	Pool   string `json:"pool,omitempty"`
	Offset uint32 `json:"offset,omitempty"`
	Count  uint32 `json:"count,omitempty"`
}

// WorkerRate is the steady state target of one mix entry on one worker.
// Exactly one target is set. The targets are pointers because a oneof member
// keeps its presence in the JSON mapping even when it holds zero.
type WorkerRate struct {
	MixEntry           string   `json:"mixEntry,omitempty"`
	SessionsPerSecond  *float64 `json:"sessionsPerSecond,omitempty"`
	ConcurrentSessions *uint32  `json:"concurrentSessions,omitempty"`
}

// LoadPlan holds the objective, the share of its value for one agent, and the phases.
type LoadPlan struct {
	Objective      string      `json:"objective,omitempty"`
	ObjectiveValue float64     `json:"objectiveValue,omitempty"`
	Phases         []LoadPhase `json:"phases,omitempty"`
}

// LoadPhase scales every worker rate over one time span.
type LoadPhase struct {
	Name          string   `json:"name,omitempty"`
	Duration      Duration `json:"duration,omitempty"`
	Shape         string   `json:"shape,omitempty"`
	StartFraction float64  `json:"startFraction,omitempty"`
	EndFraction   float64  `json:"endFraction,omitempty"`
	Steps         uint32   `json:"steps,omitempty"`
}

// Criterion is one pass and fail rule.
type Criterion struct {
	Metric string  `json:"metric,omitempty"`
	Op     string  `json:"op,omitempty"`
	Value  float64 `json:"value,omitempty"`
}

// Duration is a google.protobuf.Duration in its JSON form, such as "30s" or
// "1.500s". The empty string means zero and is left out.
type Duration string

// durationFromSeconds renders seconds in the JSON form of
// google.protobuf.Duration, which uses 0, 3, 6, or 9 fractional digits.
func durationFromSeconds(s float64) (Duration, error) {
	if math.IsNaN(s) || math.IsInf(s, 0) || s < 0 || s > 315576000000 {
		return "", fmt.Errorf("duration %v s is out of range", s)
	}
	nanos := int64(math.Round(s * 1e9))
	if nanos == 0 {
		return "", nil
	}
	secs, frac := nanos/1e9, nanos%1e9
	out := strconv.FormatInt(secs, 10)
	switch {
	case frac == 0:
	case frac%1e6 == 0:
		out += fmt.Sprintf(".%03d", frac/1e6)
	case frac%1e3 == 0:
		out += fmt.Sprintf(".%06d", frac/1e3)
	default:
		out += fmt.Sprintf(".%09d", frac)
	}
	return Duration(out + "s"), nil
}

// MarshalCanonical returns the plan as indented canonical JSON with a final
// newline, in the layout of contracts/examples/plans/.
func (p Plan) MarshalCanonical() ([]byte, error) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", "  ")
	if err := enc.Encode(p); err != nil {
		return nil, fmt.Errorf("marshal plan %s: %w", p.PlanID, err)
	}
	return buf.Bytes(), nil
}

// enumName converts a lower case configuration value such as app_sim into the
// proto enum name with the given prefix, such as TOPOLOGY_APP_SIM.
func enumName(prefix, value string) string {
	return prefix + strings.ToUpper(value)
}
