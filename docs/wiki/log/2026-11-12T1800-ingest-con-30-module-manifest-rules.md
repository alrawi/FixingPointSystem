## [2026-11-12] ingest | CON-30 module manifest rules that match the registry and the agent

CON-30 (module manifest v0.5, `contracts/changes/CON-30.md`) closes the gaps that MOD-26 listed between the module contract and the code. The manifest schema allows only the constant `extension.wasm` for `extension.wasm`, because the agent loads only that file at the module root. `tools/contracts/validate.py` now applies the registry rules for the license, file names, links, and named files, and it checks that a key of `expected_bytes` names a scenario. `contracts/README.md` names the version of every contract. The schema description of the reserved module types now says what is true. [Contracts](../Contracts.md) names manifest v0.5.

What the package found, which the coordinator and the owner may want to act on.

- The registry does not refuse an `expected_bytes` key that names no scenario, although the work package listed it with the registry rules. Only the validator checks it. A follow up in `controller/internal/registry/` (`checkTree`) can add the check.
- The registry knows five reserved names and the validator eight. The validator was already a superset, so no module that the registry refuses passes it. The disagreement stays in the other direction: the registry accepts `profiles`, `dist`, and `all`. A follow up in `reservedModuleNames` closes it.
- Constraining the schema breaks `TestParseManifestRenderHook` in `controller/internal/registry/registry_test.go` (it uses `wasm: dns.wasm`) and the generated `ui/src/api/schema.d.ts`, both outside the paths of this package. The change entry names the fixes.
- No reader rejects the reserved module types `tunnel`, `strike`, `evasion`, and `fuzzer`. The package corrected the schema text and added no rejection.

### Shared page notes

The coordinator applies these to `docs/wiki/Module-Development.md` before merge, because the page is not in the paths of this package. Each item gives the exact text to find and the exact text that replaces it. A replacement that is a whole section or list item says so. The package closes gaps 4 (the path part) and 8 of the Gaps section, and the guide must list no gap that it closes.

1. **The contracts at a glance.** In the table row `| Module manifest | v0.4 | ...`, replace `v0.4` with `v0.5`.

2. **Table of manifest fields, row `license`.** Replace the whole row with:

   `| `license` | An SPDX expression, see [Licenses](#licenses). The schema checks the length, and `validate.py` and the registry check the expression. | Schema (length), `validate.py`, registry |`

3. **Table of manifest fields, row `expected_bytes`.** Append to its meaning column, after `see [below](#expected_bytes-bytes-per-session-of-each-scenario).`, the sentence ` `validate.py` rejects a key that names no scenario of `scenarios`.` and replace the consumers column `Controller compiler` with `Controller compiler, `validate.py` (keys)`.

4. **Table of manifest fields, row `extension`.** Replace the whole row with:

   `| `extension` | `wasm`, which must be the text `extension.wasm` because the agent loads only that file at the module root, and `hooks` (a non empty list from `on_open`, `on_data`, `on_timer`, `on_close`, `render`). | Schema, `validate.py`, registry, agent |`

5. **Subsection `expected_bytes`.** After the sentence `Each value is an integer of at least 1, and each key follows the identifier pattern of scenario names (...)` add the sentences: `Each key must also be the name of a scenario that `scenarios` lists, and `validate.py` reports a key that is not (a typo in a key leaves the real scenario without an estimate). The registry does not check the keys yet.` Replace the sentence `Scenarios in `scenarios-pending/` are not in the manifest and need no entry.` with `Scenarios in `scenarios-pending/` are not in the manifest, need no entry, and may not have one.`

6. **Module layout, rule 2** (the numbered rule that starts `Every path that the manifest names is relative`). Replace its last sentence `so a file with a space in its name is refused at install although the validator accepts it.` with `so a file with a space in its name is refused at install, and `validate.py` refuses it as well (CON-30).` Replace rule 3 (`The tree holds no symbolic link...`) with: `3. The tree holds no symbolic link and no special file. The registry, the digest, and the agent refuse one (`registry/digest.go`, `registry/archive.go`, `modules.rs`), and `validate.py` reports one (`tree_problems`). A link as the module directory itself is fine for the validator, which is how the first party targets validate a module.`

7. **Names.** Replace the whole second sentence group from `The registry refuses only` to the end of the paragraph with:

   `The registry refuses only `vars`, `session`, `last`, `true`, and `false` (`controller/internal/registry/manifest.go`, `reservedModuleNames`), so a module named `profiles`, `dist`, or `all` passes the install and fails the validator, which rejects all eight. Treat all eight as reserved.`

