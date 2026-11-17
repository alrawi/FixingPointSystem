# CON-34 DHCP configuration of a host pool

Contract. Test configuration schema `schemas/test-config.schema.json` and agent control protocol `proto/fps/agent/v1/agent.proto`, plus a fix in the validator `tools/contracts/validate.py`. The change is issue #598. The vetting agent specified it and approved it on 2026-11-17 under decision D13, as an additive contract change.

Problem. HST-04 obtains host addresses through DHCP when a host pool configures it, and no schema field configures DHCP today. A pool also needs an address source, which the schema takes from `ipv4` or `ipv6`, so a DHCP pool has no way to say that it has none. The validator has a second defect that this package meets on the way: it checks the `gateway` of a pool against the prefix of both address families, so every dual stack pool with a gateway fails, whichever family the gateway is in.

## Versions

- The test configuration moves from v0.3 to v0.4. The change adds one optional property and widens one `anyOf`, so every configuration of v0.3 stays valid and keeps its meaning.
- The agent protocol moves from 0.2.5 to 0.2.6. The change adds a message and a field and no command, so it takes a patch number for the reason that CON-11, CON-13, CON-15, CON-24, and CON-32 gave: the controller admits agents by the major and minor part of `Capabilities.engine_api`, and a 0.3.0 would make it refuse every agent until its list changes. The header comment of the proto file, `buf.yaml`, `contracts/README.md`, and `docs/wiki/Contracts.md` name the new version. The Rust agent reports `ENGINE_API` 0.2.6 once it reads the field, because `engine/fps-agent/tests/contract.rs` ties `ENGINE_API` to the header comment.
- The REST API (0.13.0), the metric catalog, the scenario contract, and the module manifest schema stay as they are. The REST API holds a test configuration as an opaque body that the embedded schema checks, and the validator's own text changes no REST property.

## Schema changes

`$defs.host_pool` gains the optional property `dhcp4`.

| Rule | Text |
|---|---|
| `dhcp4` | An object with `additionalProperties: false` and no properties yet. Later options will be added as optional properties, which is additive |
| Absent | The hosts have static addresses from `ipv4` and `ipv6`, as before |
| `anyOf` | Gains `{ "required": ["dhcp4"] }`, so a pool needs one of `ipv4`, `ipv6`, and `dhcp4` |
| `if` `dhcp4` is present, `then` | `role` is `client`, and `ipv4`, `ipv6`, and `gateway` are not allowed (a `false` schema, so the error names the property) |

Clients compute the addresses of servers from the prefix of the server pool, as they do today. The lease supplies the address and the router of the client itself. `count` keeps its meaning, and the validator checks no address range for a DHCP pool, because the configuration names none.

Two cases are left open on purpose, because the specification did not decide them: a DHCP pool has no `vlan` rule beyond the existing one (`vlan` and `mac_base` stay allowed and keep their meaning), and the controller cannot check that the network of the port runs a DHCP server. Both are for HST-04 and a later option of the `dhcp4` object.

## Protocol changes

| Field | Number | Meaning |
|---|---|---|
| `HostPoolSlice.dhcp4` | 13 | Of the new message `Dhcp4`. Presence means DHCP, and `ipv4_prefix`, `ipv6_prefix`, and `gateway` are then all empty |
| `message Dhcp4 {}` | | Has no field yet. Later options will be added as optional fields |

Field number 13 of `HostPoolSlice` was free: the message ends at `os_profile = 12`, and the file declares no `reserved` range. Nothing is renumbered. (`Plan.host_pools = 13` is a field of another message.)

Plan rule 5 (`contracts/README.md`, Plan rules) gains the DHCP case. A slice with `dhcp4` has no prefix, so there is no i-th address to compute. Host i keeps the MAC address `mac_base` plus i, so MAC addresses stay unique across agents as before, and takes its address and its router from its lease. An agent that cannot lease (a backend with no DHCP client, or a port that has no DHCP server) fails Prepare with an error that names `dhcp4`.

