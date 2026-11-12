## [2026-11-12] ingest | CTL-50 Registry rules as strict as the validator

CON-30 made `tools/contracts/validate.py` check every rule that the registry enforces, and found the registry looser in two places (`contracts/changes/CON-30.md`). Both are closed in `controller/internal/registry/`.

- `reservedModuleNames` (`manifest.go`) holds all eight reserved names: `vars`, `session`, `last`, `true`, `false`, `profiles`, `dist`, and `all`. An install of a module with one of them fails with the finding `/name`, and the message now says the name is reserved by the template grammar or the metric catalog.
- `checkTree` (`registry.go`) refuses a key of `expected_bytes` that names no scenario of the manifest. Each such key is one finding with the path `/expected_bytes/<key>` (JSON Pointer escaping), in sorted order. The check runs only when every scenario file gave a name, because the name of a failed file is unknown and its own finding already fails the install. The scenarios of `scenarios-pending/` do not count, since the manifest does not list them.

Evidence: `TestInstallRejects` has the cases "module named dist" (finding `/name`) and "misspelled expected_bytes key" (`web_gte` in the `http1` example, finding `/expected_bytes/web_gte`), `TestParseManifestFindings` has `profiles`, `dist`, and `all`, and `TestUnknownEstimateKeys` checks the order and the empty case. The example modules `http1` and `dns` still install, because their keys name their scenarios. Go cannot build in this sandbox, so CI is the test loop. `gofmt -l` is clean.

Findings:

- **The contract test fails until a CON change lands.** `test_reserved_names_equal_the_registry_list` in `tools/contracts/test_contracts.py` compares the names of the Go map with `REGISTRY_RESERVED_NAMES` in `tools/contracts/validate.py`, which holds the five old names, and asserts that they are a subset of `RESERVED_ROOTS`, which lacks `all`. With eight names in the Go map the test fails, and no change inside `controller/internal/registry/` can make it pass without hiding a name from its regular expression. Both files are in CON paths, so this package does not edit them. The change is small: in `validate.py` set `REGISTRY_RESERVED_NAMES = RESERVED_ROOTS | {ALL_APPLICATIONS}` and update the comment above it (the validator and the registry now agree, so the sentence that says the validator is stricter goes), and in the test replace `assert v.REGISTRY_RESERVED_NAMES <= v.RESERVED_ROOTS` with `assert v.REGISTRY_RESERVED_NAMES == v.RESERVED_ROOTS | {v.ALL_APPLICATIONS}` and delete `test_validator_also_rejects_the_names_the_registry_lacks`, whose three names the first parametrized test now covers. The coordinator applies it on the branch of this package or in a CON follow up.
- The registry now refuses what the validator refused, so a module that validates and one that installs differ in no name or key rule. The registry can still be stricter than the validator elsewhere, which this package did not check.
- A manifest that lists no scenario but has `expected_bytes` keys now fails every key, which is right, since no key can name a scenario.

### Shared page notes

[Module Development](../Module-Development.md) is in the paths of the MOD workstream, so the coordinator applies these edits before the merge. After them the registry and the validator agree on both rules, and the guide must not say otherwise. Replace the text between the quotation marks, which are not part of it.

1. Names section (line 159). Replace the paragraph that starts "A module name must not collide" with:

   "A module name must not collide with a template root. The grammar reserves `vars`, `session`, `last`, `profiles`, `dist`, `true`, and `false`, and the metric catalog reserves `all` as the label value that totals every application (`contracts/README.md`, `tools/contracts/validate.py`, `RESERVED_ROOTS` and `ALL_APPLICATIONS`). Both the registry (`controller/internal/registry/manifest.go`, `reservedModuleNames`) and the validator refuse all eight, so a module with one of these names fails at install with the finding `/name` and fails the validator."

2. Gaps section. Delete item 8 ("The registry is looser than the validator in two places...") and renumber item 9 to 8 and item 10 to 9. Nothing else in the guide refers to the item numbers.

3. `expected_bytes` row of the manifest table (line 109). Replace "`validate.py` rejects a key that names no scenario of `scenarios`." with "The registry and `validate.py` reject a key that names no scenario of `scenarios`." and the last cell "Controller compiler, `validate.py` (keys)" with "Controller compiler, registry (keys), `validate.py` (keys)".

4. The paragraph of the `expected_bytes` subsection (line 126). Replace "and `validate.py` reports a key that is not (a typo in a key leaves the real scenario without an estimate). The registry does not check the keys yet." with "and both the registry and `validate.py` report a key that is not, with the finding `/expected_bytes/<key>` at install (a typo in a key would otherwise leave the real scenario without an estimate)."

5. Step 4 of the measurement list (line 151 area). Replace the paragraph "Neither the registry nor `tools/contracts/validate.py` checks that a key names a scenario of the manifest, so a misspelled key surfaces only as the missing estimate of the scenario that you meant. Check the keys against the `scenarios` list yourself." with "The registry refuses an install, and `tools/contracts/validate.py` reports an error, when a key names no scenario of the manifest, so a misspelled key cannot reach a controller. A key must name a scenario that `scenarios` lists, and the files of `scenarios-pending/` do not count." (This sentence was already out of date after CON-30, which gave the validator the check.)

6. The paragraph below the validator rules (line 364). Replace "It is stricter than the registry for the reserved names `profiles`, `dist`, and `all` and for the keys of `expected_bytes`. It does not check" with "It applies the same rules for the reserved names and for the keys of `expected_bytes`, and it does not check".

7. The sentence on semantic rules (line 437) needs no change.

8. Sources table. No change.
