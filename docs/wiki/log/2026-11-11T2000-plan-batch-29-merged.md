## [2026-11-11] plan | Batch 29 merged

The coordinator merged:
- AGT-15 in PR #474, CTL-42 in PR #472 and QA-10 in PR #470 (after one rerun of a runtime test flake, ENG-28).
- RLM-10 in PR #473, after one rerun of the http2 module job (MOD-23). On its branch the coordinator also:
  - refreshed the controller's copy of the realism report schema (format 0.13.0);
  - regenerated `ui/src/api/schema.d.ts`, which had gone stale since RLM-17, and added `realism/schema/**` to the paths of ui.yml, because the OpenAPI document references the report schema.
- CON-25 in PR #471, which the owner approved on 2026-11-11. A coordinator helper finished its consumers on the branch:
  - the controller's catalog copy, the catalog pins in the contract and realism tests, and `messages` in the fps-stats label tables;
  - the messages check emits `realism.check_failures`, and the report format moves to 0.14.0;
  - the controller schema copy, the UI types, and the example count of the CON-23 controller test.
- CON-26 in PR #475, written by the coordinator, which the owner approved on 2026-11-11. REST 0.12.0 documents the 409 of a read-scoped capture download (issue #452). The generated controller code did not change; the UI types and the Python SDK are regenerated. The seeder created issue #476, closed with the merge.

Before the batch the coordinator fixed the four issues of the owner's security review: CTL-43 in PR #466 (#450), CAP-06 in PR #467 (#451), CTL-44 in PR #468 (#452) and CTL-45 in PR #469 (#453). An outage of the Ubuntu archive and cloud image hosts on the hosted runners held them for about two hours.

What the workers found:
- The enrollment test flake was two races with the fake endpoint's short renewal delay (AGT-15).
- A document with two checks of one kind, and the property wording of KS metrics, are open points of the CON-23 mapping (CTL-42).
- The Gate 3 criterion of 20,000 hosts can use `python3 -m realism score --host-count 20000` (RLM-10).

We applied the shared page notes:
- [Testing](../Testing.md) corrects the QA-09 sentence and names the batch 29 tests and flakes.
- [Home](../Home.md) has the batch 29 status line.
- The CON-25 change entry and its log note now say format 0.14.0.

The catalog gains:
- CON-27: the host identity check in the metric catalog. It needs owner approval.
- CTL-46: one property wording for realism metrics.
- ENG-28: the runtime instance test flakes of batch 29.
- MOD-23: the http2 module capture that missed one TLS session.

### Shared page notes

None.
