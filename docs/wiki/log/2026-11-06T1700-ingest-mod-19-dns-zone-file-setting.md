## [2026-11-06] ingest | MOD-19 dns zone_file setting

The dns module now reads the module setting `zone_file` (decision D12 item 22). The manifest carries a settings schema with `zone_file`, and the crate has `Settings` and `Dns::from_settings`, which load the zone, resolve a relative path against the module directory, and reject an unknown key, a wrong type, an unreadable file, a malformed zone with its file and line, and a zone without an SOA record. Without a setting the server answers from the shipped zone as before. `fixture_peer` takes `--settings JSON`, and `capture.sh` passes the setting to both peers, so the fixture capture comes from a configured zone. New transcript tests show that naming the shipped zone through the setting renders the same bytes as the default, and that another zone changes the answers and not the queries. We documented the accepted subset of RFC 1035 master file syntax in the module README and the result in [Modules](../Modules.md#the-dns-module).

### Shared page notes

- [Contracts](../Contracts.md): the agent protocol's `ModuleRef` has no settings field yet, so the setting does not reach an agent through the controller. This is the gap that the CTL-18 entry already names. When the field exists, the agent should call `Dns::from_settings` with the module directory as the base path.
- [Implementation Plan](../Implementation-Plan.md) and the catalog: the summary of MOD-19 cites D12 item 22, which is right, and the work assignment cited item 14, which is the criterion `where` filter. No change needed.
- The root `Cargo.lock` gained one line, because `fps-module-dns` now depends on `serde_json`, which the workspace already uses.
