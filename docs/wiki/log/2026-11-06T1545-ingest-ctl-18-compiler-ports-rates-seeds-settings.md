## [2026-11-06] ingest | CTL-18 compiler port pinning, host share rates, seeds, and module settings

The test compiler now applies decision D12 items 7, 8, 9, and 22. A port location `<agent>/<interface>` pins the port and its host pools to that agent, and a bare or `*/` interface spreads them across the agents that have it. A mix entry gets rates only on workers that own hosts of its client pool, so a pool with fewer hosts than workers leaves some workers without rates while the total stays as configured. A configuration without a seed gets a random nonzero seed that the compiler draws once per run, and a seed of 0 is rejected. `modules.<name>.settings` validates against the settings schema of the manifest and the plan carries the settings in the module entry. Golden plans cover a pinned port, a pool smaller than the worker count, and a module setting. We documented the rules in [Controller](../Controller.md#test-compiler).

### Shared page notes

- [Contracts](../Contracts.md): `fps.agent.v1.ModuleRef` has no field for module settings, so the compiler writes `settings` ahead of the contract and an agent that parses plans with the current proto rejects a plan with settings. A contract change package needs to add `ModuleRef.settings` as a `google.protobuf.Struct`, and AGT needs to read it.
- [Contracts](../Contracts.md): after CTL-18, `contracts/examples/tests-pending/port_locations.yaml` and `dns_zone.yaml` compile. `filtered_criteria.yaml` still waits for CTL-20.
