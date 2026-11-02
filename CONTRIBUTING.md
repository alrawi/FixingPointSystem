# Contributing

We build FixingPointSystem with a swarm of AI coding agents under human review (decision D9). The rules below apply to agents and human contributors alike.

## Contracts first

The files under `contracts/` define the interfaces between workstreams. A pull request that changes a contract must:

1. Change only contract files, examples, and the validator in that pull request.
2. Pass `python3 tools/contracts/validate.py` and `python3 -m pytest tools/contracts`.
3. Bump the contract version in the changed file and record the change in `contracts/CHANGELOG.md`.
4. Receive approval from the repository owner before merge.

Implementation pull requests must not edit contract files.

## One workstream per change

Each pull request touches one workstream directory, such as `spikes/s1-engine/` or a single module directory. Cross cutting changes start as an issue.

## Gates

Each phase ends at a gate with an automated acceptance suite. The owner signs off each gate. See `docs/phase0/README.md` for Gate 0.

## Licensing

The core is licensed under GPL-3.0. Every new dependency of the agent or controller, and every module, must carry a license compatible with GPL-3.0. `cargo deny check licenses` enforces the Rust side through `deny.toml`. Tools with incompatible licenses may run only as separate processes.

## Content provenance

Module content must be synthetic or carry a documented license. Never commit packet captures that contain personal data, credentials, or third party content without a license.

## Commit messages

Use the imperative mood in the subject line and keep it under 72 characters. Reference the decision or gate the change serves when one applies.
