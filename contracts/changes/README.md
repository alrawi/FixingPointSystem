# Contract change entries

Each pull request that changes a contract adds one file `<ID>.md` here, for example `CON-06.md`. The file states the contract, the new version, and the change in a few sentences. Parallel contract packages therefore never edit the same lines of `contracts/CHANGELOG.md`. The coordinator folds the entries into `CHANGELOG.md` and removes them when a contract version ships.
