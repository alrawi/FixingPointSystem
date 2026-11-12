## [2026-11-12] ingest | AGT-21 NTP, MQTT, LDAP, and HLS in the agent

We added the four modules of batch 28 to the registry of `fps-agent`. `ntp` is stateless and uses the `stateless!` form. `mqtt`, `ldap`, and `hls` are values with state, so each has a full registration, like `http2`, and every session side clones its own copy of the fresh value. The four crates are dependencies of `engine/fps-agent/Cargo.toml`, and the registry test lists the names. `engine/fps-agent/tests/controller_e2e.sh` runs ntp/sntp_query, mqtt/publish_qos0, ldap/bind_search, and hls/vod_segments, one run each on unprivileged ports, and checks the verdict and the succeeded sessions of each application. hls/vod_segments spawns one segment flow per segment, so it covers the multi flow TCP path. The evidence is `docs/handoff/AGT-21.md`.

### Shared page notes

- Cargo.lock (shared): the agent crate gains the four dependencies, so the `fps-agent` entry of `Cargo.lock` lists them. A merge conflict resolves by running `cargo metadata` on the merged tree.
- [Modules](Modules.md) and [Testing](Testing.md): none.
