# CON-30 Module manifest rules that match the registry and the agent

Contract. Module manifest schema `contracts/schemas/module-manifest.schema.json`, `contracts/README.md`, and the module rules of `tools/contracts/validate.py`. MOD-26 wrote the module development guide and listed the places where the documents and the code that third party authors meet disagree. The owner approved this change as a contract change on 2026-11-12 (comment on issue #511).

## Versions

- The module manifest schema moves from v0.4 to v0.5. The `extension.wasm` path is now a constant, which refuses a value that v0.4 accepted, so the change takes a minor number, as CON-18 did when it tightened `transport`. No module in the repository and no example sets another path, so every manifest that a v0.4 reader accepted and an agent could use stays valid.
- OpenAPI (0.12.0), the agent protocol (0.2.4), the scenario contract (v0.6), the test configuration (v0.3), and the metric catalog (0.8.0) stay as they are.
- `contracts/README.md` carried the heading "Contracts v0.2" and the catalog version 0.5.0. It now has no single version and names the version of every contract.

## Decision: the `extension.wasm` path

The schema accepted any relative path in `extension.wasm`, and the agent loads only the file `extension.wasm` at the module root (`engine/fps-runtime/src/extension.rs`, `EXTENSION_FILE`, and `engine/fps-agent/src/modules.rs`, `walk`). A module with `wasm: ext/guest.wasm` therefore validated and installed, and its extension never ran.

Two options existed. The agent could honor the path, or the schema could allow only the file name that the agent loads. The first needs changes in `engine/fps-agent`, `engine/fps-runtime`, the digest pass, and the plan, none of which are in the paths of this package, and the digest pass would have to learn which file of a tree is an extension. The second breaks no module, because every manifest in `modules/` and `contracts/examples/` either has no `extension` key or names no other file. The schema therefore allows only `extension.wasm`: the key `wasm` takes the constant `extension.wasm`. The key `extension` stays, because it declares the hooks, and an agent loads `extension.wasm` of a tree whether the manifest has the key or not.

A later change that lets a module name another file widens the constant to the `relpath` pattern, which is additive, and it changes the agent in the same pull request.

## Decision: reserved module types

The schema said that v1 agents reject the reserved types. No reader does. The package does not add the rejection, because that would change which manifests a controller installs and no owner decision asks for it. The description now says what is true: the types `tunnel`, `strike`, `evasion`, and `fuzzer` are reserved, the schema accepts them, no reader rejects them, and the only rule that depends on the type is that the `raw` action needs a `strike` or `fuzzer` module.

## What the validator checks now

`check_module_dir` in `tools/contracts/validate.py` applies the rules that the registry applies at install (`controller/internal/registry/`). Each rule has tests in `tools/contracts/test_contracts.py`.

| Rule | Registry source | Validator before | Validator now |
|---|---|---|---|
| Reserved names | `manifest.go`, `reservedModuleNames`: `vars`, `session`, `last`, `true`, `false` | Rejected all five, and also `profiles`, `dist`, and `all` | Unchanged. The five are the constant `REGISTRY_RESERVED_NAMES`, a test compares it with the Go list, and the three extra names stay rejected |
| License | `license.go`, `CheckLicense` | Length only | The same parser, allow list, incompatible list, and exceptions. Tests compare the three lists and the cases with the Go source |
| File names | `archive.go`, `entryPath`, `hasDotSegment` | Only the named paths, through the schema | Every file and directory of the tree |
| Links | `archive.go`, `copyTree`, `digest.go`, `ErrSymlink` | Not checked | A symbolic link or special file anywhere in the tree is an error. A link as the module root, which the first party targets use, is not |
| Named files | `registry.go`, `checkTree`, `Lstat` and `IsRegular` | A path that exists | A regular file that is not a link |
| `expected_bytes` keys | None | Not checked | Each key must name a scenario of `scenarios` |

The last row is not a registry rule. The registry parses `expected_bytes` and never compares its keys with the scenario names, and the controller reads only the keys that a mix names. A typo in a key therefore installs and silently leaves the real scenario without an estimate. The validator is stricter than the registry here, on purpose, and a follow up in `controller/internal/registry/` (`checkTree`) can refuse the same keys. The check waits when a scenario file failed to load, because its name is then unknown, and it does not count the files of `scenarios-pending/`.

The reserved names stay a disagreement in the other direction. The validator refuses `profiles`, `dist`, and `all`, which the template grammar and the metric catalog reserve, and the registry does not. A follow up in `reservedModuleNames` closes it.

## Compatibility

All 15 first party modules with a manifest and the template validate with the new checks. A module that passed the old validator and fails now has a license off the allow list, a file name with a character outside letters, digits, `_`, `.`, and `-`, a link in its tree, a named path that is a directory or a link, an `extension.wasm` path other than the root file, or a stale `expected_bytes` key. The registry refuses the first four at install, and the agent loads none of the fifth.

## Consumers

| Consumer | Package | Uses |
|---|---|---|
| Module authors | MOD | The guide `docs/wiki/Module-Development.md` describes the rules, and the log entry of this package holds the replacement text |
| Controller registry copy | CTL | `controller/internal/registry/schema/module-manifest.schema.json` is a byte copy of the schema |
| Validator tests | CON | `tools/contracts/test_contracts.py` compares the Go lists with the validator lists |

## Follow up outside the catalog paths

The coordinator authorized the byte copy of the schema in `controller/internal/registry/schema/` on the branch of this package.

- `controller/internal/registry/registry_test.go`, `TestParseManifestRenderHook`: the manifest line `wasm: dns.wasm` becomes `wasm: extension.wasm`. The test expects the finding `/extension/hooks`, and the schema now refuses the old path first.
- `ui/src/api/schema.d.ts`: regenerated with `npm run gen:api`, because the descriptions and the type of `extension.wasm` come from the schema.
- `controller/internal/registry/` (a CTL change): refuse an `expected_bytes` key that names no scenario, and add `profiles`, `dist`, and `all` to `reservedModuleNames`.
