# Module template

This directory is the starting point for a new module. It holds a manifest, one scenario, and this README, and it passes `tools/contracts/validate.py` as it stands. The guide [Module Development](../../docs/wiki/Module-Development.md) explains every field and every action, and the comments in the files point at its sections.

## What a copy holds

| Path | Contents |
|---|---|
| `manifest.yaml` | Identity, parameter schema, scenario list, byte estimate, and provenance of a module named `template`, with a comment on every field |
| `scenarios/request_response.yaml` | A TCP flow in which the client sends `request_size` bytes and the server answers with `response_size` bytes, using only the core function `fill` |
| `native/` | A Rust crate for first party modules only, see below |

The module declares no template functions and ships no extension, so it needs no native crate and no WebAssembly component to run.

## Start a module

The guide section "Start from the template" gives the full walkthrough. In short, copy this directory to `modules/<name>/` (or to any directory named `<name>`), replace `template` with `<name>` in `manifest.yaml` and in the `module` field of each flow, rename the scenario, and validate the copy.

```bash
python3 -m pip install -r tools/contracts/requirements.txt
mkdir -p mods && cp -r modules/_template mods/<name>
python3 tools/contracts/validate.py --modules mods --tests --no-openapi
```

The validator treats every subdirectory of the directory that `--modules` names as a module, and it requires the directory name to equal the module name. In this repository `make lint-mod-template` validates the template through a link named `template`, because the directory name `_template` is not a valid module name.

## The native crate

The crate under `native/` serves first party modules, which decision [D10](../../docs/decisions/0010-native-extensions.md) allows to implement their template functions in Rust inside this repository. A module outside the repository cannot use it, because the agent links native crates at build time. Delete `native/` from a copy that lives outside the repository. A first party module renames the package to `fps-module-<name>` and follows the Modules section of the [Coding Standards](../../docs/wiki/Coding-Standards.md#modules).
