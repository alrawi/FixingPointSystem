## [2026-11-27] ingest | bug #696: apt-install.sh sets aside third party sources in its fallback

- From about 22:40 UTC on 2026-11-27 the hosted runner image's `packages.microsoft.com` apt source answered 403, so `apt-get update` failed all three attempts in every job that installs packages (seen on `module (ntp)` and `module (ldap)`).
- `tools/ci/apt-install.sh` now also renames every other `.list` and `.sources` file in the sources directory to `NAME.fps-ci-disabled` when its fallback runs. apt ignores that extension. CI installs only Ubuntu packages, and a successful first update still leaves every source alone.
- `tools/ci/test_apt_install.sh` checks both cases.

### Shared page notes

None.
