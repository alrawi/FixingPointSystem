## [2026-11-27] ingest | Bug 692: two mix entries of an example shared one server port

- `enterprise_mix_10g.yaml` and `port_locations.yaml` ran `web_browse` and `web_get` on one server pool, both on tcp 8080, and an agent refuses such a plan. `web_browse` now takes `server_port: 8081` in both examples and in the golden plan of the controller (option A of the issue, no contract change).
- `tools/contracts/validate.py` reports two mix entries that accept on the same transport and port of one server pool, when the scenario accepts on `{{ vars.server_port }}`.

### Shared page notes

For `docs/wiki/Module-Development.md`, "Two mix entries cannot share one server port": append "`tools/contracts/validate.py` reports the clash for a test configuration whose scenarios accept on `{{ vars.server_port }}` (bug 692)."
