## [2026-11-12] ingest | RLM-16 Deliver the realism report to the controller

`python3 -m realism score` has an option `--attach-run ID` (`realism/deliver.py`) that PUTs the report it wrote to `PUT /v1/runs/ID/realism` (`attachRealismReport`) and reports the answer: the count of results and whether the report replaced one on 200, and the problem detail with every finding pointer on 409 and 422. The token is read from `--token-file`, `FPS_TOKEN_FILE`, or `FPS_TOKEN`, never from the command line. The evidence is `realism/tests/test_deliver.py` (a fake controller on loopback that records the request, with tests of the token sources and precedence, the redaction, the refused redirect, the refused plain http, the 409 and the 422). We described it in [Realism](Realism.md#delivery-to-the-controller) and in `realism/README.md`.

Decisions: a report is delivered whether its checks pass or fail, and the exit code is 2 when the controller does not accept it, otherwise the exit code of the checks. `score` now sets `allow_abbrev=False`, because argparse would have read `--token SECRET` as `--token-file SECRET` and printed the secret in an error. The report format stays 0.16.0, so there is nothing to bump against CON-28.

### Shared page notes

- [Testing](Testing.md): the realism runner list gains `realism/tests/test_deliver.py`, which needs no tool.
- [Controller](Controller.md): none, the endpoint is as CTL-41 documented it.
- Module Development: not affected. The option is for operators and CI, and a module author scores a fixture with the same command as before.
- [Home](Home.md) and [Implementation Plan](Implementation-Plan.md): none.
