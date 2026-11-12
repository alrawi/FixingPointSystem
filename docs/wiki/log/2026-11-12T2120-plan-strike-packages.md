## [2026-11-12] plan | Packages for the strike module

The owner asked on 2026-11-12 to add the follow up packages of issue #491 to the catalog. The owner implements the strike module by hand, so its three packages carry an owner action and the `needs:owner` label, and workers do not claim them.

The catalog gains the following packages.

- MOD-27: the first slice of the strike module, which is PR #492. It reserves `modules/strike/` in the planner.
- MOD-28: fixtures for the strike module, after MOD-27 and RLM-20.
- MOD-29: raw strikes, after MOD-28 and ENG-29. Stateless floods and spoofed pools need an engine design of their own.
- RLM-20: expected parser findings in the realism labels, which deliberately malformed traffic needs. Workers take it.
- ENG-29: the raw action in the engine. Workers take it.

### Shared page notes

None.
