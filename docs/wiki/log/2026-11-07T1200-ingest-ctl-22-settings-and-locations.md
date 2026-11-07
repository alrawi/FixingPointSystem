## [2026-11-07] ingest | CTL-22 module settings and port locations in the controller

The catalog adapter in `internal/wiring` now fills `Scenario.SettingsSchema` from the `settings` key of the manifest, so the compiler checks `modules.<name>.settings` in the running controller and an installed dns module reports its schema. `internal/store/validate.go` splits port locations as the compiler does. A bare `eth1` or `*/eth1` spreads the port across the available agents that have the interface, `<agent>/<interface>` pins it, and the capacity check and the compiler dry run use those agents. The compiler path pattern covers `modules`, so a settings error names `/modules/<name>/settings`. Tests in both packages read the three examples in `contracts/examples/tests-pending/`. We documented the rules in [Controller](../Controller.md#validation).

### Shared page notes

- [Contracts](../Contracts.md): `port_locations.yaml`, `dns_zone.yaml`, and `filtered_criteria.yaml` now pass validation through the store and the REST API of the controller, so CON-11 can move them to `contracts/examples/tests/`. Its tests that read the directory `tests-pending` need the new path.
