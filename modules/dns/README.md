# dns module

The `dns` module generates DNS lookups between a stub resolver client and an authoritative server. Its native crate `fps-module-dns` implements the template functions `dns.query`, `dns.answer_ip`, `dns.response`, and `dns.tcp_length` in Rust under decision D10, so scenarios render every message without a WebAssembly runtime. The server answers from the zone file `zones/example.test.zone` unless the setting `zone_file` names another. Every name lies under the reserved `.test` domain, and every address lies in the documentation ranges 192.0.2.0/24 and 2001:db8::/32.

## Layout

The module follows the layout of the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules).

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, parameter and settings schemas, the function catalog, scenarios, the fixture, and provenance |
| `scenarios/*.yaml` | Seven flow programs, listed below |
| `zones/example.test.zone` | The synthetic zone that the server answers from |
| `native/` | The crate `fps-module-dns` with the RFC 1035 codec, the zone file parser, the module settings, the `TemplateFn` implementation `Dns`, and the catalog `FUNCTIONS` |
| `native/examples/fixture_peer.rs` | Runs one side of every scenario over kernel sockets for the fixture capture |
| `tests/` | Catalog, interpreter, and golden transcript tests, with transcripts in `tests/testdata/` |
| `fixtures/` | The synthetic capture `dns.pcapng`, its labels `expected.yaml`, its realism report, and `capture.sh`, which records it |

## Template functions

The functions extend the catalog of the contract example `contracts/examples/modules/dns` by appending optional parameters and the function `tcp_length`, so every example call keeps its meaning. Text arguments are checked before any byte is written, and a bad name, type, address, or size fails the session with the reason `extension_invalid_argument`. Bytes from the peer that do not parse as a DNS message fail it with `extension_protocol`.

| Function | Parameters with defaults | Result |
|---|---|---|
| `query` | `name`, `qtype='A'`, `tcp=false` | A query with one question of class IN, the RD flag, and a transaction id from the session generator. `qtype` takes `A`, `AAAA`, `CNAME`, `MX`, `TXT`, `NS`, or `SOA`. With `tcp` the message carries the two byte length prefix of RFC 1035 section 4.2.2. |
| `answer_ip` | `message` | The first A or AAAA address of the answer section as text, for a `set` action |
| `response` | `query`, `address=''`, `ttl=300`, `tcp=false`, `udp_size=512` | The response to `query`, with the transaction id, the question, and the RD flag of the query. An empty `address` answers from the zone. A set `address` answers with that one address and `ttl`, as in the contract example. A UDP response longer than `udp_size` keeps only the question and sets TC. With `tcp` the response carries the length prefix and never truncates. |
| `tcp_length` | `prefix` | The length that a two byte TCP prefix announces, as text, so a side reads the message with `expect bytes` |

The server answers as an authoritative server of its zone. It follows CNAME chains inside the zone for at most eight links, answers a missing name with NXDOMAIN and a missing type with an empty NOERROR answer, both with the SOA record in the authority section, and refuses names outside the zone. MX and NS answers carry the addresses of their targets in the additional section. Answers compress names as RFC 1035 section 4.1.4 describes. A query with another opcode gets NOTIMP, a query without exactly one question gets FORMERR, and a query of another class gets REFUSED.

## Zone files

`Zone::parse` reads the subset of the master file format of RFC 1035 section 5 that synthetic zones need. It knows `$ORIGIN` and `$TTL`, the owner `@`, relative and absolute names, a blank owner that repeats the previous one, TTL and class in either order, comments, quoted strings, and parentheses that continue a record over several lines. Records take the types A, AAAA, CNAME, MX, TXT, NS, and SOA in class IN. Errors name the line. `Dns::bundled()` serves the shipped zone, and `Dns::new(zone)` serves any other.

The parser rejects what it does not know instead of skipping it. That covers the directives `$INCLUDE` and `$GENERATE`, the types and classes outside the list above, a record without a TTL when no `$TTL` is set, an address that does not parse, a second SOA record, and a CNAME next to other data at one name. A zone file that the `zone_file` setting names must also hold exactly one SOA record, because its owner is the zone origin and the server refuses names outside it.

## Settings

A test configuration sets the module settings once under `modules.dns.settings` (decision D12 item 22), and the manifest describes them with a JSON Schema.

