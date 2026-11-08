## [2026-11-08] ingest | CTL-25 scenario contract v0.3 in the controller registry

The module registry now validates scenario files, which it did not do before: it read only the `scenario` name and left the rest to later stages. `internal/registry` embeds `contracts/schemas/scenario.schema.json` (v0.3) and checks every scenario that the manifest lists and every `scenarios-pending/*.yaml` file against it. A small Go parser for the template grammar then applies the v0.3 rules of CON-10 to the action templates: `profiles.<name>` must name a profile that the scenario declares, and `dist.pareto`, `dist.lognormal`, `dist.exponential`, and `dist.uniform` follow the argument and literal range rules. A module whose scenario declares profiles and calls dist installs, and a scenario with an undeclared profile is rejected with a finding at `file:<path>#<JSON Pointer>`. See [Controller](../Controller.md#install).

### Shared page notes

- Testing: `controller/internal/registry/scenario_test.go` adds the schema copy equality test, installs of the example `web_profile.yaml` (pending and listed), and a table of rejected profile and dist cases. `make test-ctl-registry` and `make lint-ctl-registry` run the package alone.
- Contracts: nothing changed. `go generate ./internal/registry` now refreshes the scenario schema copy as well. The CON-10 note says the controller compiler does not know the syntax, and this package did not change `internal/compiler`, so a plan that carries such a scenario still needs a follow-up there.
- Implementation Plan: CTL-25 of batch 12 is done once merged.
