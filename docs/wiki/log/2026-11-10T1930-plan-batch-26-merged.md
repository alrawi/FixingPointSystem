## [2026-11-10] plan | Batch 26 merged

The coordinator merged:
- RLM-13 in PR #414, MOD-13 in PR #416, IMG-07 in PR #415 and AGT-18 in PR #418.
- CTL-39 in PR #411, after one rerun of the gate job (a single http1 parser error, QA-09).
- TRN-16 in PR #413. On its branch the coordinator added `libndpi-bin` and `python3-yaml` to the `capture` job of trn.yml and `realism/**` to its path filters.
- MOD-14 in PR #417, after merging main into its branch (Modules page rows).
- CON-22 in PR #412, which the owner approved on 2026-11-10. On its branch the coordinator also:
  - set APIVersion to 0.10.0;
  - regenerated `controller/internal/api`, the UI types and the Python SDK;
  - added `AttachRealismReport` stubs to `api.Unimplemented` and the wiring `pending` type, and listed it in the pending operations of the wiring test;
  - mapped `Agent.reserved_by` in `agents.ToAPI`, with a REST test assertion.

What the workers found:
- The image boot flake is a controller handshake gap. The controller sends nothing on an idle session and the agent waits for the first response frame, so a command sent 5 s after the session began fails. IMG-07 gates the boot test on a stable session; the controller fix is CTL-40.
- The run_control capture flake counted sessions open at the capture stop as dropped (ENG-27). The ftp loss came from seeded `rand_int` data ports that repeat across concurrent instances (ENG-26).
- The window scale 10 of the harness fixtures comes from the host's `net.ipv4.tcp_rmem`, which new namespaces inherit.
- SIP with RTP cannot run through the agent until the runtime opens datagram sockets in multi flow instances (ENG-26, AGT-20).

We applied the shared page notes:
- `mk/mod-smb2.mk` is replaced, and mod-smb2.yml gains a `module` job that scores the fixture.
- The sip_rtp module gets `mk/mod-sip.mk` and `.github/workflows/mod-sip.yml`.
- image.yml boots three times and once with an unreachable controller on pull requests (`FPS_TEST_REPEAT`, `FPS_TEST_NEGATIVE`), and `mk/img.mk` lints the two new image test scripts.
- [Testing](../Testing.md) names the flake findings.
- [Home](../Home.md) has the batch 26 status line.

The catalog gains:
- CTL-40: the first response frame of an agent session.
- CTL-41: realism reports attached to a run.
- ENG-26: datagram flows in multi flow instances and data ports without repeats.
- AGT-20: SIP with RTP in the agent.
- ENG-27: dropped counts of a capture.
- RLM-15: packet inter arrival times inside a session.
- RLM-16: deliver the realism report to the controller.
- UI-09: reservation holders in the agents view.
- QA-09: the single http1 parser error of the Gate 1 run.

One question is open for the owner (CON-22): checks without metrics (parser errors, encryption, content, session structure, mix accuracy, transport behavior) add no RealismResult, so RunReport.realism does not show them.

### Shared page notes

None.
