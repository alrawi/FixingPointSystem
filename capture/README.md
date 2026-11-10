# Capture compiler

The `capture` package turns packet captures into data that the scenario generator, the scrubber, and the profile extractor read (work packages CAP-02 to CAP-04). CAP-01 added the command `sessions`, CAP-02 added `scenario`, and CAP-03 added `scrub`. The compiler uses only the Python standard library, version 3.10 or newer.

```bash
python3 -m capture sessions --pcap run.pcapng --out sessions.json
python3 -m capture sessions --pcap run.pcap --payloads --udp-idle-timeout 30
```

| Option | Meaning |
|---|---|
| `--pcap FILE` | Classic pcap (microsecond or nanosecond, either byte order) or pcapng, including concatenated sections. Required. |
| `--out FILE` | Write the JSON there. The default is standard output. |
| `--udp-idle-timeout S` | A UDP flow that is idle for more than `S` seconds ends, and later packets of the same 5-tuple start a new session. Default 60. |
| `--payloads` | Add the payload bytes as base64 to every turn. Without it the output holds sizes and times only. |
| `--interface N` | Read only frames of interface id `N`. Ids count over the whole file. |

The exit code is 0 on success and 1 when the capture cannot be read or the output cannot be written. A capture that ends inside a record still yields the earlier packets and a warning.

The output follows [`sessions.schema.json`](sessions.schema.json). The wiki page [Capture](https://github.com/alrawi/FixingPointSystem/wiki/Capture) explains the rules for sessions, reassembly, anomalies, and turns.

## Scrubbing

`python3 -m capture scrub --sessions FILE --consent TEXT --out FILE` replaces addresses (with documentation ranges), user names, passwords, tokens, cookies, JWTs, API keys, and email addresses in the payloads and endpoints of a sessions document, with replacements of the same length and shape, and writes the `scrub` record that `scenario` turns into `source: capture`, `scrubbed: true` in the manifest. `--hostnames` adds host names, `--redact TEXT` and `--redact-file FILE` add texts that no rule can know, and `--drop-port PORT` removes sessions of protocols that the scrubber cannot read. Binary protocols are not parsed. The wiki page [Capture](https://github.com/alrawi/FixingPointSystem/wiki/Capture) lists every rule and its limits.

## Scenario generation

```bash
python3 -m capture sessions --pcap run.pcapng --payloads --out sessions.json
python3 -m capture scrub --sessions sessions.json --consent "owner approved" --out scrubbed.json
python3 -m capture scenario --sessions scrubbed.json --all --out-dir out/my_module
python3 tools/contracts/validate.py --modules out --no-openapi
```

The second command writes `manifest.yaml` and one scenario per session, with lengths, identifiers, timestamps and cookies replaced by template expressions. Without the record that `scrub` writes (or `--synthetic` for your own lab traffic) the manifest records an unscrubbed capture and fails validation on purpose. The wiki page [Capture](https://github.com/alrawi/FixingPointSystem/wiki/Capture) lists every option, heuristic and false positive guard.

## Files

| File | Purpose |
|---|---|
| `pcapio.py` | pcap and pcapng reader that yields frames with nanosecond timestamps |
| `decode.py` | Ethernet (with VLAN), Linux cooked, null, raw IP, IPv4, IPv6 with extension headers, TCP, and UDP |
| `tcp.py` | Reassembly of one TCP direction |
| `sessions.py` | Session grouping, handshake, anomalies, turns, and the JSON document |
| `templates.py` | Template text for literal bytes, and a reference renderer of the template functions |
| `fields.py` | Framing (HTTP, length prefixes) and detection of dynamic fields |
| `scenario.py` | Flows, expects, manifest, YAML, and the render check |
| `scrub.py` | Scrub rules, the sweep, and the scrub record |
| `cli.py`, `__main__.py` | The command line |
| `sessions.schema.json` | JSON Schema of the output |
| `tests/` | Tests with synthetic captures that `tests/pcapgen.py` writes as bytes |

```bash
python3 -m pip install -r capture/requirements-dev.txt
make lint-cap test-cap
```
