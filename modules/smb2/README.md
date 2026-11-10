# smb2 module

The `smb2` module generates SMB2 traffic between a client and a server side on TCP port 445. Its native crate `fps-module-smb2` (MOD-12) implements thirty seven template functions in Rust under decision D10, so scenarios render every message without a WebAssembly runtime. The module has no state. This package (MOD-13) adds the manifest, three scenarios, a fixture, and the tests that run them. Every user name, password, domain, workstation, and host name is synthetic, the host names lie under the reserved `.test` domain, and every file body comes from the seeded `fill()` generator.

## Layout

The module follows the layout of the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules). The tests of the native crate live in `native/tests/`, where MOD-12 put them.

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, the parameter schema, the function catalog, the scenarios, the fixture, and provenance |
| `scenarios/*.yaml` | The three flow programs `browse`, `download`, and `upload` |
| `native/` | The crate `fps-module-smb2` with the `TemplateFn` implementation `Smb2` and the catalog `FUNCTIONS` |
| `native/examples/fixture_peer.rs` | Runs one side of every scenario over kernel sockets for the fixture capture |
| `native/examples/print_catalog.rs` | Prints the `functions` list that the manifest must hold |
| `native/tests/` | Message, NTLM, tshark, fuzz, catalog, flow, scenario, and fixture tests, with golden files in `testdata/` |
| `fixtures/` | The synthetic capture `smb2.pcapng`, its labels `expected.yaml`, its realism report `smb2.report.json`, and `capture.sh` |

## Scenarios

Each scenario has one flow whose client and server sides run against each other. Every session starts with the same exchange: NEGOTIATE, two SESSION_SETUP requests that carry a synthetic NTLMv2 authentication, and TREE_CONNECT. It ends with TREE_DISCONNECT and LOGOFF. The server draws the session id and the tree id from its session generator, and every file id comes from the CREATE response. Both sides read each message with two `expect` actions, the 4 octet transport header of direct TCP and then the `smb2.message_length(last)` octets that it announces. The client numbers its messages and offsets with `smb2.add`, and keeps the authenticate token in a variable as `smb2.hex(...)` and `smb2.unhex(vars.auth)`, because a `set` action keeps text.

| Scenario | Exchange |
|---|---|
| `browse` | Opens the root directory, lists it with two QUERY_DIRECTORY requests (the second gets STATUS_NO_MORE_FILES), closes it, thinks for a lognormal time, and lists the directory `subdir` the same way. The listings come from the variables `listing` and `subdir_listing`. |
| `download` | Opens the file `file_name`, reads `file_size` octets with READ requests of `read_size` octets, and closes it. The server answers each READ with as many octets as the request names. |
| `upload` | Creates the file `upload_name` with FILE_CREATE, writes `upload_size` octets with WRITE requests of `write_size` octets, and closes it. The server acknowledges each WRITE with its count. |

### File sizes

A flow has no arithmetic. The scenarios therefore take the length of the last READ or WRITE as a variable of its own, next to the size of the file. The rule is `tail_size = file_size mod read_size`, or `read_size` when `read_size` divides `file_size`, and `upload_tail` follows from `upload_size` and `write_size` the same way. The client counts the full requests with `smb2.chunks` and `smb2.add(..., -1)`, and then sends one request of the tail length. A file of 150000 octets in reads of 65536 octets has `tail_size` 18928, and a file of 1000 octets has `tail_size` 1000. The defaults (1 MiB in 64 KiB requests) satisfy the rule, and the test `the_default_tails_follow_from_the_default_sizes` checks it. The tests run sizes that are not a multiple, a file that fits one request, and read sizes of 64 KiB and 1 MiB.

A message with a call nested in another call gives octets, and `smb2.add` takes an integer, so a count such as `smb2.add(smb2.chunks(...), -1)` goes through two `set` actions. The values of a `set` action are strings, so a counter starts as `"0"`.

### Dialects

`dialects` is the comma separated list that the client offers and `dialect` is what the server selects. The defaults offer `2.0.2,2.1` and select `2.1`. A list with `3.1.1` adds the preauthentication and encryption negotiate contexts. The scenarios run with 3.1.1 in the tests. The module writes no compound requests, create contexts, leases, or SMB 3.x signing and encryption, so no session is signed and none is encrypted.