8. **Module types.** Replace the paragraph that starts `The schema comment says that v1 agents reject the reserved types` with:

    `The schema description names `tunnel`, `strike`, `evasion`, and `fuzzer` as reserved. The schema accepts them and no reader rejects them, so a manifest of such a type validates and installs, and no engine behavior exists for it. The one rule that depends on the type is the `raw` action, which the validator and the engine allow only for `strike` and `fuzzer` modules (`tools/contracts/validate.py`, `RESERVED_ACTION_TYPES`, and `engine/fps-flow/src/check.rs`). Because no type admits it today, do not write `raw`.`

9. **Licenses.** Replace the last sentence `The validator checks none of this (it reads only the schema's minimum length), so a module with an unlisted license validates and fails at install.` with `The validator applies the same parser and the same lists (`tools/contracts/validate.py`, `license_problem`), and a test compares them with the Go source, so a module with an unlisted license fails the validator and not only the install.`

10. **WebAssembly extensions, table row `File name`.** Replace the whole row with:

    `| File name | The agent compiles the file `extension.wasm` at the root of the module tree. The schema allows no other value for `extension.wasm` in the manifest (CON-30), and a module that ships the file needs no `extension` key for the agent to load it | `engine/fps-runtime/src/extension.rs` (`EXTENSION_FILE`), `modules.rs`, `contracts/schemas/module-manifest.schema.json` |`

11. **Validate, numbered list, item 2.** Replace the whole item with:

    `2. **Module rules.** The directory name equals `name`, the name is not reserved, the license is on the allow list, every file and directory of the tree has a path of letters, digits, `_`, `.`, and `-`, the tree holds no symbolic link, no module requires itself, function and parameter names are unique, parameter defaults match their types, every path that `scenarios`, `assets`, `fixtures`, and `extension.wasm` names is a regular file of the tree, and every key of `expected_bytes` names a scenario of `scenarios`.`

12. **Validate, the paragraph that starts `The validator does not check everything that the controller checks at install.`** Replace the whole paragraph with:

    `The validator applies the rules that the registry applies to a tree: the license, the file names, links, and named files (`tools/contracts/validate.py`, `license_problem`, `tree_problems`, `names_regular_file`). It is stricter than the registry for the reserved names `profiles`, `dist`, and `all` and for the keys of `expected_bytes`. It does not check the `settings` schema or the signature, and it never runs extension code. Check these by hand before you package.`

13. **Upload, the list of registry checks.** Replace item 4 `**Semantic rules.** Function names are unique, `render` is listed when functions and an extension exist, and the name is not reserved.` with the same text and add ` `validate.py` reports each of them.`

14. **Before you publish.** Replace the bullet `- `license` is on the allow list, and `provenance` states the real source.` with `- `license` is on the allow list, which `validate.py` checks, and `provenance` states the real source.` Replace the bullet `- The tree holds no symbolic link and no file name outside the `relpath` characters.` with `- The tree holds no symbolic link and no file name outside the `relpath` characters, which `validate.py` checks.`

15. **Gaps.** Replace gap 4 with:

    `4. **No WebAssembly toolchain for authors.** The host exists, and the repository gives no SDK, example, or instructions. Only `render` and `on-close` run (`engine/fps-runtime/src/extension.rs`).`

    Replace gap 7 with:

    `7. **Declared and not enforced.** `engine_api` is read by nothing that compares it with an engine version, `assets` and `fixtures[].expect` are never read beyond the existence of a file, and `provides` and the version of `requires` document only (`controller/internal/registry/manifest.go`, `tools/contracts/validate.py`). A search of `controller/`, `engine/`, and `realism/` found no other reader. The module types `tunnel`, `strike`, `evasion`, and `fuzzer` are reserved in the schema text and no reader rejects them.`

    Replace gap 8 with:

    `8. **The registry is looser than the validator in two places.** It accepts the names `profiles`, `dist`, and `all`, which the validator rejects, and it does not check that a key of `expected_bytes` names a scenario (`controller/internal/registry/manifest.go`, `reservedModuleNames`, and `registry.go`, `checkTree`). Both are safe for the author, because a module that validates also installs.`

    Gap 9 and gap 10 keep their text and their numbers.
