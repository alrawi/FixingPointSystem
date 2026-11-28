## [2026-11-28] ingest | bug #704: controller refuses two mix entries on one server port

- `compiler.Compile` and the configuration validator now apply the rule of `tools/contracts/validate.py` `_check_server_ports`: first flow only, accept port a literal or a single `{{ vars.<name> }}` (entry param, else scenario var default), key of transport, server pool (empty means the first server pool) and port, app_sim and server_sim only. A clash is an error at `/profile/mix/<i>` that names both entries and says to give one its own `server_port` param or `server_pool`.
- The compiler test configurations `pinned_port.yaml` and `uneven_pool.yaml` gave web_get and web_browse one port, and now give web_browse `server_port: 8081`. Their golden plans changed by that param only.

### Shared page notes

Module Development, "Watch out for", entry "Two mix entries cannot share one server port": append "The controller refuses such a configuration too, at compile time and in configuration validation, with an error at `/profile/mix/<i>` (bug 704)."
