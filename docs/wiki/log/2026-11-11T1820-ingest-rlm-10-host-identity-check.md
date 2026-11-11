## [2026-11-11] ingest | RLM-10 Host identity check

The realism runner has a host identity check (`realism/hosts.py`, report format 0.13.0) that counts the hosts of a capture and reports address conflicts. A host is an IP address per VLAN that is the source or destination of an IP packet or the sender of an ARP packet, and a conflict is an IP address that sends from more than one Ethernet source address in one VLAN, so a capture with two MAC addresses for one IP address fails with `address conflict: 10.0.0.1 sends from 2 MAC addresses (...)`. The check runs when the labels have a `hosts` section (`count` as a number or a range, and `gateway_macs` for routers), or with `--host-identity` or `--host-count N`. It needs no external tool. The evidence is `docs/handoff/RLM-10.md` and `realism/tests/test_hosts.py` (35 tests). We described it in [Realism](Realism.md#host-identity-check) and in `realism/README.md`.

Decision: the entry leaves `metrics` empty, as the messages check does, because the metric catalog 0.5.0 has no value `host_identity` for the label `check`.

### Shared page notes

- The controller embeds a byte copy of the report schema (`controller/internal/report/schema/realism-report.schema.json`, checked by a test): the coordinator refreshes it with `cp realism/schema/realism-report.schema.json controller/internal/report/schema/realism-report.schema.json`.
- [Contracts](Contracts.md): a CON follow up should add `host_identity` (and `messages`) to the values of the label `check` of the metric catalog, so that `realism.check_failures` covers them. Then an RLM change removes both from the exclusion in the schema and from the `metrics` exceptions.
- [Testing](Testing.md): the realism runner list gains `realism/tests/test_hosts.py`, which needs no tool.
- [Gates](Gates.md): the Gate 3 criterion "host identity check passes for 20,000 hosts on one port" can use `python3 -m realism score --host-count 20000`.
- [Home](Home.md) and [Implementation Plan](Implementation-Plan.md): none. [Realism](Realism.md) is updated.
