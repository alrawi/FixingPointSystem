
# strike

Application-layer attack traffic for validating the defensive posture of a
device under test. The module fills the `strike` slot that the specification
reserves for exploit and flood traffic, using only the verbs the v0.6 scenario
contract already defines, so it runs on kernel sockets today.

## Scope

The raw packet path and large spoofed source pools stay out of scope until the
engine grows them. Every scenario here is a pair of ordinary flows: an
attacker side and a server side that models a hardened origin, so the traffic
is replayable against a device under test placed between the namespaces.

| Scenario | Attack shape | Hardened origin answer |
| --- | --- | --- |
| `slowloris` | slow HTTP request read, header lines dribbled with delays | 200 after the blank line, long server-side tolerance |
| `conn_churn` | burst of connections that send nothing and reset | accept then reset, no application state |
| `bad_request_line` | request line with a method a parser cannot accept | 400 |
| `te_cl_conflict` | POST declaring both Content-Length and Transfer-Encoding | 400 |

## Layout

- `manifest.yaml` — module manifest, validated by `tools/contracts/validate.py`.
- `scenarios/` — scenario YAML flows.
- Fixtures, a fixture peer, and a QA harness job land in a follow-up once the
  scenario set settles, following `modules/dns` and `modules/http1`.

## Roadmap

1. Recorded fixture and realism scoring once the scenario set is stable.
2. A native fixture peer crate so `fixtures/capture.sh` can record in the
   QA-01 namespace harness, following `modules/dns`.
3. Raw-packet strikes (stateless SYN floods, spoofed pools) when the engine
   accepts the `raw` action, per the reserved-action rule in the contract.
