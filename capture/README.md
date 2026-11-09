# Capture compiler

The `capture` package turns packet captures into data that the scenario generator, the scrubber, and the profile extractor read (work packages CAP-02 to CAP-04). CAP-01 adds the first command, `sessions`. The package uses only the Python standard library, version 3.10 or newer.

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

## Files

| File | Purpose |
|---|---|
| `pcapio.py` | pcap and pcapng reader that yields frames with nanosecond timestamps |
| `decode.py` | Ethernet (with VLAN), Linux cooked, null, raw IP, IPv4, IPv6 with extension headers, TCP, and UDP |
| `tcp.py` | Reassembly of one TCP direction |
| `sessions.py` | Session grouping, handshake, anomalies, turns, and the JSON document |
| `cli.py`, `__main__.py` | The command line |
| `sessions.schema.json` | JSON Schema of the output |
| `tests/` | Tests with synthetic captures that `tests/pcapgen.py` writes as bytes |

```bash
python3 -m pip install -r capture/requirements-dev.txt
make lint-cap test-cap
```
