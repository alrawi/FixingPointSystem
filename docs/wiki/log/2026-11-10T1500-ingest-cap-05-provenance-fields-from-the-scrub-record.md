## [2026-11-10] ingest | CAP-05 provenance fields from the scrub record

CAP-05 (issue #393) makes the capture tools write the provenance fields that CON-20 added to module manifest schema v0.4.

- `capture/scrub.py`: `provenance_fields` builds `capture_sha256`, `scrub_rules` and `scrubbed_at` from an intact scrub record. `check_record` now also refuses a record whose capture digest differs from `source.sha256` of the document, and a record with a malformed digest, date or rule list.
- `capture/scenario.py`: the manifest provenance of a scrubbed capture carries the three fields. The `notes` sentence keeps only the scrubber version, the digest of the sessions file and the payload byte change.
- A record that fails the check gives `scrubbed: false`, no provenance fields and a warning, so the contract validator rejects the module as before. `capture profile` reports such a record as invalid.
- The test scrubs the committed http1 fixture and validates the manifest against schema v0.4 with literal values.

[Capture](../Capture.md) describes the fields and the check.

### Shared page notes

None.
