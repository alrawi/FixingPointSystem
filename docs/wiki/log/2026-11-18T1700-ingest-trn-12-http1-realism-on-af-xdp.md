## [2026-11-18] ingest | TRN-12 user space TCP over AF_XDP, end to end

TRN-12 runs the http1 module through the agent on the AF_XDP backend and scores the capture with the realism runner. It edits only `engine/fps-datapath/realism/` (new), `engine/fps-datapath/results/sandbox-2vcpu-trn12/` (new), `docs/handoff/TRN-12.md`, this entry, and the Transport page. It changes no Rust code: the end to end run found nothing to fix in `engine/fps-backend-afxdp/` or `engine/fps-datapath/`, and one gap in the agent (the TLS key log of a standalone run), which lies outside the paths and is described below with its exact change.

- `engine/fps-datapath/realism/http1-afxdp.sh AGENT_BINARY OUTPUT_DIR` (root) is the run. `make_plan.py` turns the ten scenarios of `modules/http1` into one plan with the backend `BACKEND_AF_XDP` and ten mix entries (the plain scenarios on the ports 8080 to 8085, because two mix entries cannot listen on one port of one server host, and the https scenarios on 8443 to 8446 as their manifests say), stages the module tree that the plan digests, and writes the labels from `modules/http1/fixtures/expected.yaml` with the plain ports. The script builds a wire for the capture, runs `fps-agent run` in it, converts the capture to pcapng, and scores it with `realism score` and `realism decrypted`.
- The wire: the agent needs both arms of the datapath in one process, so the agent namespace holds the veth ends `dpc` and `dps`, and their peers `wc` and `ws` sit in a second namespace behind a bridge. AF_XDP takes the frames of `dpc` and `dps` before any tap (a capture on either end sees nothing), so `tcpdump` on `wc` records both directions, as the capture on the router's client arm does in the kernel run. The `client` and `server` namespaces of `tests/harness/netns.sh` cannot carry this run: the agent starts AF_XDP only with `--side both`, and a run through the harness router needs next hop resolution in the agent (TRN-35 and AGT-26).
- The score is the score of the kernel run: conformance (nDPI, at least 99 percent), parser errors (tshark; Zeek in CI), and encryption (decryption of every sampled https session and the JA4 of the four browser profiles), then the HTTP inside TLS on the decrypted view. Without the TLS secrets the capture cannot be decrypted, so the script leaves the `tls` section out of the labels when the agent has no `run --keylog`, and prints a notice.
- `test_make_plan.py` (4 tests, no root) checks the plan, the digest of the staged tree, and the labels.

Evidence (sandbox, 2 vCPU, root, kernel 6.18, debug build, `engine/fps-datapath/results/sandbox-2vcpu-trn12/`):

- With the stock agent: 199 client and 199 server sessions attempted and succeeded, none failed, about 3,300 frames through the lane, both ports `XDP_MODE_NATIVE` in copy mode. Conformance and parser errors pass; the encryption check fails with "the capture holds no embedded keys" while JA4 matches 68 of 68 sessions. A run with `FPS_AFXDP_SESSIONS=100` (1,027 sessions, 24 seconds) passed the same checks.
- With the agent change below (built in a scratch copy outside the repository): conformance, parser errors, and encryption pass, 68 of 68 TLS sessions decrypt with matching JA4, and `realism decrypted` classifies the view as HTTP 68 of 68 with no parser error. The result files are from this run.
- Not verified here: the Zeek parser (not installed in the sandbox) and the hosted runner. The coordinator's job runs both.

Findings:

- A standalone `fps-agent run` never exports the TLS secrets of its run: `KeyTap` keeps lines only while a capture that the controller started asks for them. A realism run of a module with https scenarios through a standalone agent on any backend therefore cannot decrypt. The change below adds `run --keylog FILE`.
- The own stack numbers its client ports upward from 10000, so every 5-tuple of the capture is unique and the capture has no reused port for Zeek to report (the kernel fixture peer needs its own port allocation for that, MOD-21).
- The ten mix entries share one server host and listen at once, which the hub of AGT-25 serves (one datapath listener for each port, sessions routed by local address).

