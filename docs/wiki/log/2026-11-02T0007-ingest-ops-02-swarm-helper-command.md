## [2026-11-02] ingest | OPS-02 Swarm helper command

We added `tools/swarm/fps_swarm.py`, which runs every step of the [Swarm Protocol](Swarm-Protocol.md) over the REST API, and updated that page to show each helper command next to its raw calls. The page now also defines the `fps:block` marker that the promotion sweep reads. The evidence is `docs/handoff/OPS-02.md` and the offline tests in `tools/swarm/test_fps_swarm.py`, which include two agents that claim one issue at the same time and resolve to one holder.
