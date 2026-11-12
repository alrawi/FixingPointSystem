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

The module follows the layout of the Modules section of the
[Coding Standards](../../docs/wiki/Coding-Standards.md#modules).

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, provides, scenarios, fixtures, and provenance |
| `scenarios/*.yaml` | The four flow programs listed above |
| `native/` | The crate `fps-module-strike`; the scenarios are pure contract programs, so the crate holds no native template functions |
| `native/examples/fixture_peer.rs` | Runs one side of the selected scenarios as scenario instances over kernel sockets for the fixture capture |
| `fixtures/` | The capture `strike.pcapng`, its labels `expected.yaml`, its realism report, and `capture.sh`, which records it |

## Fixture

`fixtures/capture.sh` records `strike.pcapng` inside the QA-01 namespace
harness (`tests/harness/netns.sh`): the server side of the fixture peer runs
in the server namespace, the client side in the client namespace, and the
router's client leg records between them. The capture has two phases. The
slowloris phase runs `slowloris` on its default port 8080. The churn phase
runs `conn_churn` with `--var server_port=8081 --var churn_port=8082`, so the
structure check of the realism runner scores each scenario against a session
pool of its own, and a listen port is any variable whose name ends in
`_port`. `fixtures/expected.yaml` holds the labels, and the script ends with
the realism score, which must pass.

The malformed scenarios `bad_request_line` and `te_cl_conflict` are not in
the scored fixture yet: their origin replies are protocol errors, and their
expected-findings package lands with the RLM work.

## Roadmap

1. The malformed scenarios in the scored fixture once the RLM
   expected-findings package defines how origin errors are labelled.
2. Raw-packet strikes (stateless SYN floods, spoofed pools) when the engine
   accepts the `raw` action, per the reserved-action rule in the contract.