The plan checker (`tools/contracts/check_plans.py`) accepts a slice with `dhcp4` and no prefix. It also rejects a slice with `dhcp4` and a prefix or a `gateway`, and a `dhcp4` slice that is not a client slice, because the compiler writes none of them. A slice with neither a prefix nor `dhcp4` still fails.

## Validator fix

`check_test_config` checked the `gateway` against `ipv4` and against `ipv6`, so a dual stack pool with a gateway failed with "outside" for the other family, and it reserved one address of each prefix. It now parses the gateway once and checks it against the prefix of its own family only, as `fps-hosts` matches it (`engine/fps-hosts/src/pool.rs`, `gateway_v4` and `gateway_v6`). The gateway then reserves one address of that family. A gateway of a family for which the pool has no prefix is an error that says so, where it was an "outside" error before. The plan checker already matched by family. Tests: a dual stack pool with an IPv4 gateway and one with an IPv6 gateway pass, a gateway outside its own family's prefix fails once, and the old validator fails the first two.

## Compatibility

The change is additive.

- `buf breaking` with the `FILE` rules reports no finding against `main`, because the change only adds a message and a field with a new number (CI checks it, because `buf` is not installed in the sandbox of this package). The proto file compiles with `grpc_tools.protoc`.
- Wire level. An agent of 0.2.5 skips field 13 and sees a slice with no prefix, so a controller should not send a DHCP slice to an agent that reports an earlier `engine_api` than 0.2.6 (CTL, for the compiler consumer below). The controller keeps admitting agents by the major and minor part of `engine_api`, so only a configuration with `dhcp4` is affected, and no existing configuration is.
- A client of the REST API that stores a configuration with `dhcp4` needs a controller whose embedded schema is v0.4. A controller of an earlier version rejects it with a finding at `/network/host_pools/<i>/dhcp4` (`additionalProperties`), which is the right outcome.
- Generated Go code. A literal `agentpb.HostPoolSlice{...}` with keyed fields still compiles. A Rust literal `pb::HostPoolSlice { ... }` without `..Default::default()` does not (see the consumers).

## Consumers

HST-04 comes first, because it waits for this package. The order is the order of the work. Nothing in steps 3 to 5 compiles or passes before the regeneration of step 2, and the Go and Rust steps need the schema copy of step 2.

