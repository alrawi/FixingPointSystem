## [2026-11-28] ingest | bug #710: apt-install.sh sets aside only the sources that failed

- The fallback of bug #696 renamed every non-Ubuntu source file after a failed `apt-get update`, including the zeek repository that the module and realism jobs add, so `zeek-core` could not install when the image's `packages.microsoft.com` source answered 403.
- `tools/ci/apt-install.sh` now keeps the output of each update and sets aside only the `.list`/`.sources` files that name a host from its `Failed to fetch` or `Err:` lines. A failure that names no host keeps every third party source. `tools/ci/test_apt_install.sh` covers both (17 checks).

### Shared page notes

None.
