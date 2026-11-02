# Spike S4: TRex ASTF adapter

S4 asks whether TRex advanced stateful mode (ASTF) can serve as the tier T2 backend for our scenarios. It answers two questions. Can an ASTF profile express our scenario model without loss? Does ASTF perform real TLS key exchange?

## Translator

`fps_astf/translate.py` converts a test configuration and its scenarios into an ASTF Python profile for TRex v3.08.

- ASTF `recv` waits on a byte watermark and cannot match patterns. The translator therefore simulates both sides of each flow and learns how many bytes every `expect` consumes.
- Consecutive `expect` actions merge into one `recv(n, clear=True)`, which keeps loop iterations independent and avoids losing bytes that arrive past a watermark.
- Loops become `set_var`, `set_label`, and `jmp_nz`. Fixed and uniform delays become `delay` and `delay_rand`.
- Mix weights become per template connection rates. A byte basis divides each weight by the template's payload bytes per session.
- Templates that share a server port get disjoint server address ranges and matching association rules.
- The report classifies every action as exact, approximate, or unsupported.

## Usage

```bash
cd spikes/s4-trex-astf
python3 -m fps_astf.translate --test ../../contracts/examples/tests/http_smoke.yaml \
  --modules ../../contracts/examples/modules --out profile.py --report report.json
python3 -m pytest .   # uses a recording stand-in for the TRex API
```

To run a generated profile on TRex, start the server with `./t-rex-64 -i --astf` and load the profile with `start -f profile.py` in the TRex console. We have not yet run a generated profile on a TRex host.

## Results

See `RESULTS.md`.
