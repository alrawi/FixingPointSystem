## [2026-11-13] ingest | OPS-07 take over a blocked item

`fps_swarm.py takeover` refused an issue that carried `status:blocked`, although the [Swarm Protocol](../Swarm-Protocol.md#takeover) defines a takeover as the claim of an item whose newest card is expired, released, or blocked. The coordinator had to relabel an item by hand (CTL-49, batch 34; PR or issue #527 on 2026-11-13). The tool now accepts `status:blocked` and keeps every other check of the claim steps. The promotion sweep also reports each blocked item whose blocking comment names no blocking issue, or that has no blocking comment, with a `blocked:` line on stderr. The [Operations](../Operations.md#swarm-helper-blocked-items) page describes both changes. The protocol text already matched the new behavior, and a test reads it to keep the two in agreement.

### Shared page notes

- [Swarm Protocol](../Swarm-Protocol.md), section Takeover: add "It also accepts an item that carries `status:blocked`, and it still refuses an item that carries `needs:split` or `type:followup`, because those need a planner first." after the sentence about `status:in-progress`.
- [Swarm Protocol](../Swarm-Protocol.md), section Block: after "A marker without issues means that only the owner can unblock the item, so the sweep leaves it alone." add "The sweep prints a `blocked:` line on stderr for such an item, and for a blocked item without any marker, so the coordinator sees which items wait for a person."
- [Swarm Protocol](../Swarm-Protocol.md), helper table: no change is needed; the `takeover` row already says "or was blocked".
- Workflow: no change to `.github/workflows/swarm-scan.yml`. The new line goes to stderr, so the scanner's stdout rows keep their format.
