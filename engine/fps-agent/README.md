# fps-agent

The FixingPointSystem agent. The crate holds host capability detection, the standalone mode that runs a plan without a controller, and the controller session that dials the controller and serves `prepare`, `start`, and `stop`. The wiki page [Agent](../../docs/wiki/Agent.md) describes the design.

## Requirements

The agent needs Linux 6.3 or newer, because workers take their source ports from a range that they set with the socket option `IP_LOCAL_PORT_RANGE` (decision D12 item 3). `fps-agent connect` and `fps-agent run` read the kernel release before they do anything else and exit with code 2 on an older kernel. The error names `IP_LOCAL_PORT_RANGE`. `fps-agent capabilities` still runs on any kernel.

## Commands

```bash
fps-agent capabilities [--root DIR]
fps-agent run --plan FILE --modules DIR [--side both|client|server] [--linger SECONDS]
fps-agent connect --controller URI --agent-id ID --modules DIR [--root DIR] \
  [--side both|client|server] [--label KEY=VALUE]... [--redial-limit SECONDS]
```

## Redialing the controller

After a lost connection or an abort, `connect` dials again. The wait starts at 200 ms, doubles up to 10 s, carries jitter, and restarts at 200 ms after an abort or after a session that opened (decision D12 item 11).

| Flag | Default | Meaning |
|---|---|---|
| `--redial-limit SECONDS` | 300 | How long failed attempts may last before the agent aborts any run, releases the plan, and exits with code 3. The value 0 redials forever |

| Exit code of `connect` | Meaning |
|---|---|
| 0 | SIGINT or SIGTERM stopped the agent |
| 2 | A usage error, a kernel before Linux 6.3, or a failed capability detection |
| 3 | The agent gave up redialing the controller |

## Tests

```bash
make test-agt          # every test of the crate
make test-agt-redial   # the kernel check, the redial policy, and the exit codes
```