| Step | Consumer | What it does with the change |
|---|---|---|
| 1 | Hosts, `engine/fps-hosts` (HST-04) | Obtains and renews addresses through DHCP for a pool whose slice has `dhcp4`, one client per host, with the MAC address `mac_base` plus i (plan rule 5). `HostPool::parse` (`pool.rs`) needs a prefix today, so the DHCP slice needs its own constructor or an address source that the lease fills. `slice.rs` and `neighbor.rs` (which answers ARP and NDP from the pool table) read addresses from the pool, so they learn each address from the lease. Server addresses still come from the prefix of the server pool |
| 2 | Coordinator, regeneration | Copies `contracts/schemas/test-config.schema.json` byte for byte to `controller/internal/store/schema/test-config.schema.json` (`test_controller_copies_are_byte_identical` in `tools/contracts/test_contracts.py` checks it), and runs the workflow `regen-agentpb` after this contract merges, then pushes both to the pull request |
| 2 | Controller, agent protocol client (`controller/internal/agents/agentpb`) | Regenerated from the proto file with `regen-agentpb`. `HostPoolSlice` gains `Dhcp4 *Dhcp4`, and the new type `Dhcp4` |
| 3 | Controller, compiler (`controller/internal/compiler/config.go`, `compile.go`, `plan.go`) | `config.go`: `HostPool` gains `DHCP4 *struct{} \`yaml:"dhcp4" json:"dhcp4"\`` (the decode is strict with `KnownFields(true)`, so every example with `dhcp4` fails to parse until it does; `dhcp4: {}` decodes to a non nil pointer, and the schema forbids `null`). `compile.go`, `poolSlice`: with `DHCP4` set, require role `client` and no `ipv4`, `ipv6`, and `gateway`, skip the address and usable host checks and the "a host pool needs ipv4 or ipv6" case (also accept `DHCP4` in that case), keep the `count` bounds and the `mac_base` rule, and copy the declaration into the slice. `plan.go`: `HostPoolSlice` gains `Dhcp4 *struct{} \`json:"dhcp4,omitempty"\``, which writes `"dhcp4":{}` for a non nil pointer, as the canonical form of `check_plans.py` requires. Add a golden plan for `dhcp_clients.yaml` and the three error cases. The pool splitting across agents is unchanged, because it uses `count` and `mac_base`. Sending a DHCP slice to an agent of an `engine_api` before 0.2.6 is the controller's to refuse or to allow, and this package leaves it open |
| 3 | Controller, store (`controller/internal/store/validate.go`) | No code change expected: `references` and `scenarioRefs` read pool names and roles. The embedded schema of step 2 and the compiler of this step decide. The fixture `testdata/invalid/pool_without_address.yaml` (want `/network/host_pools/1/ipv4`) still passes, because the new `anyOf` branch adds findings and keeps the one at `ipv4`. Add fixtures for `dhcp4` with `ipv4` (want `/network/host_pools/0/ipv4`), `dhcp4` on a server pool (want `/network/host_pools/1/role`), and `dhcp4` with a property |
| 4 | Agent, `engine/fps-agent` | `src/plan.rs`: `HostPoolSlice` (which has `deny_unknown_fields`, so a standalone plan file with `dhcp4` is refused until then) gains `dhcp4: Option<Dhcp4>` with an empty struct `Dhcp4`. `src/convert.rs`: both directions (about lines 173 and 527; the second builds `pb::HostPoolSlice` without `..Default::default()`, so **the whole crate stops compiling** when the proto adds field 13 and `build.rs` regenerates `pb`). `src/assemble.rs`, `pool_of`: a DHCP slice has no prefix, so build the DHCP pool and fail Prepare with an error that names `dhcp4` when the backend cannot lease. `src/portfilter.rs`, `port_nets`: a DHCP pool contributes no prefix to the port filter, so the capture filter of a DHCP client port must not drop the leased addresses. `src/capabilities.rs`: `ENGINE_API` 0.2.6, with `tests/detect.rs` (line 44 asserts 0.2.5) and `tests/contract.rs`, which compares `ENGINE_API` with the proto header |
| 5 | UI (`ui/src/config/sections.tsx`, `ui/src/api/schema.d.ts`) | The schema import of `ui/src/config/schema.ts` reads the contract file, so the browser check follows at once. The host pool form needs a control for `dhcp4` (a checkbox that writes `{}` and removes `ipv4`, `ipv6`, and `gateway`, or hides them), and its error text for the `anyOf` reads "needs at least one of ipv4, ipv6, dhcp4" (`schema.ts` builds it from the branches). `npm run gen:api` regenerates `schema.d.ts` with `dhcp4?: Record<string, never>` on `host_pool`. The `if` keyword of ajv reports "must match "then" schema" for a pool that has `dhcp4` and a static address, which the form should word as the rule above |
| 6 | Realism, `realism/` | No change. No label or check there reads a host pool address. `capture/` profiles take addresses from the capture, not from the configuration |
| | SDK, CLI, REST API | No change. The REST API takes the configuration as a body, and the SDK and CLI do not read host pools |

### Tests that fail on the pull request until the consumers land

Found by reading the tests, since neither Go nor the UI nor Cargo can run in the sandbox of this package.

