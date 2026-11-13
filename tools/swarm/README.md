# Swarm tools

These tools run the [Swarm Protocol](https://github.com/alrawi/FixingPointSystem/wiki/Swarm-Protocol) over the GitHub REST API.

| File | Purpose |
|---|---|
| `fps_swarm.py` | The helper command with the subcommands `now`, `list`, `claim`, `takeover`, `heartbeat`, `release`, `block`, `review`, `promote`, `stale`, and `sweep` |
| `fake_github.py` | An in memory fake of the REST endpoints, with a controllable server clock, for tests |
| `testdata/` | Recorded API responses that seed the fake |
| `test_fps_swarm.py` | Unit and component tests, which never reach the network |
| `bootstrap-labels.sh` | Creates the label set of the protocol |
| `project_sync.py` | Copies each status label into the Status field of the owner's project over GraphQL, run by `../../.github/workflows/project-sync.yml` with the secret `FPS_PROJECT_TOKEN`; see the wiki page Operations |
| `test_project_sync.py` | Tests of the project sync that replay GraphQL responses from `testdata/project_sync/` |
| `../../.github/workflows/swarm-scan.yml` | The scanner, which runs `fps_swarm.py sweep` every 30 minutes and on manual dispatch with a `dry_run` input |

We call every subcommand with the agent identifier in `FPS_AGENT_ID`.

```bash
export FPS_AGENT_ID="claude-$(openssl rand -hex 3)"
python3 tools/swarm/fps_swarm.py list
python3 tools/swarm/fps_swarm.py claim 23 --steps 5
python3 tools/swarm/fps_swarm.py heartbeat 23 --check
python3 -m pytest -q tools/swarm
```

All requests go through one runner function that shells out to `gh api -i`, so the command reads GitHub server time from the `Date` header of every response. Tests pass `FakeGitHub` as that runner. The command needs only the Python standard library.

`takeover` accepts an item with `status:blocked` as well as ready, stale, and in progress items. The promotion sweep writes a `blocked:` line to stderr for each blocked item that names no blocking issue, because it cannot unblock such an item.
