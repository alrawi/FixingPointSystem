## [2026-11-09] plan | Batch 15 merged

The coordinator merged CTL-29 in PR #267, CTL-28 in PR #268, ENG-19 in PR #271, AGT-08 in PR #272, CTL-24 in PR #269, and CON-13 in PR #270, with the owner's choice to stream capture files over the agent connection. The coordinator regenerated the Go agent bindings for CON-13 with the regen workflow, and added an `UploadCapture` stub to the fake controller of the AGT-08 mutual TLS tests when the two branches met. The agent protocol is 0.2.2.

We applied the shared page notes to [Agent](../Agent.md), [Controller](../Controller.md), [Testing](../Testing.md), and [Home](../Home.md), and the `eng-capture` workflow now also runs the runtime capture test of ENG-19 with tshark required. The catalog gains CTL-30 (UpdateRates in the agent manager, which turns the CTL-24 calibration on), CTL-31 (CA in the TLS chain and trust store flags), AGT-10 (shared capture cell and pinned enrollment check in the agent), IMG-06 (mutual TLS in the image, replacing the plain gRPC stopgap), and CON-14 (signature headers and calibration history in the REST contract, which needs owner approval).

### Shared page notes

None.
