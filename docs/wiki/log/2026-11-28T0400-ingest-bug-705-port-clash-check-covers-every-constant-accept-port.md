## [2026-11-28] ingest | bug #705: validate.py port clash check covers every constant accept port

- `_check_server_ports` now reads the first flow's accept port when it is a literal or a single `{{ vars.<name> }}` (the entry's param of that name, else the scenario var default). Tests cover `sip_port`, a literal, an expression, and client_sim. The first-party modules and `contracts/examples/tests/` raise no new finding.

### Shared page notes

Module Development, "Watch out for", entry "Two mix entries cannot share one server port": replace the last sentence with: "`tools/contracts/validate.py` reports the clash for a test configuration whose scenarios accept on a literal port or a single `{{ vars.<name> }}`, such as `server_port` or `sip_port` (bugs 692 and 705)."

`docs/wiki/Contracts.md` was updated in this PR with the same scope change.
