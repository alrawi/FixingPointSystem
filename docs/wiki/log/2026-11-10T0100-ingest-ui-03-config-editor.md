## [2026-11-10] ingest | UI-03 Test configuration editor

The UI has a configuration list at `/configs` and an editor at `/configs/new` and `/configs/{id}`. Forms for network, profile, load, timeline, and criteria (plus name and topology) and a YAML view edit one text, so an edit in either shows in the other. Ajv checks the text against `contracts/schemas/test-config.schema.json` in the browser and puts each failure at its field by JSON Pointer. Save as new posts the YAML text to `createConfig`, and Validate on controller runs `validateConfig` on the stored copy, storing the text first when it changed, and shows the findings next to the fields. The round trip definition is: text stays byte for byte until a form edits it, an edit goes through a `yaml` Document serialized with `lineWidth: 0` so comments, key order, and style survive, and JSON from the controller shows as canonical YAML in schema key order. `ui/src/config/roundtrip.test.ts` runs every file of `contracts/examples/tests/` through the model and asserts byte identity. We described the editor, the definition, and the mock routes in [Web UI](Web-UI.md).

The package added `ajv` and `yaml` as runtime dependencies of `ui/package.json` (they were development dependencies), which changes `ui/package-lock.json` only by dropping their `dev` flags. The editor route loads lazily, so the initial JavaScript stays at 112.2 kB gzip against 250 kB. The mock server gained the four configuration operations, and `mock/configs.test.ts` validates their responses against the OpenAPI document.

### Shared page notes

- Testing: the UI level now includes `mock/configs.test.ts` and `ui/e2e/configs.spec.ts`. Both run inside the existing `make test-ui` and `make e2e-ui` targets, so no target changes.
- Contracts: no change. Two observations for the owner. `getConfig` returns only JSON, so comments in a stored YAML are lost when the editor reloads a stored configuration. An update operation (`PUT /configs/{id}`) does not exist, so the editor can only save as new.
- Implementation Plan: UI-03 is done once merged. UI-07 can use `/configs/new` and the labels in `ui/e2e/configs.spec.ts` for the configuration flow.
