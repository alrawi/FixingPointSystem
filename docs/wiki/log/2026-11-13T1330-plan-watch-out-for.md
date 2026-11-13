## [2026-11-13] plan | Watch out for, in the module guide

The owner asked on 2026-11-13 that every problem the project resolved while it built modules be documented for future module authors, starting with the listener race of the strike module.

[Module Development](../Module-Development.md) gains a section "Watch out for" with 31 entries in seven groups:
- listeners, ports, and spawned flows;
- timing and pacing;
- classifiers and parsers;
- TLS;
- captures and the harness;
- byte estimates and sessions;
- native code.

Each entry gives what you see, why it happens, and what to do, and names the package whose log entry or handoff holds the evidence. The sources are the batch findings of [Testing](../Testing.md) and the log entries of the MOD, ENG, TRN, AGT, and RLM packages. The checklist of "Before you publish" points at the section.

[Coding Standards](../Coding-Standards.md) extends the module documentation rule. A package that resolves a problem that a module author could meet adds an entry to the section in the same pull request. The worker brief carries the same rule.

### Shared page notes

None.
