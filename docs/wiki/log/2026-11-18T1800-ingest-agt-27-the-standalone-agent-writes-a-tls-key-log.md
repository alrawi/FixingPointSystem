## [2026-11-18] ingest | AGT-27 the standalone agent writes a TLS key log

AGT-27 adds `fps-agent run --keylog FILE`, which writes the TLS secrets of a standalone run in the NSS key log format. It edits only `engine/fps-agent/` (`src/run.rs`, `src/main.rs`, `tests/standalone.rs`), `docs/handoff/AGT-27.md`, this entry, and the Agent page (section "Standalone mode").

- The change is the diff that TRN-12 built in a scratch copy, applied to `origin/main` at 8d4ddac (AGT-26, PR 673, had not merged, so nothing moved). `RunOptions` gets `keylog: Option<PathBuf>` (default none). `run` starts the `KeyTap` before `prepare_with` when the option is set, and writes the lines when the load ends.
- Two changes to the TRN-12 diff. The file is created before the run starts, with mode 0600 (set on a file that exists, too), because it holds the secrets of every session, and a path that cannot be written ends the command with exit code 2 before any load. The agent writes the lines into that open file when the run ends and logs the byte count, with "(truncated)" when the 32 MiB limit of the tap cut them.
- `tests/standalone.rs` has two tests. One runs the `https_fetch` scenario of http1 with `keylog` set, finds `CLIENT_TRAFFIC_SECRET_0` lines in the NSS format (label, 64 hex digits, secret), checks that old content of a file with mode 0644 is replaced, and checks mode 0600. The other checks that an unwritable path ends the run with an output error before the run writes any line.

Evidence (sandbox, root, kernel 6.18, debug build, nothing changed in `http1-afxdp.sh` or the workflow): `engine/fps-datapath/realism/http1-afxdp.sh target/debug/fps-agent OUT` found `--keylog` in `run --help` and wrote 70,892 bytes of secrets. The report has `passed: true` with conformance, parser errors (tshark), and encryption. `realism decrypted`: 68 of 68 TLS sessions decrypt and ndpi classifies the view as HTTP (`https_decrypted` 68/68), 0 parser errors. Not verified here: the Zeek parser (not installed) and the hosted runner.

### Shared page notes

- Transport page (not an AGT page, the coordinator applies it): in the section "The http1 module on AF_XDP, end to end (TRN-12)", replace the sentence about the missing key log flag with "`fps-agent run --keylog FILE` writes the secrets, and the script injects them into the capture".
- Testing page, optional: `fps-agent run --keylog FILE` is how any realism run through a standalone agent gets decryptable captures.
- Module development: no change. The flag changes no module interface.
- `Cargo.lock`, `Cargo.toml`, workflows, `mk/`: unchanged.
