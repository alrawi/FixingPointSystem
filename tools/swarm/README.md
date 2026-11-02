# Swarm tools

These tools run the [Swarm Protocol](https://github.com/alrawi/FixingPointSystem/wiki/Swarm-Protocol) over the GitHub REST API.

| File | Purpose |
|---|---|
| `fps_swarm.py` | The helper command with the subcommands `now`, `list`, `claim`, `takeover`, `heartbeat`, `release`, `block`, `review`, `promote`, `stale`, and `sweep` |
| `fake_github.py` | An in memory fake of the REST endpoints, with a controllable server clock, for tests |
| `testdata/` | Recorded API responses that seed the fake |
| `test_fps_swarm.py` | Unit and component tests, which never reach the network |
| `bootstrap-labels.sh` | Creates the label set of the protocol |

We call every subcommand with the agent identifier in `FPS_AGENT_ID`.

```bash
export FPS_AGENT_ID="claude-$(openssl rand -hex 3)"
python3 tools/swarm/fps_swarm.py list
python3 tools/swarm/fps_swarm.py claim 23 --steps 5
python3 tools/swarm/fps_swarm.py heartbeat 23 --check
python3 -m pytest -q tools/swarm
```

All requests go through one runner function that shells out to `gh api -i`, so the command reads GitHub server time from the `Date` header of every response. Tests pass `FakeGitHub` as that runner. The command needs only the Python standard library.
