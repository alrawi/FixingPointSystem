## [2026-11-12] plan | A module development guide for third party authors

The owner asked that every change to module development be documented so that third parties can develop their own modules. The project had no guide for module authors outside the project. [Modules](../Modules.md) catalogs the first party modules, and `modules/_template/` holds a three line README.

We add the following.

- MOD-26 writes [Module Development](../Module-Development.md), a guide for third party authors, and makes `modules/_template/` a complete starting point that validates.
- [Coding Standards](../Coding-Standards.md) gains a rule. A pull request that changes how a module is written updates the guide in the same pull request. A package whose paths do not include the guide puts the text in its Shared page notes, and the coordinator applies it before merge.

### Shared page notes

None.
