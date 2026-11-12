## [2026-11-12] ingest | MOD-26 module development guide

MOD-26 replaces the placeholder of [Module Development](../Module-Development.md) with a guide for third party module authors. Every statement of the guide names the file or contract that it rests on. The package also makes `modules/_template/` a complete starting point and adds `mk/mod-template.mk` with the target `lint-mod-template`, which validates the template through `tools/contracts/validate.py`.

What the guide records about the gap between the documents and the code, each of which the Gaps section lists.

- The manifest accepts any path for `extension.wasm`, and the agent loads only the file `extension.wasm` at the module root.
- The agent calls only the `render` and `on-close` hooks. There is no guest SDK, example, or build instruction for a WebAssembly component.
- The agent refuses settings for any module without native functions, so a third party module cannot use `settings`.
- Nothing compares `engine_api` with the engine version, and the catalog does not read `expected_bytes` yet (CTL-47).
- There is no signing command, no distribution of module trees to agents, and no registry outside one controller. The Python SDK `install_module` cannot send the signature headers.
- The registry knows five reserved names and `validate.py` knows eight. The validator does not check the license, file name characters, or links, which the registry does.
- Decision D10 still has the status Proposed.

### Shared page notes

The coordinator applies these before merge.

1. Add `.github/workflows/mod-template.yml`, which runs the template lint in CI. The package may not add it. The text follows, modelled on `mod-dns.yml`.

```yaml
# SPDX-License-Identifier: GPL-3.0-only
#
# Workflow of the module template under modules/_template/. It proves that a copy
# of the template validates, so the guide docs/wiki/Module-Development.md never
# starts a reader from a broken module.
name: mod-template

on:
  push:
    branches: [main]
    paths:
      - modules/_template/**
      - tools/contracts/**
      - contracts/schemas/**
      - mk/mod-template.mk
      - .github/workflows/mod-template.yml
  pull_request:
    paths:
      - modules/_template/**
      - tools/contracts/**
      - contracts/schemas/**
      - mk/mod-template.mk
      - .github/workflows/mod-template.yml

permissions:
  contents: read

jobs:
  module:
    runs-on: ubuntu-24.04
    timeout-minutes: 10
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with:
          python-version: "3.12"
      - run: python -m pip install -r tools/contracts/requirements.txt
      - run: make lint-mod-template PYTHON=python
```

2. Link [Module Development](../Module-Development.md) from Home and from the sidebar.
3. The heading of `contracts/README.md` reads "Contracts v0.2", although the schemas are manifest v0.4 and scenario v0.6. The CON owner may correct it.
4. The CON owner may decide whether the manifest `extension.wasm` path should be fixed to the file name that the agent loads, or the agent should honor the path.