### Shared page notes

**Agent change (outside the paths of TRN-12; an AGT package or the coordinator applies it).** `fps-agent run --keylog FILE` writes the TLS secrets of the run in the NSS key log format. The diff applies to `engine/fps-agent/` at `origin/main` (1e724cb) and was built and run in a scratch copy. It needs a test (`tests/standalone.rs` can run an https scenario with `keylog` set and check that the file holds `CLIENT_TRAFFIC_SECRET_0` lines) and a line in `docs/wiki/Agent.md` for the `run` command. The script starts using the flag by itself, because it looks for `--keylog` in the output of `run --help`.

```diff
--- engine/fps-agent/src/run.rs
+++ engine/fps-agent/src/run.rs
@@ -66,6 +66,10 @@
     /// What the agent requires of the module signatures. The default asks for
     /// none.
     pub policy: ModulePolicy,
+    /// A file that receives the TLS secrets of the run in the NSS key log
+    /// format, so that a capture of the run can be decrypted. The default is
+    /// none, and the secrets then stay in memory.
+    pub keylog: Option<PathBuf>,
 }
 
 impl RunOptions {
@@ -79,6 +83,7 @@
             interval: Duration::from_secs(1),
             linger: Duration::from_secs(5),
             policy: ModulePolicy::default(),
+            keylog: None,
         }
     }
 }
@@ -750,7 +755,11 @@
     log: &mut dyn Write,
 ) -> Result<RunSummary, RunError> {
     let plan = Plan::load(&opts.plan)?;
-    let prepared = prepare(plan, &opts.modules, opts.sides, &opts.policy)?;
+    let keys = KeyTap::new();
+    if opts.keylog.is_some() {
+        keys.begin();
+    }
+    let prepared = prepare_with(plan, &opts.modules, opts.sides, &opts.policy, &keys)?;
     for warning in prepared.warnings() {
         writeln!(log, "fps-agent: warning: {warning}")?;
     }
@@ -795,6 +804,17 @@
     }
     let phase_end = active.phase_at(active.elapsed());
     let finished = active.finish(SERVER_RAMP_DOWN)?;
+    if let Some(path) = &opts.keylog {
+        let lines = keys.end();
+        std::fs::write(path, &lines.text)?;
+        writeln!(
+            log,
+            "fps-agent: wrote {} bytes of TLS secrets to {}{}",
+            lines.text.len(),
+            path.display(),
+            if lines.truncated { " (truncated)" } else { "" }
+        )?;
+    }
     printer.line(&finished.shown, phase_end, true)?;
     let lines = printer.sequence;
 
--- engine/fps-agent/src/main.rs
+++ engine/fps-agent/src/main.rs
@@ -41,7 +41,7 @@
 use fps_backend_afxdp::BackendKind;
 
 const USAGE: &str = "usage: fps-agent capabilities [--root DIR]
-       fps-agent run --plan FILE --modules DIR [--side both|client|server] [--linger SECONDS] [--module-trust-store FILE] [--dev-modules]
+       fps-agent run --plan FILE --modules DIR [--side both|client|server] [--linger SECONDS] [--keylog FILE] [--module-trust-store FILE] [--dev-modules]
        fps-agent connect --controller URI --agent-id ID --modules DIR [--root DIR] [--side both|client|server] [--backend kernel|af_xdp] [--label KEY=VALUE]... [--redial-limit SECONDS] [--capture-dir DIR] [--capture-retention SECONDS]
                           [--module-trust-store FILE] [--dev-modules]
                           [--cert-dir DIR [--tls-ca FILE] [--tls-ca-sha256 HEX] [--enroll-address HOST:PORT]
@@ -419,6 +419,7 @@
 fn parse_run_args(args: &[String]) -> Result<RunOptions, String> {
     let (mut plan, mut modules, mut sides, mut linger) = (None, None, Sides::Both, None);
     let mut signing = SigningFlags::default();
+    let mut keylog = None;
     let mut it = args.iter();
     while let Some(flag) = it.next() {
         if flag == "--dev-modules" {
@@ -430,6 +431,7 @@
             "--module-trust-store" => signing.store = Some(PathBuf::from(value)),
             "--plan" => plan = Some(PathBuf::from(value)),
             "--modules" => modules = Some(PathBuf::from(value)),
+            "--keylog" => keylog = Some(PathBuf::from(value)),
             "--side" => {
                 sides = Sides::parse(value).ok_or_else(|| format!("unknown side {value:?}"))?;
             }
@@ -450,6 +452,7 @@
     );
     opts.sides = sides;
     opts.policy = signing.policy()?;
+    opts.keylog = keylog;
     if let Some(l) = linger {
         opts.linger = l;
     }
```

