## [2026-11-28] ingest | bug #706: the agent's port clash message names the remedy

- `assemble` now says `mix entries X and Y both listen on tcp A:P; give one entry its own `server_port` in its params or its own `server_pool``. `engine/fps-agent/tests/fallback.rs` pins the remedy text; it was the only test that pinned the message.

### Shared page notes

None. Module Development ("Two mix entries cannot share one server port") does not quote the agent's message, so no text there changes.