| Test | Why it fails | Fix |
|---|---|---|
| `tools/contracts/test_contracts.py::test_controller_copies_are_byte_identical[...test-config.schema.json...]` (runs here) | The embedded schema copy is still v0.3 | Step 2, copy the file |
| `controller/internal/store`: `TestSchemaAcceptsEveryExample`, `TestValidateEveryExample`, `TestEveryExampleStoresAndValidates` | They load every file of `contracts/examples/tests/`, and `dhcp_clients.yaml` has `dhcp4`, which the embedded schema (`additionalProperties: false`) and the strict decode of `compiler.ParseConfig` refuse | Steps 2 and 3 |
| `controller/internal/compiler`, `wiring`, `cmd/fps-controller` tests | No failure expected: they name their examples (`http_smoke`, `enterprise_mix_10g`, `dns_zone`, `http1_first_party`, ...) and none lists the directory. They would fail on `dhcp_clients.yaml` if a later test did | Step 3 adds the golden plan |
| `engine/fps-agent`: every test and the crate build | `convert.rs` builds `pb::HostPoolSlice` without `..Default::default()`, and `pb` is generated from the proto file at build time, so the build fails with a missing field `dhcp4` (E0063). `tests/contract.rs::engine_api_matches_the_proto_contract_version` also fails until `ENGINE_API` is 0.2.6 | Step 4 |
| `ui/src/config/schema.test.ts`, "asks for an address when a host pool has neither IPv4 nor IPv6, once" | Expects `needs at least one of ipv4, ipv6`, and the message now ends with `, dhcp4` | Step 5, change the expectation |
| `ui/src/pages/ConfigEditor.test.tsx`, "lists the stored examples, newest first, and opens one" | Expects 8 rows (header and 7 examples), and the directory now has 8 examples | Step 5, expect 9 rows. `seedless_smoke` stays first, because the mock sorts by file name and `dhcp_clients` sorts before `dns_zone` |
| `ui/src/config/roundtrip.test.ts` (per example) | No failure expected: the schema comes from the contract file, and `dhcp_clients.yaml` passes it. CI confirms | |
| `engine/fps-hosts/tests/contract_examples.rs`, `every_example_test_config_pool_parses_and_maps_unique_addresses` | Calls `HostPool::parse` on every pool of every example, and the pool `clients` of `dhcp_clients.yaml` has no prefix. Found by CI on 2f537e8 | The test skips a pool with `dhcp4`, whose addresses come from leases (HST-04) |

CI of the first commit (8cb7802) confirmed the list and added four more, all consumers: `controller/internal/agents/agentpb/gen` `TestGeneratedCodeIsCurrent` (step 2, regenerate), `controller/internal/store` `TestEmbeddedSchemaMatchesContract` (step 2, copy the schema), the `lint-ui` step of the `ui` workflow (`src/api/schema.d.ts is out of date`, so `npm run gen:api`, which runs before the UI tests, so the two UI tests above are unconfirmed), and the Rust jobs that build the agent (`cargo clippy --workspace`, the `cap` replay job, `controller-e2e`, `standalone-netns`, `qa-gate1`), which fail on the missing field in `convert.rs`. The `contracts` job fails only on the schema copy test (1 failed, 509 passed). Every other workflow, including all `mod-*`, passed.

The coordinator and a consumer worker landed steps 2 to 5 in this pull request (0c9e843 and 2f537e8). Step 1 is HST-04, and until it lands the agent refuses a DHCP pool at Prepare with an error that names `dhcp4`.

The jobs that read `ENGINE_API` and the proto version are the same ones that CON-32 named.

## Verification

`python3 tools/contracts/validate.py` passes, `python3 tools/contracts/check_plans.py` passes, `python3 -m pytest tools/contracts` passes except the one test of the embedded schema copy above (371 passed, 1 skipped, 1 failed), the proto file compiles with `grpc_tools.protoc`, and `python3 tools/plan/plan.py check` passes. The pull request CI checks `buf` and the workspace tests.
