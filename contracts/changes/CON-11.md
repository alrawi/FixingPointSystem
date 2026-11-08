# CON-11 Module settings in the agent protocol

Contract. Agent control protocol in `proto/fps/agent/v1/agent.proto`.

Versions. The protocol moves from 0.2.0 to 0.2.1. The header comment of the proto file and the comment in `buf.yaml` name the new version, and `contracts/README.md` lists it. The change adds one field and no command, so it takes a patch number and not a minor number. The controller admits agents by the major and minor part of `Capabilities.engine_api` (`DefaultEngineAPIs` lists `0.1` and `0.2`), and a bump to 0.3.0 would make the controller refuse every agent until that list changes. The check of `tools/contracts/check_plans.py` also compares major and minor. `Capabilities.engine_api` is still how an agent announces what it implements: the Rust agent now reports 0.2.1.

Changes.

- `ModuleRef` gains `settings` (6, `google.protobuf.Struct`). It holds `modules.<name>.settings` of the test configuration, which the controller compiler validates against the settings schema of the module manifest (CON-09, CTL-18). The field is unset when the test gives the module no settings.
- The settings are not part of the module tree, so no module digest changes.
- The canonical JSON form carries the field as `modules[].settings`, an object, and leaves it out when unset. `examples/plans/http_dns_smoke.json` stays valid and canonical.
- The examples `port_locations.yaml`, `dns_zone.yaml`, and `filtered_criteria.yaml` move from `examples/tests-pending/` to `examples/tests/`, because CTL-18, CTL-20, and CTL-22 now compile and validate them. `examples/tests-pending/` is empty and goes away.

Compatibility. The change is additive on the wire and in the proto file.

- Field number 6 was unused and the file declares no `reserved` range, so nothing is removed or renumbered. `buf breaking` with the `FILE` rules reports no finding against `main`, as in CON-05.
- Wire level: an agent of 0.2.0 that reads a 0.2.1 `ModuleRef` skips field 6, and a 0.2.1 agent that reads a 0.2.0 `ModuleRef` sees no settings. Settings that an old agent skips are silently not applied, so the controller sends a plan with module settings only to an agent whose `engine_api` is 0.2.1 or later.
- JSON mapping: the plan parser of the standalone agent mode rejects unknown fields, so a plan file with `modules[].settings` needed this change to be accepted. A 0.2.0 agent still rejects such a file, which is the safe behaviour.
- Source level: code that builds `ModuleRef` with every field listed, and code that mirrors the proto by hand, must add the field. In this repository these are `engine/fps-agent/src/proto.rs`, `src/plan.rs`, `src/convert.rs`, and `tests/contract.rs`. The generated Go package `controller/internal/agents/agentpb` needs regeneration. `controller/internal/compiler/plan.go` already writes `modules[].settings`, and `protojson` in `controller/internal/runs` accepts the field once the package is regenerated.
- Not specified here: applying the settings in the agent, which AGT-07 does. Until then the agent carries the settings in its plan and ignores them.
