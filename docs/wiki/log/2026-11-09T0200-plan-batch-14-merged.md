## [2026-11-09] plan | Batch 14 merged

The coordinator merged CTL-11 in PR #256, CTL-26 in PR #255, CTL-27 in PR #257, CON-12 in PR #254, and AGT-05 in PR #259, with the owner's approval of CON-12 and of the HTTPS enrollment endpoint for mutual TLS. The regen workflow gained a package input in PR #253, which regenerated the REST API types for CON-12. Since CTL-27 the controller requires mutual TLS on agent gRPC, so the image of IMG-02 sets the explicit `insecure_plain_grpc` flag until AGT-08 gives the agent mutual TLS. IMG-02 waits for its boot tests on the owner host.

We applied the shared page notes to [Agent](../Agent.md), [Controller](../Controller.md), and [Home](../Home.md). The catalog gains ENG-19 (capture TCP sessions in the runtime), CTL-28 (verify module signatures at install and in plans), and CTL-29 (rebuild the agent inventory after a restart). How the controller fetches a capture file from an agent is an open owner decision, because CON-05 defines no transfer.

### Shared page notes

None.