**CI job.** Add this job to `.github/workflows/mod-http1.yml` (its path filters already hold `engine/**`, `realism/**`, and `modules/http1/**`). It runs on the hosted runner, uses root through `sudo`, and installs tools only through `realism/ci/install-tools.sh`; the agent is built as the runner user and run as root. Before the agent change merges, the script prints a notice and scores conformance and parser errors only; after it, the encryption check and the decrypted view are scored too, and the job needs no edit.

```yaml
  afxdp:
    runs-on: ubuntu-24.04
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Install the harness and realism tools
        run: realism/ci/install-tools.sh
      - run: cargo build -p fps-agent
      - name: Record the http1 module on the AF_XDP backend and score it
        run: |
          sudo env "PATH=/opt/zeek/bin:$PATH" FPS_AFXDP_PARSERS=tshark,zeek \
            engine/fps-datapath/realism/http1-afxdp.sh "$PWD/target/debug/fps-agent" "$RUNNER_TEMP/http1-afxdp"
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: http1-afxdp
          path: ${{ runner.temp }}/http1-afxdp
          if-no-files-found: ignore
```

The same run fits the `modules` job of `qa-gate2.yml` as a second pass for http1, once the job above is green: `sudo env "PATH=/opt/zeek/bin:$PATH" FPS_AFXDP_PARSERS=tshark,zeek engine/fps-datapath/realism/http1-afxdp.sh target/debug/fps-agent gate2-out/http1-afxdp`. The path filter of `trn.yml` already holds `engine/fps-datapath/**`.

- `mk/trn.mk` (lint and test targets, optional): `shellcheck engine/fps-datapath/realism/*.sh`, `ruff format --check`, `ruff check`, and `mypy --strict` on `engine/fps-datapath/realism/*.py`, and `$(PYTHON) -m pytest -q engine/fps-datapath/realism` under `test-trn`. The tests need PyYAML and add the repository root to the import path themselves.
- Testing page: the realism run of http1 on AF_XDP is `sudo engine/fps-datapath/realism/http1-afxdp.sh target/debug/fps-agent OUT`. It needs ip, ethtool, tcpdump, editcap, ndpiReader, tshark, and PyYAML (Zeek for the Zeek parser), and takes about 15 seconds.
- Transport page: done in this package (section "The http1 module on AF_XDP, end to end (TRN-12)"). After the agent change merges, replace its sentence about the missing key log flag with "`fps-agent run --keylog FILE` writes the secrets, and the script injects them into the capture".
- Module development, "Watch out for" (exact entry): "**Two mix entries cannot share one server port.** A plan listens once for each port of each server host, so two scenarios of one module that both listen on 8080 fail with an address in use error when a plan runs them as two mix entries, although a fixture peer that runs the scenarios one after another meets no clash. Give each mix entry its own `server_port` parameter, as `http1-afxdp.sh` does for the plain http1 scenarios (TRN-12)."
- TRN-31: nothing in this script changes with the flip, because the plan names the backend.
- `Cargo.lock`, `Cargo.toml`, workflows, `mk/`: unchanged.
