## [2026-11-28] ingest | CON-36 The DHCP example names a port for each arm

CON-36 (issue #721, bug #708) changes `contracts/examples/tests/dhcp_clients.yaml`: `client0` is `local/eth1`, `server0` is `local/eth2`, the `servers` pool has the gateway `48.0.0.254`, and the header comment says what the network must provide (a DHCP server on the segment of `client0`, a route from the leased subnet to the servers) and that the file needs a user space backend. The vetting agent approved it under D13. No schema, proto, OpenAPI, or version moves. `tools/contracts/test_contracts.py` pins the ports, the gateway, and the header, `contracts/README.md` names the change, and `contracts/changes/CON-36.md` lists the consumers with AGT-23 first.

Decisions.

- The gateway `48.0.0.254` is the last address of the pool prefix, so it cannot collide with the one server, which takes the first address.
- The `EXPECTED` row of `dhcp_clients.yaml` in `engine/fps-agent/tests/fallback.rs` should hold as `Refused`: `pool_of` refuses a DHCP slice before any backend or interface check, so both selections still end on the `dhcp4` cause. AGT-23 changes the row.

### Shared page notes

- Controller golden (outside CON, the coordinator applies it in this pull request): `controller/internal/compiler/compile_test.go`, case `dhcp_clients`, `Interfaces: []string{"lo"}` becomes `Interfaces: []string{"eth1", "eth2"}`. In `testdata/dhcp_clients/local.json`, port `client0` `"interfaceName": "lo"` becomes `"eth1"`, port `server0` `"interfaceName": "lo"` becomes `"eth2"`, and the `servers` host pool gets `"gateway": "48.0.0.254",` between `"count": 1,` and `"macBase": "02:00:01:00:00:00"`.
