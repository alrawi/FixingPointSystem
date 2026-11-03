## [2026-11-03] ingest | MOD-01 HTTP/1.1 module

We added the `http1` module under `modules/http1/` and the new component page [Modules](Modules.md). The native crate `fps-module-http1` implements `http1.request` and `http1.response` behind the `TemplateFn` trait of `fps-flow`, as decision D10 allows, and the manifest lists both under `functions` with contract v0.2. Six scenarios cover a single fetch, keepalive browsing with a chunked page, length delimited and chunked uploads, and HTTP/1.0 with and without keepalive, all with synthetic content. The tests validate the module, run client and server against each other over the in memory duplex, compare golden byte transcripts at seed 42, and compare the native catalog with the manifest. The example `fixture_peer` runs every scenario over the kernel TCP backend in the QA-01 harness, and nDPI classifies 120 of 120 fixture sessions as HTTP with no tshark parser errors. The new file `mk/mod-http1.mk` adds `lint-mod-http1`, `test-mod-http1`, and `e2e-mod-http1`, and `.github/workflows/mod-http1.yml` runs them. The evidence is `make lint-mod-http1 test-mod-http1`, `sudo make e2e-mod-http1`, and `docs/handoff/MOD-01.md` on the branch `wp/MOD-01`.

### Shared page notes

- [Home](Home.md) and the sidebar could link the new [Modules](Modules.md) page.
- [Testing](Testing.md) says that `validate.py --modules modules/<name>` passes, but the validator treats each subdirectory of the given directory as a module. Either the sentence or the validator should change, for example by accepting a directory that holds a `manifest.yaml`. Until then `lint-mod-http1` validates through a parent directory that links to the module.
- [Testing](Testing.md) could add the rows `test-mod-http1` and `e2e-mod-http1` to the target table. The second one needs root.
- The root `.gitignore` ignores `*.pcapng` except under `contracts/examples`, so the fixture was added with `git add -f`. An OPS change could add `!modules/*/fixtures/*.pcapng`.
- MOD-02 can copy `native/examples/fixture_peer.rs` and `fixtures/capture.sh` to record its fixture before the agent runs plans.
