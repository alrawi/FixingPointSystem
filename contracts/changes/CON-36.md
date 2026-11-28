# CON-36 The DHCP example names a port for each arm

Contract example. `contracts/examples/tests/dhcp_clients.yaml`. The change is issue #721, for bug #708. The vetting agent specified and approved it under decision D13 on 2026-11-28 (`docs/wiki/log/2026-11-28T0610-decision-bug-708-dhcp-example-and-trn-31.md`). It changes no schema, proto file, or OpenAPI document, and no version moves.

Problem. The example put both ports on `local/lo`. No DHCP server answers on loopback, so the kernel backend leases nothing (HST-04), and the datapath needs an interface for each arm, so no backend could run the file as written.

## The change

| Item | Before | After |
|---|---|---|
| `client0` location | `local/lo` | `local/eth1` |
| `server0` location | `local/lo` | `local/eth2` |
| `servers` pool `gateway` | none | `48.0.0.254`, inside the pool prefix `48.0.0.0/24` |
| Header comment | what the pools name | adds what the network must provide and that the file needs a user space backend |

The network must provide a DHCP server on the segment of `client0`, and a route from the leased subnet to the servers. The `clients` pool is unchanged: it names no `ipv4`, `ipv6`, or `gateway`.

## Compatibility

The configuration still validates against test configuration v0.4. Nothing in a schema, a message, or a version changes, so `buf breaking` and `oasdiff` have nothing to compare. Only a reader that pins the old locations of this file notices: the compiler golden case below.

## Consumers

The order is the order of the work.

| Step | Consumer | What it does with the change |
|---|---|---|
| 1 | Agent, `engine/fps-agent` (AGT-23) | First, because the package waits for this file. It runs the file through an agent forced onto AF_XDP in the namespace harness, with the DHCP server in the router namespace, and then changes the `dhcp_clients.yaml` row of `EXPECTED` in `tests/fallback.rs`. The row stays `Refused` until AGT-23 lands, because the refusal names `dhcp4` and not the interfaces |
| 2 | Controller compiler golden (coordinator, same pull request) | `compile_test.go` lists the agent interfaces of the case `dhcp_clients` as `eth1` and `eth2`, because the compiler refuses a pinned port on an agent that lacks the interface, and `testdata/dhcp_clients/local.json` names the new interfaces and the gateway of the `servers` pool |
| 3 | TRN-37 and TRN-31 | TRN-37 gives the datapath its DHCP leases. TRN-31 needs the file to run on an unforced agent, which it now can on AF_XDP |
| | Docs | `contracts/README.md` names the change. `docs/wiki/Contracts.md` lists the file and needs no edit |

## Verification

`python3 tools/contracts/validate.py`, `python3 -m pytest -q tools/contracts`, and `python3 tools/plan/plan.py check`, `render --check`, and `lint` pass. The two new tests in `tools/contracts/test_contracts.py` check the ports, the gateway, and the header. The controller golden test fails on this branch until the coordinator applies the golden edits, and CI checks `cargo test -p fps-agent --test fallback`.