| Setting | Type | Default | Meaning |
|---|---|---|---|
| `zone_file` | non empty string | none | Zone file that the server answers from, in the subset of master file syntax above. A relative path resolves against the module directory, so `zones/example.test.zone` names the shipped zone. Without the setting, the server answers from the shipped zone, as before. |

`Dns::from_settings(&value, base)` reads the settings value that the plan carries, with the module directory as `base`. It rejects a value that is not an object, any key other than `zone_file`, a `zone_file` that is not a non empty string, a file that cannot be read, a malformed zone, and a zone without an SOA record. Each error names the file, and a parse error also names the line, as in `zone file zones/x.zone: zone line 3: ...`. The test configuration `contracts/examples/tests-pending/dns_zone.yaml` shows the setting in use. The controller compiler validates and carries the settings, but the agent protocol has no field for them yet, so a plan does not deliver them to an agent until a contract change adds one.

## Scenarios

Each UDP flow sends one query datagram and reads one response datagram with `max_datagram: 512`. The TCP flow reads the two byte prefix with `expect bytes: 2`, binds the length with `dns.tcp_length(last)`, and reads the message with `expect bytes`.

| Scenario | Transport | Exchange |
|---|---|---|
| `lookup_a` | UDP | A for `www.example.test`, and the client binds `server_ip` |
| `lookup_aaaa` | UDP | AAAA for `www.example.test`, and the client binds `server_ip` |
| `lookup_cname` | UDP | A for `static.example.test`, answered with two CNAME records and the address |
| `lookup_mx` | UDP | MX for the zone apex, with two exchanges and three glue addresses |
| `lookup_txt` | UDP | TXT for the SPF record of the zone apex |
| `nxdomain` | UDP | A for a missing name, answered with NXDOMAIN and the SOA record |
| `tcp_fallback` | UDP, then TCP | TXT for `big.example.test`, whose 800 byte answer truncates over UDP, then the same question over TCP |

The parameter schema in the manifest names the variables `host` and `server_port`. The variable `qtype` of the lookup scenarios is fixed per scenario.

## Tests

We run the checks with the root `Makefile`.

```bash
make lint-mod-dns     # validate.py on the manifest and every scenario
make test-mod-dns     # cargo test -p fps-module-dns, then the realism score of the fixture
sudo make e2e-mod-dns # record and score a fresh capture in the namespace harness
```

The unit tests cover the codec against the contract example bytes, compression, every record type, malformed input at every truncation length, and 20,000 random inputs that must never panic the parser. They also cover the zone parser, its error lines, the lookup rules, and every function. The catalog test compares `FUNCTIONS` with the `functions` list of the manifest in every name, parameter, type, and default. The scenario tests run every flow of every scenario with both sides over the in memory datagram pair or duplex for four session ids and parse every message, so they check the transaction id, the question, the response code, the TC flag, the answer types, the section sizes, and the bound `server_ip`. The transcript test renders every flow with seed 42 and compares both directions with `tests/testdata/<scenario>.<flow>.seed42.c2s.bin` and `.s2c.bin`. A UDP transcript prefixes each datagram with its length, so the file keeps the datagram boundaries. `UPDATE_GOLDEN=1` rewrites them.

## Fixture and realism

`fixtures/capture.sh` brings up the QA-01 harness under its own name prefix, runs the server side of every scenario in the server namespace and the client side in the client namespace through `fixture_peer`, records the router's client arm, and scores the capture with the realism runner. TCP flows run over the kernel TCP backend. UDP flows run over Tokio UDP sockets until the kernel UDP backend of TRN-02 merges. The client opens one socket per session from an ephemeral port, and the server binds port 53 once. It runs 20 sessions per scenario, so the capture holds 160 sessions, 140 over UDP and 20 over TCP, in about 75 KB. Both peers receive the module settings with `--settings`, so the server answers from a configured zone. `capture.sh` passes `{"zone_file":"zones/example.test.zone"}` by default, and the environment variable `FPS_DNS_SETTINGS` replaces that JSON. `fixture_peer --zone FILE` is a shorthand that resolves FILE against the current directory.

The committed capture scores as follows. nDPI 4.2.0 classifies 160 of 160 sessions as DNS from payload alone, so the conformance ratio is 1.0 against the threshold 0.99, and tshark 4.2.2 reports no parser errors and no malformed packets. Zeek was not installed when we recorded it, so the report covers tshark only. The `mod-dns` workflow scores the committed capture and also records and scores a fresh one on every change.
