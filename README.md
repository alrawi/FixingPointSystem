# FixingPointSystem

FixingPointSystem is an open source, modular generator of realistic application traffic for testing firewalls, intrusion prevention systems, proxies, NAT devices, and load balancers. A controller compiles test configurations, agents execute them over real TCP, UDP, and TLS sessions, and every application protocol ships as an independent, versioned module.

> **Status: Phase 0 (spikes and contracts).** The repository holds the v1 contracts and four measurement spikes. No component is ready for production use. The [Gate 0 report](docs/phase0/gate0-report.md) summarizes the spike results.

## Repository layout

| Path | Contents |
|---|---|
| `docs/spec/specification.md` | Feasibility assessment and v1 specification |
| `docs/decisions/` | Architecture decision records for decisions D1 through D9 |
| `docs/phase0/` | Phase 0 plan, spike briefs, and the Gate 0 report |
| `contracts/` | OpenAPI document, JSON Schemas for modules, scenarios, and tests, plus examples |
| `tools/contracts/` | Contract validator and its tests |
| `spikes/s1-engine/` | Rust prototype of the flow program interpreter over kernel sockets |
| `spikes/s2-tls/` | Rust benchmark of TLS handshake cost per core |
| `spikes/s3-afxdp/` | Harness for AF_XDP throughput on KVM guests with virtio-net |
| `spikes/s4-trex-astf/` | Translator from our test configuration to TRex ASTF profiles |

## Quick start

We require current stable Rust (1.87 or newer) and Python 3.10 or newer.

```bash
# Validate every contract and example
python3 -m pip install -r tools/contracts/requirements.txt
python3 tools/contracts/validate.py
python3 -m pytest tools/contracts spikes/s4-trex-astf

# Build and test the Rust spikes
cargo test --workspace

# Run the engine spike on loopback for 10 seconds
cargo run --release -p fps-spike-engine -- bench \
  --scenario contracts/examples/modules/http1/scenarios/web_get.yaml \
  --concurrency 256 --duration 10

# Run the TLS handshake benchmark
cargo run --release -p fps-spike-tls -- --duration 5
```

## Decisions

| ID | Decision | Choice |
|---|---|---|
| D1 | Engine strategy | Own controller and flow programs; kernel backend first; TRex ASTF evaluated as a scale adapter |
| D2 | Languages | Rust agent, Go controller |
| D3 | Extension sandbox | WebAssembly |
| D4 | Core license | GPL-3.0 |
| D5 | API baseline | Independent REST API with Open Traffic Generator vocabulary |
| D6 | Image base and builder | Ubuntu LTS with Packer |
| D7 | v1 application list | 15 module enterprise mix |
| D8 | User interface | Full web UI in v1 |
| D9 | Team model | Swarm of AI coding agents |

## Authorized use

FixingPointSystem generates high volumes of traffic. We support its use only on networks and devices that the operator owns or has written permission to test. Keep test interfaces isolated from production networks.

## License

FixingPointSystem is free software under the GNU General Public License version 3. See [LICENSE](LICENSE). Every module and every library linked into the agent must carry a license compatible with GPL-3.0.
