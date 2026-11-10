## [2026-11-10] ingest | AGT-19 SMB2 in the agent

The agent registry now holds `smb2` (MOD-12 and MOD-13). `fps_module_smb2::Smb2` is a unit value without state, so the registry line is `stateless!(fps_module_smb2, Smb2),` and `engine/fps-agent/Cargo.toml` depends on `modules/smb2/native`. The unit test `the_registry_covers_every_first_party_module` lists `smb2`, and `registered_with_manifest` finds `modules/smb2/manifest.yaml`, so the manifest tests and the resolution test of every function cover the module. `controller_e2e.sh` installs the module tree (with its fixtures) and runs `smb2/download` and `smb2/upload` under the real controller, one run each. Each run must pass its verdict and count succeeded sessions of the application `smb2`. The runs use the server ports 18687 and 18688, because the script runs without root and a bind to port 445 needs a privilege, and nDPI classifies SMB only on 445, but the script checks sessions and not the classification. The evidence is `docs/handoff/AGT-19.md`.

### Shared page notes

- [Testing](Testing.md): `make test-agt-controller` now also needs the `smb2` module tree and two more runs of three seconds each. It needs Go, so the check runs in CI.
