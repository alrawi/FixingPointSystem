# Agent

This page describes the agent of the AGT workstream. The crate [`fps-agent`](https://github.com/alrawi/FixingPointSystem/tree/main/engine/fps-agent) holds the agent binary. AGT-03 added capability detection, which [Implementation Plan](Implementation-Plan.md) describes, and AGT-01 adds the standalone mode, which runs a plan without a controller. AGT-02 added the client loop that dials the controller, which the section [Controller session](#controller-session) describes, and AGT-06 added the kernel requirement and the redial policy, which the sections [Kernel requirement](#kernel-requirement) and [Redial policy](#redial-policy) describe. AGT-05 added the handlers of the v0.2 commands `UpdateRates`, `StartCapture`, and `StopCapture`, which the section [Run time rates and captures](#run-time-rates-and-captures) describes. AGT-08 added mutual TLS, enrollment, and renewal, which the section [Mutual TLS and enrollment](#mutual-tls-and-enrollment) describes. AGT-07 applies module settings and reports every session series with the `side` label, which the sections [Module settings](#module-settings), [Snapshot lines](#snapshot-lines), and [Criteria](#criteria) describe. AGT-09 serves the capture download of CON-13, which the section [Capture download](#capture-download) describes. AGT-04 verifies module signatures against a trust store, which the section [Signed modules](#signed-modules) describes. AGT-10 shares one capture cell between the TCP and UDP workers and checks a pinned fingerprint during the enrollment handshake, which the sections [Run time rates and captures](#run-time-rates-and-captures) and [Trust](#trust) describe, and it classifies a close right after the handshake, which the section [Permanent failures](#permanent-failures) describes. AGT-11 loads the WebAssembly extension of a module and wraps the UDP session functions with it, which the section [WebAssembly extensions](#webassembly-extensions) describes. AGT-14 builds the TLS profiles of a run, ships the TLS key log in the capture file, and applies the labels of a criterion, which the sections [TLS profiles](#tls-profiles) and [Criteria](#criteria) describe.

## Standalone mode

The command below runs one plan file and exits with the verdict of its criteria.

```bash
fps-agent run --plan contracts/examples/plans/http_dns_smoke.json \
  --modules contracts/examples/modules [--side both|client|server] [--linger SECONDS] \
  [--module-trust-store FILE] [--dev-modules]
```

| Exit code | Meaning |
|---|---|
| 0 | The run completed and every criterion holds |
| 1 | The run completed and at least one criterion fails |
| 2 | A usage error, an invalid plan, a module or scenario that fails its checks, a feature that the mode does not run, a failed worker, or a kernel before Linux 6.3 |

The run takes eight steps.

1. Read the plan in the canonical JSON mapping of `fps.agent.v1.Plan` from `contracts/README.md`. The reader rejects unknown fields, reads `seed` as a string or a number, reads durations such as `"30s"`, and checks the references between modules, scenarios, mix entries, pools, and workers.
2. Load every module of the plan from `DIR/<name>`. The manifest must carry the plan's name and version, and the tree digest of the directory must equal the plan digest. The digest follows the rule of `contracts/README.md`, so a symbolic link fails the run.
3. Check the digest of every scenario source, compile it in the call scope of its module with `Scenario::parse_instance` of `fps-flow` (AGT-16, which also accepts `spawned` flows and the `spawn` action), apply the mix parameters as variable overrides, and fold constants. An integral JSON number binds as an integer.
4. Register the native template functions of decision D10. A module gets the functions of its crate, `fps-module-http1` or `fps-module-dns`, only when its manifest catalog equals the crate catalog in names, parameter order, types, and required parameters. The run reports every module without native functions on stderr, and calls into such a module fail the session. A module that ships `extension.wasm` is served by its extension instead, see [WebAssembly extensions](#webassembly-extensions).
5. Turn every plan worker into engine workers, as the next section describes.
6. Start the server workers, then the client workers. TCP flows run on `fps-runtime`. UDP flows run on worker threads of the agent with the datagram openers of `fps-backend-kernel`, with the same session ids, generators, and metric series as the TCP workers.
7. Write one counter snapshot per second to stdout as a JSON line, and a final line with `isFinal` when the run ends.
8. Evaluate the criteria and print the totals of each side, every criterion, and the verdict on stderr.

The standalone mode runs `BACKEND_KERNEL`. Other backends, a scenario with several flows of which one is UDP, and external servers named by host name end the run with exit code 2. A scenario with several flows runs as a scenario instance, see [Scenarios with several flows](#scenarios-with-several-flows).

### From plan workers to engine workers

An engine worker runs one side of one flow, so one plan worker becomes several engine workers. Every rate of a plan worker gives a client worker, and every mix entry whose server pool has a host range on the plan worker gives a server worker. The engine worker id is the plan worker id shifted left by 8 bits, plus the mix index shifted left by 1 bit, plus 1 on the server side. Plans therefore hold at most 256 workers and 128 mix entries. Every engine worker pins to the core of its plan worker, and a pin that fails leaves the thread unpinned.

| Setting | Value |
|---|---|
| Client source addresses | The plan worker's range of the entry's client pool, picked round robin per session |
| Client destinations | The plan worker's range of the entry's server pool, so a session stays on its worker, or the whole slice when the worker owns no range there |
| Client schedule | The rate as `sessions_per_second` or `concurrent_sessions`, the plan phases, Poisson arrivals, and the stream of the plan worker id and mix index |
| Server listen addresses | Every address of the plan worker's range of the server pool, at the constant port of the scenario's `accept` |
| Topology | `TOPOLOGY_APP_SIM` runs both sides, `TOPOLOGY_CLIENT_SIM` sends to the IP addresses of `externalServers`, and `TOPOLOGY_SERVER_SIM` runs only servers |

Two mix entries that listen on the same transport, address, and port fail the run, because the server could not tell their sessions apart.

The agent does not configure addresses. Every pool address that a worker binds must be local to the namespace of the process, for example through a `local` route on the loopback for the whole prefix, which [Hosts](Hosts.md) calls AnyIP. Port setup with `fps-hosts` moves an interface into a namespace of its own, which needs the process to enter that namespace, and we leave that step to a later package.

### First party modules

First party modules implement their template functions in Rust (decision D10), and the agent links their crates. One registry in `modules.rs` (AGT-13) maps a module name to the function catalog of its crate and a constructor. The registry holds `http1`, `dns`, `content`, `smtp`, `imap`, `pop3`, `ftp`, `smb2` (AGT-19), `http2`, and `ssh`. A module whose native functions are a unit value and take no settings needs one line, `stateless!(fps_module_x, X)`, and a dependency in `Cargo.toml`. A module that reads settings, as `dns` does, or whose value holds state, as `http2` does (one HPACK encoder and decoder per connection side, copied for every session) and `ssh` does (AGT-17: the keys and packet counters of one SSH session), gets a constructor function of its own. `fps_module_ssh::Ssh` is not `Clone`, because a copy would carry the keys of one session into another, so the registry holds a wrapper whose clone is a fresh `Ssh`, and every session side starts a new key exchange from nothing. `NativeFunctions` copies the registered functions per session and routes each call by its module name.

`NativeFunctions::for_modules` registers a module only when the catalog of its manifest equals the catalog of its crate in names, parameter order, types, and required parameters. A module that the registry does not know, or whose manifest differs, stays without native functions and the run notes it, unless its WebAssembly extension serves it. Settings for such a module end the run, extension or not.

The plan lists the modules of its scenarios and nothing more, so an smtp plan names smtp and not `content`, whose functions the smtp scenarios call through `requires`. The registry therefore also registers every module that a plan module requires and the plan omits, when the registry knows it, with no settings. When the manifest of that module is installed in the modules directory, its catalog must equal the crate catalog, as for a listed module. The run notes `module content, which smtp requires, runs the native functions of this agent`.

The `ftp` scenarios run since AGT-16, because each has a control flow that spawns its data flows and the agent runs such scenarios as instances. The manifest of `http2` merged with MOD-06, so the unit tests of the registry now require a manifest for every registered module and keep no exception.

The agent reads each module tree once. The digest pass keeps the bytes of `manifest.yaml` and `extension.wasm` of the tree, and the agent parses and compiles those bytes (AGT-17 for the manifest, AGT-16 for the extension). A manifest that a writer swaps after the digest matched is never parsed, and one swapped during the pass either fails the digest or is the digested one, so the catalog that the agent compiles the scenarios against is the one that the plan digest and its signature name. `modules::installed`, which reports the modules to the controller, takes the name, version, and digest from one pass as well.

### Scenarios with several flows

A scenario with several flows runs as a scenario instance of `fps-runtime` (ENG-23). The agent compiles every scenario with `Scenario::parse_instance`, so a `spawned` flow and the `spawn` action load, and it assembles the workers as for a scenario with one flow: the entry flow, which is the first flow, gives the L4 protocol, the accept port, and the hosts, and the worker runs the other flows inside the instance. The server workers bind the data ports themselves, and `ExternalServer.port` of a client simulator topology replaces the port of the control connection only. `assemble::check_flows` refuses at prepare a scenario with several flows of which any is UDP (`UNIMPLEMENTED`, with the flow named), because the instance runner opens TCP connections only, and a scenario whose entry flow is `spawned`, which no client could start. A scenario with one UDP flow runs on the UDP workers as before.

`FlowRef::port` carries the name of the test port that a side runs on, which the runtime puts in the `port` label of the `tls.*` series: a client worker uses the port of its client pool, and a server worker the port of its server pool. A pool without a port name leaves the label at `default`.

### Module settings

`ModuleRef.settings` (CON-11) holds `modules.<name>.settings` of the test configuration, which the controller validated against the settings schema of the manifest. The agent applies them when it loads the plan, in `modules::NativeFunctions::for_modules`, through the registry above. The `dns` module reads `zone_file` (MOD-19): a relative path resolves against the module directory `<modules dir>/dns`, and the server answers from that zone instead of the shipped one.

A setting path may not leave the module directory. The contract does not allow paths outside it, so the agent refuses a path with a `..` component and any path whose canonical form (symbolic links resolved) lies outside the module directory, absolute paths included. An absolute path that names a file inside the module directory works. The failure ends `prepare` with `InvalidArgument` and a message such as `module dns: settings: zone_file: "../x.zone" has a .. component, which could leave the module directory`. A missing file, a zone with a syntax error, an unknown key, and a value of the wrong type end `prepare` the same way.

The agent refuses settings that it cannot apply, and never skips them in silence, because a run that ignores them measures another test. That covers a module that has no native functions in this agent, such as a module whose manifest catalog differs from its crate, and a module whose native functions take no settings, such as `http1`. A module without settings behaves as before.

### Signed modules

AGT-04 makes the agent verify the signature of every module before it loads the module, with the check that the controller applies at install (CTL-11, CTL-28, [Controller](Controller.md#signature-checks-at-install-and-in-plans)). The code is `src/trust.rs`, and `modules::load` calls it for each module of a plan, in the standalone mode and for a `prepare` command alike.

```
fps-agent run     ... [--module-trust-store FILE] [--dev-modules]
fps-agent connect ... [--module-trust-store FILE] [--dev-modules]
```

**What is signed.** The signature of a module is an Ed25519 signature over the UTF-8 bytes of its tree digest, the `sha256:` string of rule 1 of `contracts/README.md`. The plan carries it in `ModuleRef.signature` (base64 in the canonical JSON) and `ModuleRef.signature_key_id` (CON-05). The agent computes the tree digest of the module directory with `modules::tree_digest`, which is the same rule as `registry.TreeDigest` and `signing.ArchiveDigest` of the controller, and requires it to equal the digest of the plan. The signature then covers the bytes that the agent runs. The agent never reads an archive, because the controller installs archives and the agent reads trees under `--modules`. A key id is `ed25519:` and the hex of the first 16 bytes of the SHA-256 of the public key. The signature check is Ed25519 of `ring`, which the lock already held through rustls, so the agent adds no package.

**The trust store.** `--module-trust-store FILE` names a file in the format of the controller's `trust_store_file`: `version` 1 and a list of keys with `key_id`, `public_key` (base64), `status`, and for a retired key `retired_after` (RFC 3339). The parser refuses what the Go parser refuses, which are unknown fields, trailing data, a key id that its key does not derive, a retired key without a cutoff, a cutoff on another standing, and a key twice. The agent reads the file at startup, so a malformed file ends the process with exit code 2 before it dials. A rotation that adds a key means copying the new file to the agents and restarting them. An active key verifies at any time, a retired key strictly before its cutoff, and a revoked key never. The checks run in the order of the Go verifier, so the error names the first fault: no signature, a signature that is not 64 bytes, an unknown key, a revoked key, a retired key past its cutoff, and last the signature.

| Case | Without `--dev-modules` | With `--dev-modules` |
|---|---|---|
| Valid signature of an active key, or of a retired key before its cutoff | Loads | Loads, no warning |
| No signature, a signature of the wrong length, an unknown key, a revoked key, a retired key past its cutoff, or a signature that does not verify | `prepare` fails with `ERROR_CODE_FAILED_PRECONDITION`, and `run` ends with exit code 2 | Loads, and a `warn` log line names the module and the reason |
| Tree digest differs from the plan digest | Fails | Fails, because the digest check of AGT-02 is not a signature check |
| No `--module-trust-store` | Loads and ignores any signature, with one warning that signatures are not required | The same |

With no trust store the agent behaves like a controller without `trust_store_file`: it requires no signature and checks none, and it logs one warning per plan (`no --module-trust-store: ...`). A signature that comes with a module cannot be checked without keys, so the agent ignores it. This default keeps the example modules and the controller end to end test working, which install unsigned modules. A fleet that wants signatures sets the flag on every agent. `--dev-modules` is for developers: with a trust store it lets a module through that fails the signature check and warns for each, and it changes nothing when there is no trust store.

The plan JSON reader of the standalone mode accepts `signature` and `signatureKeyId` on a module. It read neither before, and its strict reader refused a plan that carried them.

**Tests.** `tests/signing.rs` runs the module loader on the example modules. The golden test builds a small tree, requires the tree digest that Python computed from rule 1, and signs it with the key of seed 0x07, the key of the CTL-11 vectors, and requires the signature and key id of Python and of `golden_test.go`. The other tests cover a valid signature, a changed tree (digest error in both modes), a tree whose plan digest was rewritten (bad signature, accepted with a warning in developer mode), an unknown key, an unsigned module, a revoked key, a retired key on both sides of its cutoff with an injected clock, a plan in the canonical JSON with base64 signatures, and the default without a trust store. The unit tests of `src/trust.rs` pin the key ids and the signature of the vectors, the order of the checks, and the parser rules.

### WebAssembly extensions

AGT-11 runs the `extension.wasm` of a module (ENG-11, ENG-17, [Engine](Engine.md#webassembly-extensions-in-sessions)) in the agent. `modules::load` does it for every module of a plan, in the standalone mode and for a `prepare` command alike.

**Order.** The extension is a file of the module tree, so the tree digest that must equal the plan digest, and the signature of AGT-04 over that digest, cover its bytes. `modules::load` checks the digest and the signature of a module first, and only then compiles its extension. A changed `extension.wasm` therefore fails the digest check before anything compiles it, and a module whose signature fails (without `--dev-modules`) is never compiled.

**One read (AGT-16).** The pass that digests the module tree reads `extension.wasm` once and keeps its bytes, and `ExtensionSet::load_module_bytes` compiles those bytes. The file is not read a second time, so a writer that replaces it after the digest check changes nothing that runs. Before AGT-16 the agent called `ExtensionSet::load_module_dir`, which read the file again after the digest. `tests/extensions.rs` swaps the file inside the hook of `modules::load_with`, between the digest and the compile, and the plan still loads with the digested extension. The manifest is still read by its own call before the digest pass, which the same flaw would apply to, and it could close it by parsing the manifest from the bytes that the digest pass kept.

**Cost.** A module without `extension.wasm` costs one stat call. The agent creates the WebAssembly engine at the first extension, so a plan without extensions starts none, and `LoadedModules::extensions` is `None`. With an extension, `assemble` hands one shared `Arc<ExtensionSet>` to `FlowRef::extensions` of every flow. Each session instantiates its own copy at its first call of the module, with the default `fps_wasm::Limits` (20 million fuel units per hook call, 16 MiB of memory).

**Refusal.** An extension that does not compile, or whose catalog differs from the manifest in a function name, a parameter name, order, type, or required flag, ends `prepare` with `ERROR_CODE_FAILED_PRECONDITION`, and `run` with exit code 2. The message names the module and the first difference, such as `module xmod: extension catalog differs from the manifest: the manifest has function 'fail'`. The agent does not run a plan without the extension, because it would measure another test. The run lists a module that runs an extension in its notes, in place of the note that the module has no native functions. If a module has native functions and an extension, the extension serves its calls.

**Sessions.** The TCP workers use the wrapper of `fps-runtime`. The UDP workers of the agent build it the same way in `udp.rs`: `ExtensionSet::session_functions` wraps the native functions with a `SessionKey` of the run seed, the global session id, the side, the scenario, and the flow, and `finish` ends the extension sessions with the `on-close` hook after the outcome is recorded, with the `CloseReason` that the outcome maps to. A looping, trapping, or failing extension ends only its own session with the failure reason `extension_error`, and other sessions on the same thread and the same compiled component go on. A flow without an extension set takes no wrapper.

**Limits of the first version.** The interpreter has no seam for `send`, timers, or `on-data` (ENG-17), so an extension serves template functions (`render`) and `on-close` only. A UDP session reports the transport `tcp` to `on-open`, because `SessionExtensions` fixes it. The limits are the defaults, and no plan setting changes them.

### Snapshot lines

Each line is the proto3 JSON mapping of an aggregate `CounterSnapshot` with `planId`, `sequence` from 1, `time`, the `phase` at that moment, and the counters, gauges, and histograms of every side that the process runs. Catalog 0.2.0 gives every scenario and transport series the `side` label, so the client series and the server series of one session sit side by side in the line and do not add up. The UDP workers of the agent register their series through `fps_stats::SessionCounters`, as the TCP workers of `fps-runtime` do, so both transports label their series alike. A process that runs one side only, as with `--side client`, carries that side. A sum over every label of one line counts each session of a two sided process twice, so a reader selects `side` first. The counters are cumulative from the start of the run.

### Sides and the end of a run

`--side client` and `--side server` restrict a process to one side, so a test can run the two sides in two network namespaces. A process with client workers ends when every schedule ends and every session closes, and it resets client sessions that are still open 30 seconds after the load phases. A process with server workers only stops them when the load phases end plus the linger time, five seconds by default. Server workers then let open sessions finish within two seconds.

### Criteria

A criterion has a metric, an operator, a threshold, and since AGT-14 `labels`, which select the series that it counts, as the `where` field of a test criterion does (CON-15 added the field to the wire criterion, and the controller copies `where` into it). A series counts when it carries every label of the filter with that value. Each key must be a label that the metric catalog lists for the metric, so a filter on `port` for `sessions.succeeded`, or a `side` that is neither `client` nor `server`, ends the run with a rule error that names the criterion. A histogram statistic such as `sessions.duration.p99` takes the labels of its histogram, and a derived ratio applies the filter to both of its counters.

The `side` label picks the side of the process. Without it a criterion counts the client side, as CON-08 asks, because a total over both sides counts each session twice, and a process that runs no client workers counts the server side. A metric without the `side` label counts as it is. The other labels (`application`, `scenario`, `reason`) aggregate when a criterion does not name them. Counters and histogram statistics count from the end of the warmup, which the agent marks with a baseline snapshot at the first tick after `warmup`, and the baseline subtracts per filter. Gauges take their final value. The derived ratios `sessions.failed_ratio` and `tls.resumption_ratio` divide their counters as the catalog formula says. A ratio over zero sessions is undefined and fails its criterion. A histogram reference needs one statistic. A gauge or histogram that the agent does not record ends the run with exit code 2. Before AGT-14 the agent refused a plan with a labeled criterion, because a silent skip would measure another rule, and the refusal is gone.

In the controller mode the controller decides the verdict from the snapshots, so the agent only needs to accept the plan. The plan JSON reader of the standalone mode takes `labels` on a criterion as a string map.

### TLS profiles

A scenario upgrades a connection with `tls_upgrade`, which names a profile (ENG-20). Since AGT-14 the agent builds the profiles that the scenarios of the mix name, sets `RuntimeConfig::tls` on every TCP worker, and sends the secrets of every handshake to a key tap. The module `fps_agent::tls` holds the code. `run::prepare` scans the compiled scenarios of the mix, both sides of every flow, and refuses the plan when an action names a profile that the table of its role lacks (`InvalidArgument` with the scenario, the role, the name, and the names that exist). A plan without `tls_upgrade` builds nothing.

| Role | Profile names | What they are |
|---|---|---|
| client | `chrome-136`, `firefox-139`, `safari-18.5`, `edge-134` | The browser profiles of the TLS module (TRN-05), with the JA4 that `modules/tls` documents |
| client | `mail-client` | The ClientHello of `firefox-139` without the ALPN extension, as Thunderbird sends it on the mail ports (AGT-16; its JA4 is not compared with a real Thunderbird) |
| server | `server-tls13`, `server-tls12` | One version each, the ALPN id `http/1.1`, a certificate for `www.example.test`, `localhost`, and every server name that an action of the plan sends, with stateless tickets |
| server | `server-tls13-h2` | TLS 1.3, selects `h2`, and `http/1.1` for a client that offers only that, with the same certificate |
| server | `server-tls13-noalpn` | TLS 1.3, selects no ALPN id, with the same certificate |

`modules/http1/tests/common/tls.rs` is the reference registry, and the agent follows it. The server name of a client action that names none is `localhost`, which `fps-runtime` fixes, and the certificate carries it. The names are DNS names. The profile tables are small on purpose: a package that needs another profile adds it to the registry of the TLS module (`fps_module_tls::browser` or `fps_module_tls::mail`) and to the name lists of `tls::client_profiles` and `tls::server_profiles`. Since AGT-17 `mail-client`, `server-tls13-h2`, and `server-tls13-noalpn` are lookups of the TLS module registry (`mail::mail_client` and `mail::server_profile`) through `tls::client_profile` and `tls::server_profile`, and the agent keeps no copy; only `server-tls13` and `server-tls12`, which the module does not register, are defined in the agent. The module's server profiles disable resumption, because a connector that several sessions share must not resume. The agent builds one connector per session, so `tls::server_profile` sets the resumption of every server profile back to stateless tickets, as the agent always had. A unit test compares every field of the three profiles with the module's.

**Trust between agents.** The client and the server side of a run are often different agents, and the only channel between them is the plan, so no agent can hand its certificate to the other. Both derive the authority of the run from the run seed: the P-256 key of the authority is the SHA-256 of a fixed label and the seed, reduced to a valid scalar, and its public point is computed with `num-bigint` (`tls::authority`). `RunAuthority::from_pkcs8_der` (added to the TLS module by AGT-14) builds the authority from that key. The server side issues its leaf certificate from its authority, and the client side trusts its own authority built the same way. The two self signed certificates differ in the bytes of the ECDSA signature, but they share the subject and the key, which is all that a trust anchor needs, and `tests/tls.rs` runs a client process and a server process, and two registries in memory, to show it. A registry built from another seed refuses the handshake. The key is a function of the seed, so anyone who knows the seed (the plan carries it) can sign a certificate that the clients of the run accept. The roots exist only inside the connectors of the engine, never in a system store, and the traffic is synthetic, so the agent accepts this. A run that must resist an attacker on the lab network needs a plan that carries a certificate, which is a contract change.

**One connector per session.** MOD-04 found that a connector with a resumption store changes its JA4 from the second handshake on, because the ClientHello then carries `pre_shared_key`. Since AGT-16 the agent registers every client profile with `TlsProfiles::with_client_factory` (ENG-24), so each session builds its own connector with an empty store, never resumes, and sends the ClientHello of its profile against any server. The server profiles issue tickets again (the default of `ServerProfile`), because nothing resumes. `tls::build` makes one connector per profile first, so a profile that cannot be built still refuses the plan at prepare. Before AGT-16 the registry shared one connector and the server profiles issued no tickets to keep it from resuming, which a third party server that issues tickets would have defeated. `tests/tls.rs` sends three handshakes through one registry against a server that issues tickets and compares the extension sets of the ClientHellos.

**The key log in the capture.** The profiles write their secrets to a `KeyTap`. The tap keeps lines only while a capture that was started with `export_tls_keys` runs, so a run without such a capture holds no secret in memory, and it keeps at most 32 MiB of lines (the agent logs a warning when it cuts them). When that capture ends, the agent joins the ring files and puts one decryption secrets block of pcapng (type `0x0A`, secrets type `TLSK`) right after the interface description of every section, so the lines stand ahead of every packet and the capture file decrypts alone in Wireshark and tshark with no key log beside it. The controller stores one file per agent and the download carries one file, so the embedded block is the least invasive choice. It needs no sidecar, no change to the download protocol, and no change in the controller. The lines of the plan go into every section, which repeats them when a long capture has several ring files. With `export_tls_keys` false the file holds no key at all. The contract lets an agent decide on its own in that case, and the agent chooses to withhold the secrets, because a capture is shared more widely than a run. The capture holds the encrypted records, since the tap sits below the TLS layer, so the lines are the only way to read the session. A key log is as sensitive as a private key, and the capture directory has mode 0700.

`tests/tls.rs` decrypts the captured https sessions with the lines of the file alone, using a decrypter that is built on `ring` and shares no code with the TLS module, checks that the lines stand ahead of the first packet and that wrong digits decrypt nothing, and checks that a capture without `export_tls_keys` has no secrets block. No tshark is needed. The controller e2e job of CI installs no tshark either.

**Limits.** The runtime records the `tls.*` series since ENG-24, so `tls.resumption_ratio` has its inputs, with the `port` label that `FlowRef::port` carries. A capture through the REST API of the controller works since AGT-12, and `tests/controller_e2e.sh` downloads one with the TLS key log in it. UDP flows with `tls_upgrade` are refused by the flow loader (ENG-22).

## Tests

| Level | Test | What it shows |
|---|---|---|
| Unit | `src/*.rs` | Plan parsing, durations, criteria on both sides of a snapshot, tree digests, native function registration with module settings and the path checks, and a UDP client and server over loopback whose series carry the side label |
| Component | `tests/standalone.rs` | The example plan on loopback pools with high ports passes with sessions of both modules on both sides, and every series of a snapshot line carries the `side` label with the client and the server series next to each other. A failed criterion gives fail, a server only run reports the server side, and the criteria count the client side, which is half of a total over both sides. A changed digest, version, or backend is an error. A plan with the `zone_file` setting of a copy of the dns module passes because the zone answers its name, and the same plan without the setting fails. A setting with `..`, a path outside the module directory, or a missing file ends the run |
| Component | `tests/extensions.rs` | A module `xmod` ships the WAT built test extension of `fps-wasm`. A plan with a TCP and a UDP scenario that call it runs through the agent: the good sessions succeed on both sides, and every session of the looping TCP and UDP scenarios fails with `extension_error` and no other reason, on the same threads and the same compiled extension. A module without an extension starts no engine, garbage as `extension.wasm` and a catalog that differs from the manifest refuse the plan with an error that names the module, a changed `extension.wasm` fails the tree digest, and one that is swapped after the digest matched is never compiled (the plan loads with the digested bytes). A `manifest.yaml` swapped after the digest matched is never parsed (AGT-17), a manifest that a writer flips during repeated loads never gives a catalog other than the digested one, and a tree without a manifest is a manifest error |
| Component | `tests/first_party.rs` | A plan with an smtp, an imap, and a pop3 scenario, which lists no `content` module, runs on loopback pools with the real module trees and passes with sessions of the three modules. The mail scenarios take their messages from the content functions, so the sessions fail when the registry lacks `content`. The `ftp` scenario `mixed_session`, with a control flow and three spawned data flows, runs through `run` and gives at least one full instance of four succeeded client sessions. The `ssh` scenarios `interactive_shell` and `scp_upload` (AGT-17) run a real key exchange and an encrypted transport through `run` and give succeeded client sessions, which they do only when every session side has its own `Ssh` value. The three tests take no lock and run in parallel (AGT-18): the ftp data ports of the test are drawn from a range in which the seeded draws never repeat, because two concurrent instances that draw the same data port reset one of the two connections. The ignored tests `repeat_the_ftp_test`, `repeat_the_ssh_test`, and `repeat_the_mail_test` run each test 50 times in a row, side by side (`cargo test -p fps-agent --test first_party -- --ignored repeat`). The unit tests of `modules.rs` resolve every function of every first party manifest through the registry, and the tests of `assemble.rs` cover the check on the flows of a scenario |
| Component | `tests/tls.rs` | The https scenarios of `http1` (TLS 1.3 with chrome-136, TLS 1.2 with safari-18.5) run through the agent and pass with a criterion on the server side, whose value equals the server total and not the client one. A plan that names an unknown profile is refused at prepare. A client process and a server process with one seed trust each other, and another seed is refused. A capture started with `export_tls_keys` decrypts every https session with the key lines of the file alone, and one without has no secrets block. Three handshakes through one registry against a server that issues tickets keep one ClientHello shape, the tls series of that run carry the port names of the plan, `mail-client` sends no ALPN extension and otherwise the ClientHello of `firefox-139`, and the new server profiles complete handshakes with clients that offer ALPN and with one that does not |
| Component | `tests/signing.rs` | The signature check of module loading with the CTL-11 vectors, see [Signed modules](#signed-modules) |
| End to end | `tests/controller_e2e.sh`, `make test-agt-controller` | The real controller and the real agent: the first party `http1` module (`single_fetch`), the `dns` zone file setting with its negative control, an `smtp/relay` run (AGT-13) that installs the first party `content` and `smtp` modules in both and passes with succeeded smtp sessions, and an `http1/https_fetch` run (AGT-14) that passes with criteria on the server side, with a negative control whose impossible server side criterion fails the run, an `ssh/interactive_shell` run and an `ssh/scp_upload` run (AGT-17) that install the first party `ssh` module and pass with succeeded ssh sessions, an `smb2/download` run and an `smb2/upload` run (AGT-19) that do the same for `smb2` on the ports 18687 and 18688 (the script has no root for port 445), and the captures of AGT-12 through `POST /v1/runs/{id}/captures`: on a dns run with port 18681, with a port that no session uses, and with the port name `server0`, and on an https run with `export_tls_keys` true and false. Each is stopped with `POST .../captures/{id}:stop` and downloaded, and the script checks the content type, the size, the SHA-256 against `Capture.files[].digest`, the packet count, and a small pcapng reader (and tshark when installed) finds only UDP packets of the port, no packet for port 9, and a decryption secrets block (type 0x0A) with `CLIENT_TRAFFIC_SECRET_0` only in the file with `export_tls_keys` |
| End to end | `tests/netns_smoke.sh`, `make e2e-agt` | The example plan, unchanged, runs in the QA-01 harness with the server side in the server namespace and the client side in the client namespace |

The end to end test routes the plan pools `16.0.0.0/24` and `48.0.0.0/24` through the routing namespace and adds a local route for each pool in its namespace. A local run with a debug build completed 38,107 sessions, 34,299 of `http1` and 3,808 of `dns`, on both sides without a failure, which matches the planned load. The workflow [`agt.yml`](https://github.com/alrawi/FixingPointSystem/blob/main/.github/workflows/agt.yml) runs it as root on `ubuntu-24.04`.

## Controller session

AGT-02 added the agent side of the control protocol. The agent dials the controller and serves no gRPC service, as CON-06 decided. The command below registers the agent and serves the commands of the controller until SIGINT or SIGTERM.

```bash
fps-agent connect --controller http://127.0.0.1:7443 --agent-id local \
  --modules contracts/examples/modules [--root DIR] [--side both|client|server] [--label site=lab] \
  [--redial-limit SECONDS] [--capture-dir DIR] [--capture-retention SECONDS] \
  [--module-trust-store FILE] [--dev-modules]
  [--cert-dir DIR [--tls-ca FILE] [--tls-ca-sha256 HEX] [--enroll-address HOST:PORT] \
   [--enroll-token-file FILE | --enroll-token-env NAME]]    # https:// controllers
```

The binary detects the host capabilities, adds the installed modules of `--modules` with their tree digests and the `--label` pairs, and calls `Register` with its state and plan. It then opens the `Session` stream with the token that `Register` returned, sends a heartbeat at the interval of the response and at once after every state change, and serves `prepare`, `start`, `stop`, `update_rates`, `start_capture`, `stop_capture`, and `fetch_capture`. Logs are JSON lines on standard error from `tracing`. Since CTL-27 the controller requires mutual TLS unless it runs with `insecure_plain_grpc`, so the agent dials an `https://` address with a certificate, see [Mutual TLS and enrollment](#mutual-tls-and-enrollment), and an `http://` address only against a controller in that mode.

### Generated types

`build.rs` compiles `contracts/proto/fps/agent/v1/agent.proto` with `protox` and `tonic-prost-build`, so the build needs no `protoc`. The module `fps_agent::pb` holds the messages, the client stub, and the server stub, which the tests use for a fake controller. `fps_agent::convert` turns the `Plan` of a `prepare` command into the plan type of the standalone mode and back, and turns an `fps-stats` snapshot into a `CounterSnapshot` through its proto3 JSON mapping. Both modes therefore run the same plan checks and send the same counters.

### State machine

The module `fps_agent::session` keeps the agent state in one structure behind a mutex that the session task and the run driver thread share. No lock is held across an `.await`.

| Command | Allowed in | Effect |
|---|---|---|
| `prepare` | `IDLE`, `STOPPED`, `FAILED` | `PREPARING` at once, then `PREPARED` after the plan checks, the module digests, and the worker assembly on a dedicated thread |
| `start` | `PREPARED` | `RUNNING` at once, and the run driver thread starts the workers at once or at `start_at` |
| `stop` | `PREPARED` | Releases the plan and answers with `IDLE` |
| `stop` | `RUNNING`, `STOPPING` | A ramp down moves to `STOPPING`, an abort resets every session, and the answer carries the final snapshot once the workers end |
| `stop` | `STOPPED` | Answers again with the final snapshot of the run |
| `update_rates` | `RUNNING` | Sets the target of the whole agent for each named mix entry, see [Run time rates and captures](#run-time-rates-and-captures) |
| `start_capture` | `RUNNING` | Starts a capture of the sessions that begin from now on |
| `stop_capture` | Any state that holds the plan | Ends the capture and answers with the digest of its file |
| `fetch_capture` | Any state that holds the plan | Answers with the size, digest, and chunk size of a finished capture file, then uploads it, see [Capture download](#capture-download) |

Any other pair fails with `ERROR_CODE_FAILED_PRECONDITION`, and a `plan_id` that does not name the loaded plan fails with `ERROR_CODE_NOT_FOUND`. A plan that breaks a rule or names another agent fails with `ERROR_CODE_INVALID_ARGUMENT`, a module digest, version, or tree that does not match fails with `ERROR_CODE_FAILED_PRECONDITION`, a plan with more workers than cores fails with `ERROR_CODE_RESOURCE_EXHAUSTED`, and a backend or flow kind that the run path lacks fails with `ERROR_CODE_UNIMPLEMENTED`. A failed command leaves the state unchanged. A worker that fails moves the agent to `FAILED`, and the next heartbeat carries the reason in `message`. A run that ends on its own moves to `STOPPED` and sends the final snapshot and a heartbeat.

The run driver thread reuses the run path of the standalone mode. `run::prepare` checks and assembles a plan, `Prepared::start` starts the server workers and then the client workers, and `Active` samples, stops, and finishes the run. The standalone command `fps-agent run` uses the same three steps, so the two modes cannot drift.

### Snapshots, reconnection, and the heartbeat timeout

The driver sends one aggregate `CounterSnapshot` per `counter_interval`, numbered from 1 for each run. A snapshot that the agent takes while the connection is down waits in the agent and goes out on the next session, so the sequence continues across sessions and the controller drops duplicates. Command results belong to the session that received the command, and the agent drops them when that session ends.

After a lost connection the agent dials again with exponential backoff from 200 ms up to 10 s and calls `Register` with its current state and plan, so the controller resumes the run. The [Redial policy](#redial-policy) section describes the jitter, the reset after an abort, and the limit. A watchdog compares the time since the last message from the controller with the heartbeat timeout of `Register`. When the timeout passes, the agent aborts any run, releases the plan, moves to `IDLE`, ends the session, and dials again.

### Kernel requirement

The agent needs the socket option `IP_LOCAL_PORT_RANGE`, which Linux offers from release 6.3, so that every worker can take source ports from its own range. Decision D12 item 3 lets the agent refuse an older kernel. `fps-agent connect` and `fps-agent run` read `proc/sys/kernel/osrelease` under the same injectable `SystemRoot` as capability detection, before they detect anything else or dial. A release before 6.3 ends the process with exit code 2 and this message on stderr, with the release of the host in place of `5.4.0-150-custom`.

```text
fps-agent: error: kernel 5.4.0-150-custom is too old: the agent needs the IP_LOCAL_PORT_RANGE socket option, which requires Linux 6.3 or newer (decision D12 item 3)
```

An unreadable or malformed release is an error too, and the check never passes on a guess. The command `fps-agent capabilities` still reports any kernel, so an operator can inspect an old host. Distribution kernels that backport the option under an older version number are refused as well, because the check compares versions and does not probe the option.

### Redial policy

Decision D12 item 11 says that an agent keeps redialing after an abort, with a backoff that restarts at 200 ms with jitter and with a limit. The type `fps_agent::session::RedialPolicy` holds the rules, and `serve` applies it after every ended session.

| Rule | Behavior |
|---|---|
| Backoff | The wait doubles from 200 ms up to 10 s |
| Jitter | Every wait is drawn uniformly from the upper half of the step, so a 200 ms step waits 100 to 200 ms and agents that lost the controller together do not dial together. The generator is seeded from the clock at process start |
| Reset | A session that opened and then ended, and every session that ended with `ABORTED`, send the backoff back to 200 ms. `ABORTED` means that another session of the same identity replaced this one |
| Limit | Failed attempts may last for `--redial-limit SECONDS`, which defaults to 300. A session that opened and ended without an abort clears the window. A failed dial and an abort start the window when none runs and leave it running otherwise, so two agents of one identity that replace each other forever still reach the limit. The value 0 means no limit |
| Exit | At the limit the agent aborts any run, releases the plan, logs `redial limit reached, the agent exits`, and exits with code 3 |

| Exit code of `fps-agent connect` | Meaning |
|---|---|
| 0 | SIGINT or SIGTERM stopped the agent |
| 2 | A usage error, an invalid controller address, a kernel before Linux 6.3, a failed capability detection, or a failure that redialing cannot fix, see [Mutual TLS and enrollment](#mutual-tls-and-enrollment) |
| 3 | The agent gave up redialing the controller after the redial limit |

A supervisor such as systemd can restart on code 3 and treat code 2 as a configuration fault.

### Mutual TLS and enrollment

AGT-08 gave the agent the TLS side of the controller (CTL-15 and CTL-27). A controller address with the scheme `https://` selects mutual TLS, and `http://` stays plain TCP for a controller that runs with `insecure_plain_grpc`. The module `fps_agent::identity` holds the code, and the tonic channel uses rustls with the ring provider, the stack of `fps-module-tls`. The owner chose the HTTPS enrollment endpoint, so no contract changed.

```bash
fps-agent connect --controller https://ctl.lab.example:8444 --agent-id edge-1 \
  --modules /var/lib/fps/modules --cert-dir /var/lib/fps-agent/pki \
  --tls-ca /etc/fps/ca.crt --enroll-token-file /etc/fps/enroll.token
```

| Flag | Default | Meaning |
|---|---|---|
| `--cert-dir DIR` | none | The directory of the key and the certificate. An `https://` controller needs it |
| `--tls-ca FILE` | none | A PEM file with the CA certificate of the controller, from `pki_dir/ca.crt` |
| `--tls-ca-sha256 HEX` | none | The SHA-256 fingerprint of the CA certificate, which the controller logs as `ca_sha256`. Colons, upper case, and a `sha256:` prefix are fine |
| `--enroll-address HOST:PORT` | host of `--controller`, port 8445 | The HTTPS listener of the controller for `POST /enroll` and `POST /renew` (`enroll_listen`) |
| `--enroll-token-file FILE` | none | A file that holds the enrollment token |
| `--enroll-token-env NAME` | `FPS_ENROLL_TOKEN` | The environment variable that holds the token when no file is named |

An `https://` controller needs `--cert-dir` and one of `--tls-ca`, `--tls-ca-sha256`, or a `ca.pem` in the directory from an earlier enrollment. An `http://` controller refuses every TLS flag, which keeps a plain connection from passing for a secure one. The token never travels on the command line, and `fps-agent` has no flag that takes it.

#### The files

The directory has mode 0700 when the agent creates it. Every file is written to a temporary name and renamed, so a reader sees the old file or the new one, and the agent refuses a key that group or others can read.

| File | Mode | Content |
|---|---|---|
| `agent.key` | 0600 | The ECDSA P-256 private key in PKCS 8 PEM |
| `agent.crt` | 0644 | The certificate that the controller issued, with the common name set to `--agent-id` |
| `identity.json` | 0644 | `not_after` and `renew_at` of the certificate as Unix seconds |
| `ca.pem` | 0644 | The CA certificate, written only by an enrollment that pinned a fingerprint |

#### Enrollment

At every dial the agent reads the directory. Without a valid certificate, or with a token file that exists, it enrolls first, which takes six steps.

1. It reads the token from the file or the environment variable. Without one it exits with the error `no enrollment token`, which names `fps-controller token` and both sources.
2. It makes an ECDSA P-256 key and a CSR with the common name `--agent-id` and no other name.
3. It connects to the enrollment address over TLS with the CA of the trust rules below, and sends `POST /enroll` with the token and the CSR.
4. It checks that the answer names its own identity, and it parses `not_after` and `renew_at`.
5. It writes the key, `identity.json`, and the certificate.
6. It deletes the token file or removes the environment variable, so the agent keeps no spent secret.

A token file that exists at a dial means that the operator wants a new enrollment, for example after `fps-controller revoke`, so the agent enrolls again over a valid certificate. The agent deletes the file after every enrollment, which makes a leftover file always new. A token in the environment enrolls only when no valid certificate exists, because the variable survives a restart.

#### Trust

The CA is the only trust anchor of both connections, and the agent never reads the system roots. It resolves the CA in this order: the file of `--tls-ca`, the `ca.pem` that an earlier enrollment stored, and the fingerprint of `--tls-ca-sha256`. With a CA file and a fingerprint, the file must hold a certificate with that fingerprint.

A fingerprint without a CA file has no CA certificate before the first enrollment, so the agent checks the enrollment endpoint during the TLS handshake, before it sends the token (AGT-10). Since CTL-31 the controller presents the leaf and the CA on the enrollment listener. A custom rustls `ServerCertVerifier` (`PinnedVerifier` in `src/identity.rs`) looks for the certificate with the pinned SHA-256 among the certificates after the leaf. It refuses the handshake when there is none. Otherwise it makes a root store of that one certificate and delegates to the webpki verifier of rustls, with the intermediates that remain and the server name of the address that the agent dialed, so the leaf must chain to the pinned CA, be valid, carry the server authentication usage, and name the address. The handshake signatures use the algorithms of the crypto provider as usual. A refusal ends the connection before the HTTP request, so an endpoint of another CA, an endpoint that presents only its leaf, and an endpoint whose leaf does not chain to the pinned CA never see the token. After the answer the agent still requires that its `ca_pem` holds the pinned certificate before it writes `ca.pem`. A CA file stays the safer choice, because it does not depend on the controller sending its CA, and an older controller that sends only the leaf makes a pin alone fail with `the endpoint presented no certificate with the pinned fingerprint`.

#### Renewal

A task checks `identity.json` and calls `POST /renew` when `renew_at` has passed. The request presents the current certificate over mutual TLS and carries a fresh CSR for the same key, because the files change one at a time and a new key would leave a window with a mismatched pair. The renewal runs on its own HTTPS connection and never touches the open session, so the agent never redials just to renew, and decision D12 item 12, which drops the plan with a lost session, holds. A renewal during a run is harmless, and it keeps a long run from outliving its certificate. The next dial reads the new files. A failed renewal retries with a wait from 5 s to 60 s until the certificate expires. When the controller refuses the renewal, the agent logs the reason, and the next dial enrolls again if a new token exists.

#### Permanent failures

Redialing cannot fix a handshake failure, `UNAUTHENTICATED`, a refused token, or a missing trust anchor, so the agent aborts any run, releases the plan, logs `the failure is permanent, the agent exits` with the reason, prints `fps-agent: error:` and the reason, and exits with code 2, which a supervisor reads as a configuration fault.

| Failure | Where it shows | Message starts with |
|---|---|---|
| The CA does not match the controller certificate, or the peer does not speak TLS | The dial | `TLS failure: invalid peer certificate: UnknownIssuer` and similar |
| The controller refuses the certificate of the agent, for example after a revocation | The first call after the dial, as an alert of TLS 1.3 | `TLS failure: received fatal alert: BadCertificate` |
| The controller refuses the certificate and closes the connection with no alert that the agent reads, twice in a row | The first call after two dials, see below | `TLS failure: the controller closed the connection right after the TLS handshake` |
| The endpoint of a pinned fingerprint presents no certificate with that fingerprint, or a leaf that does not chain to it | The handshake of `POST /enroll`, before the token | `TLS failure toward the enrollment endpoint` and `the pinned fingerprint` |
| `Register` answers `UNAUTHENTICATED` | The first call | `the controller answered UNAUTHENTICATED` |
| The token is unknown, used, or expired | `POST /enroll` | `the controller refused the enrollment token (HTTP 401)` |
| The `ca_pem` of the answer lacks the pinned certificate | `POST /enroll` | `the CA certificate does not have the pinned fingerprint` |
| No certificate and no token | The dial | `no enrollment token is available` |

A pinned fingerprint that does not match is found in the handshake, so the token never leaves the agent, and the agent keeps the token file. ##### A close right after the handshake

Under TLS 1.3 a client finishes its handshake before the server checks the client certificate. A controller that refuses the certificate then sends an alert and closes the connection, and the agent reads either the alert, which is the row above, or only the close, which surfaces as an h2 or hyper `connection closed`, `broken pipe`, or `connection reset` on the first call. The first outcome was a permanent failure and the second a redial, so a refused certificate ended the agent at the redial limit, which made `tests/mtls.rs` unstable in CI.

The rule is narrow. The agent counts a session as a close after the handshake only when all of these hold: the agent presented a client certificate, the dial completed, so the TCP and TLS handshakes finished, the `Register` call is the first call of that connection, so no response arrived, and the call failed with `UNAVAILABLE`, `UNKNOWN`, or `CANCELLED` (CI saw `operation was canceled: connection closed`) whose cause chain says that the connection was closed, broken, or reset (`identity::closed_before_response`). A controller that restarts can close a fresh connection in the same way, so the agent does not exit on the first close. It dials again at once, without the backoff, and exits with the permanent `TLS failure` only when that second dial ends the same way. Anything else resets the count: a dial that fails (a controller that is down is `connection refused` and redials with the backoff), a timeout, a gRPC status from the controller, a registered session. An agent with no client certificate, that is a `http://` controller, never takes this path. The cost is one extra dial of a refused agent, and a restarting controller has to close two fresh connections in a row after their handshakes to end an agent, which the next dial then fixes by a restart of the agent.

`PERMISSION_DENIED`, which means that the inventory does not list the agent, and every unreachable controller still redial with the redial limit, because an operator can fix both while the agent runs.

### Run time rates and captures

AGT-05 serves the commands that CON-05 added in protocol 0.2.0. The agent announces `engine_api` 0.2.1, so the controller may send them.

#### UpdateRates

The command is valid in `RUNNING` only, once the workers run. A `plan_id` that does not name the loaded plan fails with `ERROR_CODE_NOT_FOUND`. The handler turns every `RateUpdate` into one `fps_sched::RateControl::update` entry. The target is the rate or the concurrency of the whole agent for the mix entry, and each registry divides it among the workers of the entry in proportion to their current targets. A mix entry that the command does not name keeps its targets, a target of 0 pauses an entry, and a positive one resumes it.

Mix entries run in two places. `fps-runtime` registers the rate handles of the TCP client workers, which `RuntimeHandle::rate_control()` returns, and `assemble.rs` now sets `FlowRef::entry` to the mix entry id, so two entries that share a scenario do not collide. The UDP workers of `src/udp.rs` build their schedules outside `fps-runtime`. They build the schedule before they report ready, register its handle in a registry of their own that `UdpRuntime::rate_control()` returns, and sleep at most 100 ms, so a paused or capped worker sees a new target within that time, as the TCP workers do. We chose to register the UDP handles, so `UpdateRates` accepts UDP entries too. `fps_agent::control::Rates` joins the two registries.

The command applies completely or not at all. `Rates::update` checks every entry, kind, and value across both registries before it stores anything, and each refusal is `ERROR_CODE_INVALID_ARGUMENT` with a message that names the entry.

| Refusal | Message names |
|---|---|
| The entry is not in the plan | `is not a mix entry of the plan` |
| The plan has the entry, but no client worker of this agent runs it, as on a `--side server` agent | `has no client worker with a rate on this agent` |
| One entry twice | `is named twice` |
| `sessions_per_second` for a `concurrent_sessions` entry, or the reverse, or no target | `runs concurrent_sessions` or `runs sessions_per_second` |
| A rate below 0 or not finite | `is not a finite number of at least 0` |

`applied_at` is the agent time at which the registries took the targets. The workers apply them on their next tick, at most 100 ms later.

#### StartCapture

The command is valid in `RUNNING` only. The capture needs a `capture_id` of 1 to 128 letters, digits, dots, underscores, and hyphens that does not start with a dot, because the id names files, and a `capture_id` that is already in use in the plan fails with `ERROR_CODE_FAILED_PRECONDITION`. The agent runs one capture at a time, and a second `StartCapture` while one runs fails with `ERROR_CODE_FAILED_PRECONDITION` as well.

The engine sees session bytes and no packets, so the agent captures sessions and not an interface. `fps-capture` samples every session (`EveryNth(1)`), synthesizes Ethernet, IP, and transport frames around the bytes, and writes them into a bounded ring. A capture that starts in the middle of a run reaches the workers through `fps_capture::CaptureCell` (re-exported as `fps_agent::control::CaptureCell`), a cell with one atomic flag and the current `Capture` that the TCP and UDP workers share, see [One cell for TCP and UDP](#one-cell-for-tcp-and-udp). A worker reads it once per session, when the session opens, so a run without capture pays one atomic load per session, and a session that opens right after `StartCapture` is recorded. A session that was already open is not.

| Field of the command | What the agent does |
|---|---|
| `port` | Empty records every session. Otherwise the name of a port of the plan, or a server port, a list, or a range of ports, see [Port filter](#port-filter). A text that is none of these is `ERROR_CODE_INVALID_ARGUMENT` |
| `filter` | Empty only. The agent has no pcap-filter compiler, so any expression is `ERROR_CODE_INVALID_ARGUMENT`, the code that the contract gives for a filter that the agent cannot compile |
| `snap_length` | 0 only. A positive value is `ERROR_CODE_UNIMPLEMENTED`, because the capture keeps whole sessions |
| `max_bytes` | Ends the capture once the packets written hold this many bytes, without framing. 0 means no limit |
| `max_duration` | Ends the capture after this time. Absent or 0 means no limit |

The agent refuses what it cannot honor, so it never records more than the controller asked for. The ring holds at most `file_bytes * max_files` of packets, 64 MiB by 8 files, so the oldest sessions of a long capture leave the file. Sessions that the writer queue cannot take, or that do not fit a ring file, are dropped and counted.

Both sides of a conversation are in one capture when the agent runs client workers, because a client session holds both directions. A process with server workers only, such as `--side server`, captures its server sessions. With `export_tls_keys` the key log lines of the TLS sessions go into the file as decryption secrets blocks, see [TLS profiles](#tls-profiles), so such a capture is as sensitive as a private key. The agent creates its capture directory with mode 0700.

#### Port filter

The REST contract of `startCapture` requires a `port`, the name of a port of the test configuration (`network.ports[].name`), and the agent protocol calls it the name of a `PortBinding` of the plan. AGT-12 adds a second form, because a test usually thinks of a capture as "the DNS traffic". `fps_agent::portfilter::PortFilter::parse` reads the `port` of the command in this order.

1. Empty: every session, as before.
2. The name of a port of the plan, such as `client0`: the sessions that run between hosts of the pools that sit on that port. A session matches when its client address or its server address lies in the IPv4 or IPv6 prefix of such a pool.
3. A server port of TCP and UDP: one port (`53`), a comma separated list (`80,443`), a range (`8000-8999`), or a list of both (`53,8000-8999`). A session matches when its server port is in the set, for TCP and UDP alike. A port is a decimal number from 1 to 65535, a range must not end before it starts, a list has at most 64 items, and spaces around an item are ignored.
4. Anything else, such as `nope`, `0`, `70000`, `90-80`, or `80,,81`, is `ERROR_CODE_INVALID_ARGUMENT`, and the message lists the ports of the plan.

A name wins over a number, so a port of the plan that is called `53` is a name. The `filter` field (a pcap-filter expression) stays refused.

**Where the filter acts.** The taps of the TCP sessions live in `fps-runtime` and hand every session to the capture of the shared cell, so the filter cannot sit at the tap without a change in `fps-capture` or `fps-runtime`. The agent applies it when `StopCapture` joins the ring into the file. `FrameFilter` reads the Ethernet, IP, and TCP or UDP headers of every frame that the capture synthesized, finds where a session starts (a TCP SYN, or a change of the two endpoints), takes the server end from the first frame, which the client sent, and drops the frames of the sessions that do not match. The file holds the matching sessions only, and the `packets`, `bytes`, and `digest` of `StopCaptureResult` describe that file, so the controller and the download agree. A port with no session leaves a valid file with a section header and an interface description and no packet.

**Limit.** The ring still takes the sessions that the filter will drop, so the `max_bytes` limit counts them, and the ring bound (`file_bytes * max_files`) can push the oldest matching sessions out of a long capture that runs beside much other traffic. A filter at the tap would fix both. It is a small change in `fps-capture` (a predicate on the flow in `Capture::begin`) that belongs to an ENG package.

#### StopCapture

The command names the plan and the capture. It is valid in every state that holds the plan, and it is idempotent: a capture that ended at a limit, at the end of the run, or at an earlier `StopCapture` answers again with the same result. A `plan_id` or `capture_id` that the agent does not know fails with `ERROR_CODE_NOT_FOUND`. A thread ends the capture and answers when the file is ready, so a long join does not hold up the heartbeat. The run end ends every capture too, after the workers ended and every session reached the writer.

**What the digest covers.** `StopCaptureResult` carries one digest, and the ring holds several files. The agent joins the ring files, oldest first, into one pcapng file by concatenating them, which the pcapng format allows because every ring file is a complete section. `bytes` is the size of that file, `digest` is `sha256:` and the lowercase hex SHA-256 of its bytes, and `packets` counts its enhanced packet blocks. The file is the deterministic result of the ring, so the digest does not depend on a tar header or a clock. A capture with no session is a valid file with a section header and an interface description. `dropped` counts sessions that the capture could not record, because the controller needs a number and the agent cannot know how many packets a dropped session held. Sessions that rotated out of a full ring are not in the file and not in `packets`. The ring files are removed once the joined file is written.

**How the controller gets the file.** CON-13 defines the transfer, which the owner chose to run over the agent connection: the controller sends `fetch_capture` with a start offset and a single use transfer id, and the agent opens the client streaming `UploadCapture` call and sends the file in chunks of at most 256 KiB, ending with the size and the digest of `StopCaptureResult` ([Contracts](Contracts.md)). The agent reports `engine_api` 0.2.2 and serves `fetch_capture`, see [Capture download](#capture-download). The file stays on the agent at the path below, and the agent logs the path, the packets, and the digest at `info` level (`capture file ready`). The directory is `--capture-dir`, which defaults to `fps-agent-captures` under the temporary directory of the process.

```text
<capture dir>/<plan id with each other byte as _>-<8 hex digits of sha256(plan id)>/<capture id>.pcapng
```

The function `fps_agent::control::capture_path` computes it. Starting a capture removes the files of an earlier capture with the same plan and capture id, so a reused id never mixes two captures.

A failure to write the file is `ERROR_CODE_RESOURCE_EXHAUSTED`, never `ERROR_CODE_INTERNAL`, because the contract moves an agent to `FAILED` after `ERROR_CODE_INTERNAL`, and a full disk must not end a healthy run.

#### Capture download

`fetch_capture` is valid in every state that holds the plan and leaves the state unchanged. The command names the plan, the capture, a `start_offset`, a single use `transfer_id` of 1 to 128 letters, digits, dots, underscores, and hyphens, and an optional `max_chunk_bytes`. The session loop checks it and answers at once with `FetchCaptureResult`: the `total_bytes` and `digest` that `StopCaptureResult` reported and the chunk size, which is the smaller of `max_chunk_bytes` and 262144 bytes and 262144 when the command says zero. Only after that answer is queued on the `Session` stream does the agent open `UploadCapture`, so the controller always knows the transfer before the call arrives.

| Command | Answer |
|---|---|
| Unknown plan, unknown capture, a capture that ended without a file, or a file that the retention removed | `ERROR_CODE_NOT_FOUND` |
| A capture that has not ended | `ERROR_CODE_FAILED_PRECONDITION` |
| `start_offset` above the size of the file, or a missing or malformed `transfer_id` | `ERROR_CODE_INVALID_ARGUMENT` |
| Another transfer is in flight | `ERROR_CODE_RESOURCE_EXHAUSTED` |

A `start_offset` equal to the size is valid, and the upload then holds the `open` and `end` messages and no chunk. The upload is a task of the session that carried the command. It reuses the tonic channel of that session, so the call rides the same TCP connection and the same mutual TLS identity as `Session` (AGT-08), and it ends when the session ends. It sends `open` with the `transfer_id`, the session token, the capture id, and the start offset, then chunks with ascending offsets from `start_offset` to the end of the file with no gap, and `end` with the `total_bytes` and the digest of the whole file. A reader task feeds a channel of two messages, so HTTP/2 flow control stalls the file read when the controller reads slowly, and the heartbeat and the command results go on, because the call is its own HTTP/2 stream and the task never holds the session loop. A file that ends early or cannot be read resets the call, which the controller sees as a broken transfer. The agent logs `capture upload finished` with the `received_offset` and `verified` of the controller answer, or `capture upload refused or broken`.

The agent serves one transfer at a time. The finished file is immutable: the transfer holds the file open and checks its size against `StopCaptureResult` before it answers.

**Retention.** `--capture-retention SECONDS` sets how long a finished capture file stays on the agent, counted from the moment the joined file was written. The default is 86400 seconds (24 hours), and 0 keeps every file. A sweep a few times within the retention deletes the expired files, including the files of captures of an earlier plan, which no command finds any more. A file with a transfer in flight is never deleted: the sweep skips it, and it goes at the first sweep after the transfer ended. Starting a capture whose id an earlier run of the same plan used would replace that file, and fails with `ERROR_CODE_FAILED_PRECONDITION` while a transfer reads the old file. After the retention, `fetch_capture` answers `ERROR_CODE_NOT_FOUND`, and `stop_capture` still answers with the old digest, because the retention changes no state of the capture.

#### One cell for TCP and UDP

`fps_agent::control::CaptureCell` is `fps_capture::CaptureCell` (ENG-19, AGT-10). `Prepared::start_with` gives the same cell to the `RuntimeConfig` of the TCP workers (`cfg.capture`) and to the UDP workers, so one `StartCapture` records the sessions of both transports from the next session on. A process that runs clients and servers records the client side only, because a client session holds both directions, so its server runtime gets a cell that stays empty and the UDP workers skip their server sessions, as before. A process that runs servers only records them. The test `a_fake_controller_changes_rates_and_reads_a_capture` parses the pcapng file of a plan with an HTTP entry on TCP and a DNS entry on UDP with its own reader and finds TCP packets on the HTTP port and UDP packets in the one file.

### Session tests

| Level | Test | What it shows |
|---|---|---|
| Unit | `src/convert.rs` | The example plan survives the round trip through the generated types, and unknown enum numbers, negative durations, and list parameters are errors |
| Component | `tests/session.rs` | An in process fake controller drives prepare, start, and stop over loopback and receives numbered cumulative snapshots and a final snapshot, out of order calls fail with their codes, a bad plan leaves the state unchanged, a run continues across a reconnection and stops on its own, and a silent controller makes the agent release the plan |
| Component | `tests/run_control.rs` | A fake controller changes the rates of a TCP entry and a UDP entry of a running plan, and the new rates show in the snapshots, refused updates change nothing, a pause and a resume work, and the capture commands answer with the codes above. The controller starts a capture in the middle of the run, stops it, locates the file below the capture directory, reads it block by block, and checks the size, the SHA-256, and the packet count of the result. A second capture ends at its time limit with no `StopCapture`, and a third ends with the run and answers after it. The first capture stops at rest (AGT-18): the test pauses both entries and waits until `sessions.concurrent` is zero, because a session that is open at the stop ends after the writer has gone and the capture counts it in `dropped`. `a_capture_that_loses_sessions_reports_them_as_dropped` shows that a really lost session reaches `StopCaptureResult.dropped`, with a ring file too small for the HTTP sessions, and the ignored `repeat_stopping_a_capture_at_rest` stops a capture at rest 50 times in a row |
| Component | `tests/run_control.rs` (port filter) | `a_capture_with_a_port_skips_the_sessions_of_other_ports`: on a plan with an HTTP entry on TCP and a DNS entry on UDP, captures with the TCP server port, the UDP server port, a list of both, a range, port 9, and the names `client0` and `server0` keep only what the filter selects, and every file agrees with its `StopCaptureResult` in size, digest, and packet count. The unit tests of `src/portfilter.rs` cover the grammar, the refusals, the session start rule, IPv6, and the pool match |
| Component | `tests/run_control.rs` (download) | The fake controller refuses an unknown capture and plan, a running capture, an offset beyond the size, and bad transfer ids with the codes above. It then downloads a finished capture in chunks of 64 bytes, drops the call after a third of the file, and resumes at the offset that it holds with a new transfer id. The two parts join into the file, and the SHA-256 of the joined bytes equals the digest of `StopCaptureResult` and of both `end` messages. An offset equal to the size sends `open` and `end` only |
| Unit | `src/control.rs` | An update spans both registries and keeps entries that it does not name, every refusal changes nothing, and plan ids with a separator map to flat, distinct directories. A transfer reads from an offset, the agent serves one at a time, and the retention deletes an old file, spares a file with a transfer in flight and deletes it after the transfer, and handles the files of an earlier plan |
| Component | `tests/detect.rs`, `tests/redial.rs`, `tests/exit_codes.rs`, `make test-agt-redial` | A 5.4 kernel string gives an error that names `IP_LOCAL_PORT_RANGE`, and 6.3 and newer pass. The backoff doubles, restarts at 200 ms with jitter after an abort, and resets after a session that opened. Failed attempts end `serve` with `RedialLimit` at the limit, aborts alone reach the limit too, and the binary exits with 2 and 3 |
| Component | `tests/mtls.rs` | A fake enrollment endpoint and a fake controller over mutual TLS, both made with rcgen. The agent enrolls with a token from a file, which it deletes, registers with a certificate that names it, and renews at `renew_at` without a new session. A pinned fingerprint replaces the CA file and a stored CA serves the next start. A new token file replaces a certificate. A pinned fingerprint is checked in the handshake: an endpoint of another CA, one that presents only its leaf, and one whose leaf does not chain to the pinned CA end the agent and never see a request. A controller that closes the first connection after its handshake is redialed, and one that closes two in a row ends the agent. A wrong pin, a refused token, a missing token, a controller of another CA, a certificate that the controller refuses, `UNAUTHENTICATED`, and a peer that does not speak TLS each end the agent with a permanent error, an unreachable controller does not, and the binary exits with code 2 and a TLS message |
| End to end | `tests/controller_e2e.sh FPS_CONTROLLER FPS_AGENT` | The real `fps-controller` mints a token with `fps-controller token`, the real agent enrolls with it over HTTPS and registers over mutual TLS, installs the example `http1` module and the first party `dns` module, and drives a loopback run over REST to `completed` with a passed verdict. A dns test with the `zone_file` module setting passes, and the same test without it fails. An agent that trusts another CA ends with exit code 2 before it spends its token, and after `fps-controller revoke` the running agent ends with exit code 2 and enrolls again with a new token |

A local run of the end to end script with debug builds completed its run with 148 succeeded `http1` sessions in three seconds at 40 sessions per second. A second run follows `contracts/examples/tests/dns_zone.yaml` with the first party `dns` module: its `zone_file` setting names a zone for `www.e2e.test` that the script adds to the module directory, the lookups ask for that name, and the run passes only when the agent applied the setting. The same test without the setting fails its criteria. The script installs the same module trees in the controller and in the agent directory, because the agent checks the tree digest of every module. The controller checks port locations against the agent capabilities, which leave out loopback, so the script names the first detected interface while the loopback pools carry the traffic.

### Limits

- A scenario with several flows runs over TCP only. A scenario with a UDP flow among several is refused at prepare, because the instance runner opens no datagram sockets (an SIP scenario with RTP needs a runtime package first).
- `per_worker` snapshots are not implemented. The agent logs a warning and sends aggregate snapshots.
- The agent does not bind ports during `prepare`. The workers bind their sockets when the run starts.
- The redial limit counts time and not attempts, and a dial that hangs for its five second connect timeout can overshoot it by that much.
- The agent renews with the key that it enrolled with, and it never rotates the key. A new key needs a cert and key pair that one rename switches.
- A fingerprint without a CA file needs a controller that presents its CA in the chain of the enrollment endpoint (CTL-31), see [Trust](#trust).
- Filters (pcap-filter expressions) and snap lengths are refused. A port is accepted since AGT-12, see [Port filter](#port-filter), and a capture through `POST /v1/runs/{id}/captures` runs against the real agent.
