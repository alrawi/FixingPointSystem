## [2026-11-09] plan | Batch 21 merged

The coordinator merged ENG-22 in PR #341, RLM-08 in PR #343, CTL-37 in PR #342, AGT-13 in PR #345, MOD-04 in PR #346, and ENG-21 in PR #344, and fixed the flaky controller e2e test for CLI-03 in PR #347. The CLI-03 worker found that the failure was in `controller/cmd/fps-controller/e2e_test.go`, not in `cli/e2e`: the orchestrator moves a run to running before Start reaches the agent. For MOD-04 the coordinator made the report print step of seven workflows accept entries without `applications`, and added the decrypted HTTPS check to `mod-http1`. For ENG-21 the coordinator added the two agent lines after AGT-13 merged.

With AGT-13 the real agent runs the smtp, imap, and pop3 modules; the controller e2e test relays mail through smtp and content. FTP is registered but needs multi flow instances in the runtime.

The catalog gains ENG-23 (multi flow scenario instances in the runtime), AGT-16 (multi flow scenarios and digested extension bytes in the agent), and CON-18 (decide the transport keyword of the scenario contract, which needs owner approval). AGT-14 now also carries the per session TLS connector and the server profiles that MOD-04 found it needs.

### Shared page notes

None.