## Template function readers

`smb2.header_field(last, name)` and `smb2.body_field(last, name)` read the message `last`. The header fields are `command`, `command_code`, `status`, `status_code`, `message_id`, `session_id`, `tree_id`, `credits`, `credit_charge`, `flags`, `response`, `signed`, `async`, and `next_command`. The body fields depend on the command.

| Command | Body fields |
|---|---|
| `negotiate` | `dialects` or `dialect`, `security_mode`, `capabilities`, `guid`, `max_read`, `max_write` |
| `session_setup` | `session_flags`, `previous_session_id`, `binding` |
| `tree_connect` | `path`, `share_type`, `maximal_access` |
| `create` | `name`, `file_id`, `size`, `attributes`, `action`, `desired_access`, `disposition`, `options` |
| `read`, `write`, `close` | `file_id`, `offset`, `length`, `count`, `remaining`, `flags` |
| `query_directory` | `pattern`, `flags`, `output_length`, `entries`, `entry_count` |

## Tests

We run the checks with the root `Makefile`.

```bash
make lint-mod-smb2 # validate.py on the manifest and every scenario
make test-mod-smb2 # cargo test -p fps-module-smb2, then the realism score of the fixture
```

The scenario tests in `native/tests/scenarios.rs` run both sides of every scenario over the in memory duplex and parse both directions with the readers of the module. They check the commands in order, the pairing of requests and responses, the message ids that count up without a gap, the session id and tree id that the server assigned, the NTLM user, domain, and workstation, the listings, the sizes of every READ and WRITE for several file sizes, SMB 3.1.1, the distinct ids of six sessions, and the parameter schema of the manifest against the variables of the scenarios. Golden transcripts of the three scenarios with seed 42 and session id 1 live in `native/tests/testdata/<scenario>.seed42.{c2s,s2c}.bin`, with files of 10000 octets in reads of 4096. `UPDATE_GOLDEN=1` rewrites them. The catalog test compares `FUNCTIONS` with the `functions` list of the manifest in every name, parameter, type, and default.

## Fixture and realism

`fixtures/capture.sh` brings up the QA-01 harness under its own name prefix, runs the server side of every scenario in the server namespace and the client side in the client namespace through `fixture_peer` and the kernel TCP backend, records the router's client arm, and scores the capture with the realism runner. It runs six sessions per scenario with a file of 100000 octets to download and one of 90000 octets to upload, so the capture holds 18 sessions in about 1.3 MB. It needs root, `ip`, `tcpdump`, `ss`, `ndpiReader`, and `tshark`.

The client peer gives every session its own source port, from 40000 on in the order of the sessions (MOD-21), so every 5-tuple of the capture is unique, and `capture.sh` fails when the capture holds fewer sessions than the peers ran.

**nDPI and the transport header.** Every message starts with the 4 octet header of direct TCP: a zero octet and a 3 octet length. That is also the NetBIOS session header of a session message, so the header needs no change for nDPI: the 18 sessions of the capture need no other framing to be recognized as SMB on port 445. nDPI 4.2.0 names the protocol `NetBIOS.SMBv23`, and the realism runner counts each part of the dotted name, so `expected.yaml` accepts `SMBv23` and `SMB`. The classification depends on the port. A session of the same scenarios on port 4450 is `Unknown` in nDPI 4.2.0, so the scenarios keep port 445 and the fixture never moves them.

The committed capture scores as follows.

| Check | Result |
|---|---|
| nDPI 4.2.0 conformance | 18 of 18 sessions are SMB (ratio 1.0), reported as `NetBIOS.SMBv23` |
| tshark 4.2.2 parser errors | none, and no malformed packets |

The `realism` workflow of CI records a fresh capture of the module with nDPI 4.14.0, tshark 4.2.2, and Zeek 9.0.1, and its first run on this package gave the same result: 18 of 18 sessions conform, nDPI 4.14.0 names them `NetBIOS.SMBv23` as 4.2.0 does, and neither tshark nor Zeek reports a parser error or a malformed packet. Zeek was not installed when we recorded the committed capture, so its report covers tshark only.
