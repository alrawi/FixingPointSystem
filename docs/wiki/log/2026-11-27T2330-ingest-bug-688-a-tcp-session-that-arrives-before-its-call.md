## [2026-11-27] ingest | bug #688: the datapath hub keeps a TCP session that arrives before its call

- `Port::route` in `engine/fps-agent/src/transport.rs` used to hand back a TCP session whose host no call held, and the hub reset it after the handshake. A client that dialed a moment before the server side bound (`dns/tcp_fallback`, the segment flows of `hls/vod_segments`) failed with `unexpected_reset`.
- The port now parks such a session for `PENDING_GRACE` (2 s, the `OPEN_GRACE` of `fps-runtime`; at most 256 per port) and `subscribe` gives it to the call that holds its host. The hub resets the sessions whose grace is over. UDP ports keep dropping at once.
- `dns/tcp_fallback` and `hls/vod_segments` left the `excluded` lists of `profiles.yaml` and the pinned list in `engine/fps-agent/src/backend.rs`. `strike/conn_churn` (bug #689) stays.

### Shared page notes

- `docs/wiki/Transport.md`, section on the TRN-36 exclusion list: the table has two rows fewer (`dns/tcp_fallback`, `hls/vod_segments`) and "Three scenarios" becomes "One scenario"; the fix of both rows is bug #688 (`PENDING_GRACE` in `engine/fps-agent/src/transport.rs`).
- `docs/wiki/Module-Development.md`, Watch out for, "A TCP flow that follows another on the same port is reset on AF_XDP": replace the last sentence with "The hub of the agent now holds such a connection for 2 s (`PENDING_GRACE`, bug #688) and gives it to the flow that binds the port, so these scenarios run on AF_XDP. A server side that binds later than 2 s after its client dialed still sees the reset; bind the port earlier or start the client later with `after`."
