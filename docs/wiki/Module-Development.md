# Module Development

This page is the guide for people who write modules outside the project. It assumes that you have never seen the repository internals. It tells you how a module is laid out, how to write and validate one, how to package, sign, and upload it, and how the agent loads it. It also says plainly what a third party can do today and what only the project can do. Every statement names the file that it rests on, so you can check it and so a later change to that file shows which statement to update.

[Modules](Modules.md) catalogs the first party modules and is the place to look for worked examples, `modules/dns/` being the fullest. [Contracts](Contracts.md) summarizes the contracts that this page uses, and the [contracts README](https://github.com/alrawi/FixingPointSystem/blob/main/contracts/README.md) is the normative text. [SDK](SDK.md) describes the Python client of the controller REST API. A pull request that changes how a module is written updates this page in the same pull request, as the Modules section of [Coding Standards](Coding-Standards.md#modules) requires.

## What you can do today

The table separates what works from what does not. The sections below give the detail, and the last section lists the gaps.

| You want to | Today | Rests on |
|---|---|---|
| Write scenarios that use the core functions, profiles, distributions, loops, branches, spawned flows, and TLS upgrades | Works. A module like this needs no Rust crate and no WebAssembly component. | `contracts/schemas/scenario.schema.json`, `engine/fps-agent/src/modules.rs` |
| Call template functions of a first party module such as `content` | Works through `requires`, when that module is installed in the controller and in every agent. | `engine/fps-agent/src/modules.rs` (`add_required`) |
| Write your own template functions in Rust | First party only. A crate is linked into the agent at build time, and decision D10 (status Proposed) admits only modules of this repository. | `docs/decisions/0010-native-extensions.md`, `engine/fps-agent/src/modules.rs` (`REGISTRY`) |
| Write your own template functions as a WebAssembly component | The agent loads `extension.wasm` and serves the `render` and `on-close` hooks. The repository ships no guest SDK, no build instructions, and no example component. | `docs/wiki/Agent.md`, `docs/wiki/Engine.md`, `contracts/extension/README.md` |
| Give a module settings | Not for a module outside the repository. The agent refuses settings for a module without native functions in its registry. | `engine/fps-agent/src/modules.rs` (`for_modules`) |
| Publish to a shared registry | Not supported. A registry is one local directory of one controller, and an operator uploads a module to it. | `controller/internal/registry/doc.go` |
| Sign a module | Works with any Ed25519 library, and the procedure is below. The repository has no signing command. | `controller/internal/registry/signing/`, `docs/wiki/Controller.md` |
| Have the controller send a module to the agents | Not supported. An operator copies the module tree into the modules directory of every agent. | `docs/wiki/Agent.md` |
| Use the types `tunnel`, `strike`, `evasion`, and `fuzzer`, or the `raw` action | Not supported. The schema accepts the types, and the specification reserves them. | `docs/spec/specification.md`, `tools/contracts/validate.py` |

## The contracts at a glance

Three contracts define a module, and the validator enforces the first two.

| Contract | Version | File |
|---|---|---|
| Module manifest | v0.5 | `contracts/schemas/module-manifest.schema.json` |
| Scenario | v0.6 | `contracts/schemas/scenario.schema.json` |
| Extension interface | `fps:extension@0.1.0` | `contracts/extension/fps-extension.wit` and `contracts/extension/README.md` |

The versions come from the `description` of each schema and from [Contracts](Contracts.md#contract-files). A field is never removed or renamed in a minor version, and deprecation lasts two minor releases (Coding Standards, [Contracts section](Coding-Standards.md)).

## Module layout

A module is a directory whose name equals the module name. The file `manifest.yaml` sits at its root. Everything else is optional or named by the manifest.

```text
acme_ping/
  manifest.yaml        identity, parameters, scenario list, byte estimates, provenance
  scenarios/*.yaml     flow programs, listed in the manifest
  scenarios-pending/   scenarios that use syntax the engine does not compile yet (not listed, still checked)
  extension.wasm       optional WebAssembly component, see "Template functions of your own"
  fixtures/            optional capture and expected labels
  README.md            parameters, scenarios, and results, for your readers
```

The layout follows the Modules section of [Coding Standards](Coding-Standards.md#modules). Three rules come from the code.

1. The directory name must equal `name` in the manifest, or `tools/contracts/validate.py` reports an error (`check_module_dir`). The agent reads a module from `<modules directory>/<name>` (`engine/fps-agent/src/modules.rs`, `load_module`).
2. Every path that the manifest names is relative, has no `..` segment, and uses only letters, digits, `_`, `.`, `/`, and `-` (the `relpath` definition of the manifest schema). The registry holds every file of the tree to the same rule, not only the named files (`controller/internal/registry/archive.go`, `entryPath`), so a file with a space in its name is refused at install, and `validate.py` refuses it as well (CON-30).
3. The tree holds no symbolic link and no special file. The registry, the digest, and the agent refuse one (`registry/digest.go`, `registry/archive.go`, `modules.rs`), and `validate.py` reports one (`tree_problems`). A link as the module directory itself is fine for the validator, which is how the first party targets validate a module.

The directory `scenarios-pending/` holds scenarios that use syntax the engine does not compile yet. The validator checks each file against the scenario schema and indexes none (`check_module_dir`), the registry checks the same way (`registry.go`, `checkTree`), and the manifest does not list them.

## Start from the template

The directory [`modules/_template/`](https://github.com/alrawi/FixingPointSystem/tree/main/modules/_template) is a complete starting point. It holds a manifest of a module named `template`, one scenario in which a client sends a request and a server answers, and a README. It validates as it stands, and `make lint-mod-template` proves that on every change (`mk/mod-template.mk`). Its comments point at the sections of this page.

You need Python 3 with the packages of `tools/contracts/requirements.txt` (CI runs Python 3.12) and a checkout of the repository, because the validator reads the schemas from `contracts/` next to it (`tools/contracts/validate.py`, `REPO`). Your module does not have to live in the checkout.

```bash
git clone https://github.com/alrawi/FixingPointSystem.git fps
python3 -m pip install -r fps/tools/contracts/requirements.txt

mkdir mods
cp -r fps/modules/_template mods/acme_ping
rm -r mods/acme_ping/native            # the Rust crate serves first party modules only

sed -i 's/^name: template$/name: acme_ping/; s/protocol: template/protocol: acme_ping/' mods/acme_ping/manifest.yaml
sed -i 's/module: template/module: acme_ping/' mods/acme_ping/scenarios/request_response.yaml

python3 fps/tools/contracts/validate.py --modules mods --tests --no-openapi
```

The last command prints `contracts: 0 error(s), 0 warning(s)` and exits with 0. Four details of the command matter.

- `--modules` names a directory that holds module directories. The validator treats every subdirectory as a module (`run`), so do not point it at the module itself. This is why the first party targets validate through a parent directory that holds a link (`mk/mod-dns.mk`).
- `--tests` with no file switches the test configurations off. Without it the validator checks the example configurations of `contracts/examples/tests/` against your modules and reports their scenarios as unknown (`main`).
- `--no-openapi` skips the OpenAPI document, which a module author does not need.
- The exit code is 1 when any finding has the severity error, and warnings do not change it (`main`).

Then work through the manifest and the scenario in the following sections, replace the README, and run the validator after every change. The comment blocks of the template are yours to delete.

### Starting from a capture

The compiler under `capture/` turns a capture of your own traffic into a module directory. `python3 -m capture sessions`, `scrub`, and `scenario` run in that order (`docs/wiki/Capture.md`, [Scenario command](Capture.md#scenario-command)). A module that comes from a capture validates only when its provenance carries a consent note and `scrubbed: true`, which `scrub` writes, see [Provenance](#provenance).

## The manifest

The manifest is `manifest.yaml` at the root. The schema sets `additionalProperties: false` on the root, so an unknown key is an error. `apiVersion`, `kind`, `name`, `version`, `type`, `engine_api`, `license`, and `provenance` are required.

| Field | Meaning and rules | Consumed by |
|---|---|---|
| `apiVersion` | The constant `fixingpoint.dev/v1alpha1` | Schema |
| `kind` | The constant `Module` | Schema |
| `name` | An identifier, `^[a-z][a-z0-9_]{0,62}$`. It equals the directory name. The names `vars`, `session`, `last`, `profiles`, `dist`, `true`, `false`, and `all` are reserved, see [Names](#names). | Schema, `validate.py`, registry, agent |
| `version` | A semantic version, with optional pre-release and build parts. A version in a registry is immutable. | Schema, registry (`ErrModuleExists`), agent |
| `type` | One of `app`, `transport`, `host`, `content`, `tunnel`, `strike`, `evasion`, `fuzzer`, see [Module types](#module-types). | Schema, `validate.py` (`raw` rule) |
| `summary` | Up to 200 characters. It appears in the module list of the controller. | Registry index |
| `engine_api` | A semver range such as `">=0.2, <0.3"`. We found no code that compares it with the engine version, see [Gaps](#gaps). | Schema only |
| `license` | An SPDX expression, see [Licenses](#licenses). The schema checks the length, and `validate.py` and the registry check the expression. | Schema (length), `validate.py`, registry |
| `provides` | Items with `protocol`, `transport` (`tcp`, `udp`, or `sctp`), and an optional `port`. It documents the module. | Schema only |
| `requires` | Items with `module` and `version`. A scenario may call the functions of a module that it requires. | `validate.py`, agent |
| `parameters` | A JSON Schema (draft 2020-12) with an object root. A test sets it per mix entry in `profile.mix[].params`. | `validate.py`, controller compiler, agent |
| `settings` | A JSON Schema with an object root. A test sets it once per module under `modules.<name>.settings`. A module without it accepts no settings. | Controller compiler |
| `scenarios` | Scenario files relative to the manifest, unique, each of which must exist. | `validate.py`, registry, agent |
| `expected_bytes` | Application payload bytes per session in both directions with default parameters, keyed by scenario name, each an integer of at least 1. The controller reads it into the scenario catalog (CTL-47), see [below](#expected_bytes-bytes-per-session-of-each-scenario). `validate.py` rejects a key that names no scenario of `scenarios`. | Controller compiler, `validate.py` (keys) |
| `assets` | Items with a `name` and exactly one of `path` and `generator` (`module.function`). | Existence check only |
| `functions` | The template function catalog, see [Template functions](#template-functions-and-the-catalog). | `validate.py`, agent, registry |
| `extension` | `wasm`, which must be the text `extension.wasm` because the agent loads only that file at the module root, and `hooks` (a non empty list from `on_open`, `on_data`, `on_timer`, `on_close`, `render`). | Schema, `validate.py`, registry, agent |
| `fixtures` | Items with a `capture` path and an `expect` object with `ndpi` or `zeek` name lists. | Existence check only |
| `provenance` | Where the content comes from, see [Provenance](#provenance). | Schema |

The first column names fields of `contracts/schemas/module-manifest.schema.json`. The last column comes from the readers: `tools/contracts/validate.py`, the registry in `controller/internal/registry/` (`manifest.go`, `registry.go`), the controller compiler (`controller/internal/compiler/settings.go`, `controller/internal/mixcontrol/`, `controller/internal/runs/calibrate/`), and the agent (`engine/fps-agent/src/modules.rs`).

Three details need care.

- **The version line.** The agent reads the top level `version` by a line prefix and not through the YAML parser (`modules.rs`, `version_of`). Write `version: 0.1.0` on a line of its own, with no trailing comment, or the agent sees a different version than the plan and refuses the module.
- **Parameters name variables.** An entry of `profile.mix[].params` overrides a scenario variable, and the engine refuses a parameter that names no variable of the scenario (`engine/fps-flow/src/program.rs`, `override_vars`). Give every property of `parameters` the name of a `vars` entry. The validator checks a test configuration against the `parameters` schema (`check_test_config`), and it does not check that the names agree.
- **Byte estimates.** Give every scenario an entry in `expected_bytes`. A test with the `bytes` basis or the `throughput_bps` objective cannot compile a mix with a scenario that has none. The next subsection explains the field. For the template the estimate is 256 plus 1024.

#### `expected_bytes`: bytes per session of each scenario

`expected_bytes` maps the name of each scenario to the estimated application payload bytes that one session of the scenario moves, in both directions together, with the default parameters of the scenario. Each value is an integer of at least 1, and each key follows the identifier pattern of scenario names (`contracts/schemas/module-manifest.schema.json`, key `expected_bytes`). Each key must also be the name of a scenario that `scenarios` lists, and `validate.py` reports a key that is not (a typo in a key leaves the real scenario without an estimate). The registry does not check the keys yet. The schema marks the field as optional, but a module that has scenarios needs an entry for each of them.

```yaml
scenarios:
  - scenarios/web_get.yaml
  - scenarios/web_browse.yaml
expected_bytes:
  web_get: 17244
  web_browse: 70279
```

**What the controller does with it.** The controller reads the field into the scenario catalog (`controller/internal/wiring/adapters.go`, `registry.Manifest.ExpectedBytes`) and uses it in two places of the compiler (`controller/internal/compiler/`, see the Test compiler section of [Controller](Controller.md)):

- `profile.basis: bytes`. The test configuration gives byte weights, and the compiler turns them into session shares with `share_i = (weight_i / bytes_i) / sum_j (weight_j / bytes_j)`, where `bytes_i` is your estimate. Sessions at these shares then carry the byte weights.
- `load.objective.type: throughput_bps`. The session rate is `objective / (8 * mean bytes per session)`, with the mean taken over the mix. A wrong estimate makes the run miss the objective by the same ratio until the calibration of the run loop corrects it.

**What happens without it.** A scenario without an entry has no estimate. A test that uses the bytes basis, or the `throughput_bps` objective, and mixes that scenario fails to compile with the error `no bytes per session estimate` (`ErrNoByteEstimate`), and the message names the scenario as `<module>/<scenario>`. The controller never falls back to session shares, because that would run another mix than the one the test asks for. A test with the `sessions` basis and a session rate or concurrency objective does not need the field. Scenarios in `scenarios-pending/` are not in the manifest, need no entry, and may not have one.

**The estimate is a starting point.** During a run the controller measures the real bytes per session and replaces the manifest estimate of a scenario after 20 pooled sessions (`controller/internal/runs/calibrate/`, the Throughput rate calibration section of [Controller](Controller.md)). A good estimate keeps the first seconds of a run on target, so measure it and do not guess it.

**How to measure it.**

1. Run the scenario to its end with the default parameters, in a test of your module over the in memory network, as `modules/hls/tests/scenarios.rs` does. A scenario whose sizes come from the session generator, such as a message with `attachment_min` and `attachment_max` or a file with `file_min` and `file_max`, moves a different number of bytes in every session. One session then says little about the mean that the controller needs. In the first party `ftp` module the `retr_epsv` scenario moved between 46000 and 413000 bytes over 100 sessions. Run such a scenario with the session ids 1 to N under one seed and take the mean. A scenario without drawn sizes gives the same count in every session, and one session is enough, as in `hls`. The tests of `modules/ftp/tests/scenarios.rs` (`ESTIMATE_SESSIONS`, 100 sessions) and `modules/smtp/tests/scenarios.rs` (100 sessions) show the loop. A larger N narrows the mean and lengthens the test, since a session of the `ssh` module takes about 150 milliseconds in a debug build. The first party modules use between 10 and 100 sessions.
2. Add the payload bytes of every connection that the session opens, client to server and server to client. `modules/hls/tests/scenarios.rs` does this with `c2s().len() + s2c().len()` over `net.conns()`, and that sum includes the connections that a spawned flow opens, such as the FTP data connection and the RTP media of `sip_rtp`. A datagram counts with its whole payload (`modules/dns/tests/scenarios.rs`, `modules/ntp/tests/scenarios.rs`, `modules/sip_rtp/tests/scenarios.rs`). For a session over TLS count the plaintext above the TLS layer, which is what `Run::c2s` and `Run::s2c` hold in the `http1`, `http2`, `smtp`, `imap`, and `pop3` tests. The handshake and the record overhead in `TlsRun::wire_c2s` are not application payload. The plaintext before a STARTTLS upgrade counts.
3. Write the result into the manifest as an integer. Round to a figure that you can reproduce, since the value is an estimate and the calibration corrects the rest. The first party manifests of MOD-25 keep three significant digits from 1000 bytes up and the whole number below that, which stays within 0.5 percent of the measurement.
4. Keep the estimate true. A test like `default_parameters_move_the_expected_bytes` reads `expected_bytes` from the manifest, runs each scenario, and fails when the measured bytes differ from the estimate by 2 percent or more. `modules/hls/tests/scenarios.rs` has the single session form, and the other first party modules have the form of step 1 with a mean over `ESTIMATE_SESSIONS` sessions. The test also fails for a key that names no scenario, and with `--nocapture` it prints each mean, minimum, and maximum as `MEASURED` lines, which is where the number of step 3 comes from. Add the same test to your module, so that a change of a template or a fixture that alters the size of a session also changes the manifest. A guard of this kind found four hand written estimates of `sip_rtp` that were 12 to 21 percent above the measurement (MOD-25), so do not skip the measurement.

The schema defines the estimate as application payload bytes. It holds for the default parameters, so a test configuration that sets a size parameter such as `body_size` moves more or fewer bytes than the estimate says, and the calibration absorbs the difference. The Gate 2 suite sets such parameters, and it stages its module trees with the estimates of its own `mix.yaml` for them (`tests/gates/gate2/mix_config.py`, `stage_modules`), which replace the manifest value for the scenarios of the mix only.

Neither the registry nor `tools/contracts/validate.py` checks that a key names a scenario of the manifest, so a misspelled key surfaces only as the missing estimate of the scenario that you meant. Check the keys against the `scenarios` list yourself.

### Names

A module name must not collide with a template root. The grammar reserves `vars`, `session`, `last`, `profiles`, `dist`, `true`, and `false`, and the metric catalog reserves `all` as the label value that totals every application (`contracts/README.md`, `tools/contracts/validate.py`, `RESERVED_ROOTS` and `ALL_APPLICATIONS`). The registry refuses only `vars`, `session`, `last`, `true`, and `false` (`controller/internal/registry/manifest.go`, `reservedModuleNames`), so a module named `profiles`, `dist`, or `all` passes the install and fails the validator, which rejects all eight. Treat all eight as reserved.

Choose a name that no first party module uses. The agent hands a module the native functions of its own registry when the module has the name of a first party module and a manifest catalog that equals the catalog of that crate (`modules.rs`, `NativeFunctions::for_modules`), so a module named `dns` with the catalog of `dns` runs the code of the agent and not yours.

### Module types

The specification defines eight types (`docs/spec/specification.md`, Module types).

| Type | Status in the specification | What a third party can do with it today |
|---|---|---|
| `app` | v1 | Write scenarios. This is the type of every module that you can build without the project. |
| `transport` | v1 (TLS) | The TLS profiles belong to the agent, see [Transport](Transport.md). A scenario names a profile in `tls_upgrade`. |
| `host` | v1 | Host behavior belongs to the project, see [Hosts](Hosts.md). |
| `content` | v1 | The first party `content` module renders bodies with a native crate. A module outside the repository has no way to add native functions. |
| `tunnel`, `strike`, `evasion`, `fuzzer` | Reserved | Not supported. |

The schema description names `tunnel`, `strike`, `evasion`, and `fuzzer` as reserved. The schema accepts them and no reader rejects them, so a manifest of such a type validates and installs, and no engine behavior exists for it. The one rule that depends on the type is the `raw` action, which the validator and the contract check of the engine admit only in `strike` and `fuzzer` modules (`tools/contracts/validate.py`, `RESERVED_ACTION_TYPES`, and `engine/fps-flow/src/check.rs`). The interpreter does not run it yet: a runnable program with `raw` fails to load as an unsupported action (`engine/fps-flow/src/program.rs`) until ENG-29. A strike scenario with `raw` therefore validates and installs, and the agent refuses to run it.

### Licenses

The registry parses `license` as an SPDX expression and accepts it only when the content may be distributed with GPL-3.0 code. Every AND operand must be allowed, at least one operand of each OR must be allowed, and every identifier and exception in the expression must be on the list, so `MIT OR LicenseRef-acme` fails as unknown (`controller/internal/registry/license.go`, `docs/wiki/Controller.md`, Allowed licenses). The allowed identifiers are the following.

- Copyleft licenses GPL-3.0-only, GPL-3.0-or-later, GPL-2.0-or-later, LGPL-3.0-only, LGPL-3.0-or-later, LGPL-2.1-only, LGPL-2.1-or-later, AGPL-3.0-only, AGPL-3.0-or-later, and MPL-2.0.
- Permissive licenses Apache-2.0, MIT, MIT-0, X11, ISC, 0BSD, Zlib, BSL-1.0, BSD-2-Clause, BSD-3-Clause, Artistic-2.0, Unicode-3.0, Unicode-DFS-2016, and CDLA-Permissive-2.0.
- Content licenses Unlicense, CC0-1.0, CC-BY-4.0, and CC-BY-SA-4.0.

The exceptions `LLVM-exception`, `Classpath-exception-2.0`, and `GCC-exception-3.1` may follow `WITH`. Identifiers match without regard to case, the operators `AND`, `OR`, and `WITH` must be upper case, and the bare `GPL-3.0` and the `+` suffix are unknown. The validator applies the same parser and the same lists (`tools/contracts/validate.py`, `license_problem`), and a test compares them with the Go source, so a module with an unlisted license fails the validator and not only the install.

### Provenance

`provenance.source` is required and takes `synthetic`, `capture`, or `specification`. Content that you wrote is `synthetic`, and content that follows a public specification is `specification`. A module whose source is `capture` must also carry a `consent` note and `scrubbed: true`, and the schema rejects it otherwise (the `if` and `then` of `provenance`). The optional fields `capture_sha256`, `scrub_rules`, and `scrubbed_at` record the scrub, and `python3 -m capture scrub` writes them (`capture/scenario.py`, `provenance_record`). The project rule is that module content is synthetic or carries a documented license in the provenance field (Coding Standards, Modules).

## Scenarios

A scenario is a YAML file with the keys `scenario` (an identifier), `description`, `vars`, `profiles`, and `flows` (`contracts/schemas/scenario.schema.json`). A scenario name is unique within its module (`check_module_dir`), and a test configuration names a scenario as `<module>/<scenario>` or as a bare name that is unique among the installed modules (`contracts/schemas/test-config.schema.json`, `profile.mix[].scenario`). The controller resolves a scenario against the latest installed version of each module (`controller/internal/wiring/adapters.go`).

`vars` holds strings, numbers, and booleans, and a template reads them as `vars.<name>`. A test overrides them through the manifest `parameters`.

### Flows

Each flow runs one session, and the client side and the server side each run their action list in order. A flow fails when an action fails or times out (`contracts/README.md`, Flow program semantics).

| Key | Meaning | Rule |
|---|---|---|
| `id` | Identifier, unique in the scenario | `validate.py` |
| `module` | The module that owns the flow | It is this module or a module in `requires` |
| `l4` | `tcp` (default) or `udp` | |
| `after` | A flow id or a list of ids that must complete first | No cycle, no unknown id, and the flow sees the variables that the earlier flows bound |
| `spawned` | True when the flow starts only through a `spawn` action | A spawned flow no `spawn` names is a warning |
| `transport` | `{module: tls, profile, server_name?}`, TCP only | Sugar for a `tls_upgrade` after the opener of each side |
| `client`, `server` | Action lists. `client` is required. | A client list starts with `connect` and a server list with `accept` |

### Actions

An action is a mapping with exactly one key.

| Action | Fields | Notes |
|---|---|---|
| `connect` | `port` (integer or template), `host` (template, optional) | Without `host` the engine picks an address from the server pool |
| `accept` | `port` | Listens and accepts one session |
| `send` | A template | Renders and writes the bytes |
| `expect` | One of `pattern`, `bytes`, `min_bytes`, plus `timeout_ms` and, for UDP, `max_datagram` | `pattern` consumes through the first match, `bytes` consumes exactly N, `min_bytes` waits for N and consumes everything buffered. The default timeout is 10,000 ms. The consumed bytes become `last`. |
| `delay` | `ms`, or `dist` with `exponential`, `uniform`, `lognormal`, or `pareto` and their parameters | `lognormal` parameters apply to the natural log of the delay in `unit`, which defaults to seconds |
| `set` | A mapping from variable names to templates | The rendered bytes parse as text with the rules of a command line override, so `42` binds an integer |
| `loop` | `count` (integer or template) and `do` | |
| `branch` | A list of at least two arms with `p` and `do` | The probabilities must sum to 1 |
| `spawn` | `flow` | The named flow must declare `spawned: true` |
| `tls_upgrade` | `profile` (required), `role`, `server_name` | TCP only. Wraps the running session in TLS, and every later action runs over TLS. |
| `close` | `fin`, `rst`, or `half` | `fin` shuts down the write side and drains until the peer closes. `rst` aborts. `half` shuts down the write side and goes on. |
| `raw` | `hex` | Reserved for `strike` and `fuzzer` modules, so do not use it |

The sources are `contracts/schemas/scenario.schema.json` for the fields and `contracts/README.md` for the semantics. The validator adds the semantic rules that the schema cannot express (`check_scenario`). It reports a duplicate flow id, an unknown or cyclic `after`, a spawn of a flow that is not `spawned`, a branch whose probabilities do not sum to 1, a uniform delay with `min_ms` above `max_ms`, a side that does not start with its opener, and `tls_upgrade` or `transport` in a UDP flow.

UDP flows follow six datagram rules (`contracts/README.md`, Datagram rules for UDP). Each `send` writes one datagram of at most 65,507 bytes, and each `expect` reads exactly one datagram and consumes all of it. The stop rule checks that one datagram, and `max_datagram` bounds its size. `timeout_ms` bounds the wait for one datagram, and a session receives only the datagrams of its peer. The engine never reorders, deduplicates, or retransmits. The validator rejects `max_datagram` on a TCP flow and a fixed size that exceeds the limit on a UDP flow (`_check_expect`).

### Templates

A template is a string with zero or more `{{ expression }}` placeholders, and the engine copies the text outside them byte for byte. The grammar is in `contracts/README.md` (Template grammar). The rules that cause most mistakes follow.

1. A placeholder ends at the first `}}`, so a string literal cannot contain `}}` or a single quote, and strings have no escape sequences. Use `hex('...')` for binary content.
2. Positional arguments come before keyword arguments, and a trailing comma is a syntax error.
3. Calls nest at most 8 levels deep (`MAX_CALL_DEPTH`).
4. A function name with a module prefix calls a template function of a module. The module must be the one that ships the scenario or one in its `requires` list.
5. A template may refer to `last` only when an `expect` always runs earlier on the same side. An `expect` inside a `loop` counts only when `count` is a positive integer literal, and an `expect` inside a `branch` counts only when every arm has one.

The references and the core functions are these (`contracts/README.md`, `tools/contracts/validate.py`, `CORE_FUNCTIONS` and `DIST_FUNCTIONS`).

| Name | Type | Value |
|---|---|---|
| `vars.<name>` | type of the variable | A scenario variable, after the parameter overrides |
| `session.id`, `session.flow` | int, text | The session sequence number and the flow id |
| `last` | bytes | The bytes that the previous `expect` of the same side consumed |
| `profiles.<name>` | text | The entries of a scenario profile, rendered as `Name: value` lines that end with CRLF, in declaration order |
| `fill(n)` | text | `n` printable bytes from the seeded generator |
| `hex(s)` | bytes | The bytes that the hexadecimal string `s` encodes, which needs an even number of digits |
| `rand_int(a, b)` | int | A uniform integer in the range `a` to `b` from the seeded generator |
| `int_bytes(value, width, endianness)` | bytes | An unsigned integer as exactly `width` bytes (1 to 8), `'big'` by default or `'little'` |
| `now(format)` | text or int | The wall clock as `'http_date'`, `'epoch_s'`, or `'epoch_ms'`. Golden tests must pin the clock. |
| `dist.pareto`, `dist.lognormal`, `dist.exponential`, `dist.uniform` | int | A draw from the seeded generator, limited by `cap` |

Values have the types `int`, `float`, `boolean`, `text`, `bytes`, and `any`. An argument binds a parameter of the same type, a `float` parameter also takes an `int`, a `bytes` parameter also takes `text`, and `any` matches everything. A variable that only a `set` action binds has the type `any`.

A variable that a `set` action binds lives in the scenario instance. A flow that runs `after` another sees its variables, and the client sides and the server sides do not share variables during a run, so each role binds a value that both need from its own `last` (`contracts/README.md`, Variables across `after`).

### Spawned flows, profiles, and TLS

A flow that declares `spawned: true` starts through a `spawn` action. The first party `ftp` scenarios use it for their data connections (`modules/ftp/`, `docs/wiki/Modules.md`). The agent runs a scenario with several flows as an instance, and the entry flow, which is the first flow, must not be spawned (`docs/wiki/Agent.md`, Scenarios with several flows).

A `profiles` block holds named header sets, which a template reads as `profiles.<name>` (`contracts/examples/modules/http1/scenarios/web_profile.yaml` shows one). The validator reports a reference to an undeclared profile.

A flow gets TLS with the `transport` keyword or with `tls_upgrade`. The profile names a TLS profile that the run builds, such as a browser profile, and the scenario cannot define one. The profile names that exist belong to the agent ([Transport](Transport.md#browser-profiles-and-ja4) and [Agent](Agent.md#tls-profiles)).

## Template functions of your own

A scenario that needs computation which templates cannot express, such as a checksum or a key exchange, calls a function that a module implements. This is where a third party meets the limits of the project.

### What the manifest says

A module lists its functions under `functions`, each with a `name`, a `summary` of 1 to 200 characters, and `params` in positional order. A parameter has a `name`, a `type`, and an optional `default`, and a parameter without a default is required. A `bytes` default is a string of hexadecimal digit pairs. The validator reads this list to check every call without running code, and it reports an unknown function, a missing required parameter, an unknown or repeated keyword, too many positional arguments, and an argument whose type cannot bind its parameter (`tools/contracts/validate.py`, `_check_call`). The manifest is the single catalog, and the engine rejects a module whose implementation returns another catalog (`contracts/README.md`, Template function catalog).

A manifest that declares `functions` together with an `extension` must list the `render` hook, and both the validator and the registry report an error otherwise (`check_functions`, `ParseManifest`).

### Rust crates are first party only

Decision D10 lets a first party module implement its functions as a Rust crate in `native/` that the agent links at build time. The record has the status Proposed. It says that only modules in this repository qualify, that the owner approves each addition to the module table, and that every other module, and any module installed from the registry, runs as a WebAssembly component (`docs/decisions/0010-native-extensions.md`). The agent follows it. A module gets native functions only when its name is in the static `REGISTRY` of `engine/fps-agent/src/modules.rs` and its manifest catalog equals the catalog of the crate in names, parameter order, types, and required parameters. A call into any other module that has no extension fails the session, and the run reports the module on standard error (`docs/wiki/Agent.md`, Standalone mode).

A third party therefore cannot add native functions. A module outside the repository that declares `functions` and ships no `extension.wasm` has calls that fail, so declare none unless you ship a component.

### WebAssembly extensions

The agent does load a WebAssembly component for a module. The component targets the world `extension` of `contracts/extension/fps-extension.wit`, which imports the engine calls `send`, `set-timer`, `cancel-timer`, `log`, `random-u64`, and `random-bytes`, and exports the catalog and the hooks (`contracts/extension/README.md`). Decision D3 chose WebAssembly so that faulty community code cannot crash an agent (`docs/decisions/0003-extension-sandbox.md`). What runs today is narrower than the interface.

| Point | Behavior | Rests on |
|---|---|---|
| File name | The agent compiles the file `extension.wasm` at the root of the module tree. The schema allows no other value for `extension.wasm` in the manifest (CON-30), and a module that ships the file needs no `extension` key for the agent to load it | `engine/fps-runtime/src/extension.rs` (`EXTENSION_FILE`), `modules.rs`, `contracts/schemas/module-manifest.schema.json` |
| Verification order | The tree digest and the signature are checked first, and the component compiles only afterwards, from the bytes that the digest covered | `docs/wiki/Agent.md`, WebAssembly extensions |
| Catalog check | The function names, parameter names and order, types, and required flags must equal the manifest, or `prepare` fails. Defaults are not compared. | `docs/wiki/Engine.md`, WebAssembly extensions in sessions |
| Hooks called | `render` and `on-close` only. `on-data` and `on-timer` are never called, and `send` and `set-timer` return `not-allowed` to the guest. | `docs/wiki/Engine.md`, Limits of this step |
| Limits | 20 million fuel units per hook call and 16 MiB of memory, with no setting to change them | `docs/wiki/Agent.md`, Cost |
| Failure | A trap, a hook error, or a limit ends only the calling session, with the failure reason `extension_error` | `docs/wiki/Engine.md`, Failures |
| Features | SIMD, relaxed SIMD, multiple memories, and 64 bit memories are off | `docs/wiki/Engine.md`, The WebAssembly extension host |

The repository ships no guest SDK, no build instructions for a component, and no example component. The only component in the tree is a hand written WAT test file (`engine/fps-wasm/tests/support/mod.rs`). We have not verified that an authoring toolchain produces a component that the host accepts, so this page gives no procedure. A module with an extension is a module that you must build and test against the WIT file yourself.

### Settings

A third party module cannot receive settings today. The controller validates `modules.<name>.settings` against the `settings` schema of the manifest (`controller/internal/compiler/settings.go`), and the agent then refuses settings for every module that has no native functions in its registry, whether or not the module ships an extension (`modules.rs`, `for_modules` and `refuse_settings`, and `docs/wiki/Agent.md`, Module settings). Leave `settings` out of the manifest, and use `parameters` for values that a test sets.

## Fixtures and realism

A fixture is a capture of the traffic that a module generates, together with the labels that say what the traffic is meant to be. The realism runner scores it against the checks of [Realism](Realism.md).

The manifest field `fixtures` lists a `capture` file and the classifier names that are expected. The validator checks that the capture exists, and nothing reads `expect` (`check_module_dir`, and a search of `realism/` and `tools/`). The expectation that counts is the labels file, which first party modules keep as `fixtures/expected.yaml`. This is the file of the `dns` module, which `realism/schema/labels.schema.json` defines.

```yaml
labels: fps-realism-labels
version: 1
threshold: 0.99
sessions:
  - application: dns
    match: {transport: udp, server_port: 53}
  - application: dns
    match: {transport: tcp, server_port: 53}
applications:
  dns: {ndpi: [DNS]}
```

Each session rule names the intended application of the sessions that its `match` fields select, and `applications` maps the application to the names that nDPI may report. A section turns on a further check when the labels file has it, as the table shows (`docs/wiki/Realism.md`, `realism/README.md`).

| Section | Check | What it needs |
|---|---|---|
| (always) | `conformance` | At least 99 percent of the labelled sessions carry the intended nDPI label |
| (always, `--parsers`) | `parser_errors` | tshark or Zeek reports no parser error |
| `tls` | `encryption` | Sessions decrypt with the embedded keys, and each ClientHello has the JA4 of its profile |
| `content` | `content` | No application repeats more than 1 percent of its payloads |
| `structure` | `session_structure` | The connections of each instance follow the `after` and `spawn` edges of the scenario |
| `temporal` | `temporal` | Arrival times, sizes, and packet gaps agree with the target distributions in a KS test |
| `mix` | `mix_accuracy` | The achieved mix matches the configured weights |
| `transport` | `transport_behavior` | The SYN options match the OS profile of the sending host |
| `messages` | `messages` | Sessions show the expected number of requests, responses, commands, or messages |
| `hosts` | `host_identity` | The host count and the address and MAC pairs make sense |

For a module whose sessions run inside TLS, `python3 -m realism decrypted` scores the decrypted view with a labels file whose session rules match the server ports of the TLS sessions. Without `--view-port`, tshark decodes each TCP `server_port` that a rule names as TLS, so name the real server port of every TLS session in a rule (`realism/README.md`, Decrypted view, RLM-18). A TLS session on a port that no rule names is found only by the heuristic of tshark, which misses a ClientHello split after its first bytes.

You score a capture that you recorded yourself with the runner of the checkout. It needs `ndpiReader` and tshark, and Zeek is optional.

```bash
python3 -m pip install ./fps/realism
python3 -m realism score --pcap run.pcapng --labels expected.yaml --parsers tshark --output report.json
```

The exit code is 0 when every check passes, 1 when a check fails, and 2 when the capture, the labels, or a tool cannot be read or run (`realism/README.md`, Command).

A fixture score can list a parser finding as not counted. The operating system picks the client port of each session at random, and tshark decodes the traffic of about 900 TCP and UDP ports with the dissector that is bound to the port (UDP 41170 is Manolito, and TCP 44322 is the Performance Co-Pilot proxy). When a session of your module uses such a port as its client port, tshark reports malformed packets for payload that is well formed. The runner leaves a tshark finding out of the count when the client port of its session is bound to a dissector for the transport and the server port is not bound to the same dissector. The `parser_errors` entry of the report lists it under `not_counted` with the rule `port_collision` and the dissector, and the job summary of a module says `N findings not counted (port collision)`. A finding that is not counted does not change `errors_total` or the verdict. You do not change your traffic or your labels for it, and a recording that you make again draws another client port. Findings of Zeek, findings in a session whose client port is free, and findings that belong to no session always count, so a malformed message of your module still fails the check. A runner older than report format 0.18.0 counts these findings, so score with the runner of the checkout that you build against (`realism/README.md`, Parser errors).

A module whose traffic is malformed on purpose, such as a scenario that sends an invalid request line, produces parser findings that you want, and the check must not count them as faults. Declare them in the label rule of the session. A rule takes `expect_findings`, which lists per parser the items that its sessions must produce.

```yaml
sessions:
  - application: http1
    match: {transport: tcp, server_port: 8080}
    expect_findings:
      zeek: [bad_HTTP_request]
      tshark: ["http: Illegal characters"]
```

A Zeek item is the name of a weird, as `weird.log` prints it in its `name` column. A tshark item is the protocol of the frame with the expert item, which is the last name in the `frame.protocols` field of that frame (`http`, `dns`, `tcp`), and optionally a colon, a space, and a part of the expert message, which the first expert item of error severity of the frame must contain. List exactly what a recording of your scenario shows: take the items from the `errors` of the `parser_errors` entry of a first score that has no declaration, and check each against the traffic that you meant to send. A declared finding does not count, and the entry lists it under `expected_findings`. A declared item that no session of the rule produced fails the check and is listed under `missing_findings`, so a scenario that stops being malformed fails, and a finding that no rule declares fails as before, so a second fault in the same scenario does not hide behind the first. The first rule that matches a session applies, so declare the items on the rule that the sessions of the malformed scenario match first, and keep well formed scenarios on their own rules. One finding of any session of the rule meets an item. When a parser is not installed, its items are listed under `unchecked_expectations` and do not fail the check, so score with `--parsers tshark,zeek` to require both (`realism/README.md`, Parser errors).

What the repository does for first party modules goes further than what it does for yours. A first party module records its capture with `fixtures/capture.sh` in the QA-01 namespace harness (`tests/harness/netns.sh`), which needs root, a checkout, and a peer program built from the native crate (`modules/dns/fixtures/capture.sh`). The `realism` workflow runs a module only when it has `native/Cargo.toml`, `fixtures/capture.sh`, and `fixtures/expected.yaml` (`realism/modules.py`, `docs/wiki/Realism.md`, Module realism job). A module outside the repository therefore gets no recording in CI, and the project rule that fixtures come from harness runs and never from third party captures (Coding Standards, Modules) applies to modules that join the repository. The template has no `fixtures/` directory for this reason, and its native crate is not selected by the job.

## Validate

Run the validator on every change. It checks three things (`tools/contracts/validate.py`).

1. **Schemas.** The manifest and every scenario that it lists validate against their JSON Schemas, and the `parameters` value is itself a valid JSON Schema (`check_module_dir`).
2. **Module rules.** The directory name equals `name`, the name is not reserved, the license is on the allow list, every file and directory of the tree has a path of letters, digits, `_`, `.`, and `-`, the tree holds no symbolic link, no module requires itself, function and parameter names are unique, parameter defaults match their types, every path that `scenarios`, `assets`, `fixtures`, and `extension.wasm` names is a regular file of the tree, and every key of `expected_bytes` names a scenario of `scenarios`.
3. **Scenario rules.** The rules of [Actions](#actions) and [Templates](#templates), including each module call against the catalog of its manifest. A call into a required module whose manifest the run does not hold is a warning and stays unchecked (`_resolve_call`).

The validator applies the rules that the registry applies to a tree: the license, the file names, links, and named files (`tools/contracts/validate.py`, `license_problem`, `tree_problems`, `names_regular_file`). It is stricter than the registry for the reserved names `profiles`, `dist`, and `all` and for the keys of `expected_bytes`. It does not check the `settings` schema or the signature, and it never runs extension code. Check these by hand before you package.

## Package

A module package is a tar archive, optionally gzip compressed, with `manifest.yaml` at its root (`controller/internal/registry/archive.go`, `unpack`). The rules follow.

- Only directories and regular files are allowed, and every path is relative, uses only `relpath` characters, has no `.` or `..` segment, and appears once.
- The unpacked size is at most 256 MiB and the file count at most 10,000 by default (`DefaultLimits`).

```bash
tar -C mods -czf acme_ping-0.1.0.tar.gz acme_ping/
```

That command puts the module under a directory in the archive, and the registry wants the manifest at the root, so package with the directory as the working directory.

```bash
tar -C mods/acme_ping -czf acme_ping-0.1.0.tar.gz .
```

The entry `./manifest.yaml` is accepted because the registry strips a leading `./` (`unpack`).

### The tree digest

A module is identified by its tree digest, the value that plans carry and that a signature covers (`contracts/README.md`, Plan rules, rule 1). Every regular file contributes the line `<sha256 hex of the file>  <relative POSIX path>` and a newline, the lines sort by the bytes of the path, and the digest is `sha256:` followed by the SHA-256 of their concatenation. The digest covers file names and contents. It does not cover modes, owners, times, or empty directories (`controller/internal/registry/signing/archive.go`, `ArchiveDigest`). The checkout prints it for a directory.

```bash
python3 fps/tools/contracts/check_plans.py --modules mods --print-digests
```

The agent computes the same digest of the directory that it loads (`modules.rs`, `tree_digest`), so the controller, the agent, and your command agree when the trees are the same files.

## Sign

A controller that has a trust store refuses a module without a verified signature, unless the operator also sets `allow_unsigned_modules`. A controller without a trust store installs every module that passes validation and logs a warning (`controller/internal/registry/trust.go`, `TrustPolicy`; `docs/wiki/Controller.md`, Signature checks at install and in plans). Signing is therefore the operator's decision, and a signature helps you only when the operator has your public key in the trust store.

The repository has no command that generates keys or signs a module, and `docs/wiki/Controller.md` lists that as a follow up. The format is simple enough to follow with any Ed25519 library.

1. Compute the tree digest, as above.
2. Sign the UTF-8 bytes of the digest string, including the `sha256:` prefix, with an Ed25519 private key. The signature is 64 bytes (`signing/keys.go`, `SignDigest`).
3. Derive the key id. It is `ed25519:` followed by the hex of the first 16 bytes of the SHA-256 of the 32 byte raw public key, so it has 40 characters (`signing/keys.go`, `KeyID`).
4. Send the public key to the operator. The trust store is a JSON file with `version` 1 and a list of keys, each with `key_id`, `public_key` (standard base64 of the 32 raw bytes), and `status` (`active`, `retired` with `retired_after`, or `revoked`) (`signing/truststore.go`, `docs/wiki/Controller.md`, Trust store).

The signature is not part of the module tree, so signing changes no digest. Keep the private key out of the module directory and out of every repository.

## Upload

The controller keeps modules in a local registry, a directory with `index.json` and one tree per installed module (`controller/internal/registry/doc.go`). An operator installs a module with `POST /v1/modules`, the operation `installModule`. It needs a token with the scope `admin` (`contracts/openapi/fps-api-v1.yaml`). The body is the archive with the content type `application/octet-stream`.

```bash
curl -sS -X POST "$FPS_URL/v1/modules" \
  -H "Authorization: Bearer $FPS_TOKEN" \
  -H "Content-Type: application/octet-stream" \
  -H "X-Module-Signature-Key-Id: $KEY_ID" \
  -H "X-Module-Signature: $SIGNATURE_B64" \
  --data-binary @acme_ping-0.1.0.tar.gz
```

The two signature headers go together or not at all, and the signature is standard base64 of the 64 byte value (`controller/internal/registry/http.go`, `InstallModule`). The generated Python client sends only the body, because its `install_module` method has no header parameters (`sdk/python/fixingpoint/_generated.py`, and [SDK](SDK.md)), so use curl or another HTTP client for a signed upload.

| Status | Meaning |
|---|---|
| 201 | Installed. The `Location` header names `/v1/modules/<name>/<version>`. |
| 400 | One signature header is missing, the signature is not base64, or the archive is broken |
| 403 | The signature is refused or missing under a trust store, or the token lacks the scope |
| 409 | The name and version are already installed. A version is immutable, so a new build needs a new version. |
| 413 | The archive exceeds the limits |
| 422 | The manifest or the tree is invalid. The problem lists each finding with a pointer. |

The registry checks an install in a fixed order (`registry.go`, `checkTree`, and `docs/wiki/Controller.md`, Install).

1. **Archive.** Only files and directories, with `relpath` paths, within the size limits.
2. **Manifest schema.** `manifest.yaml` validates against an embedded copy of `module-manifest.schema.json`.
3. **License.** The expression is on the allow list.
4. **Semantic rules.** Function names are unique, `render` is listed when functions and an extension exist, and the name is not reserved. `validate.py` reports each of them.
5. **Files.** Every named scenario, asset path, extension, and fixture capture exists as a regular file.
6. **Scenarios.** Each scenario file, and each file of `scenarios-pending/`, validates against the scenario schema, and its `profiles` and `dist` references follow the rules (`controller/internal/registry/scenario.go`). The registry does not check module function calls, which the validator and the agent own.
7. **Digest.** The registry computes the tree digest and records it in the index, with the signature when the install had one.

`GET /v1/modules` lists the installed modules, `GET /v1/modules/{name}/{version}` returns a manifest, and `DELETE /v1/modules/{name}/{version}` removes one.

### Publishing

The repository supports no publication outside one controller. We found no remote registry, no index that a controller fetches, no `fps` command for modules, and no publishing step. The registry code can sign an index, which lets a client trust a list of modules, but it stores no serials, and the publisher tooling that would build and sign one is a follow up (`docs/wiki/Controller.md`, Signed index and Publisher tooling). If you want others to use your module, give them the archive, its digest, and your public key, and have each operator upload it to their own controller and copy it to their agents.

## How the agent loads a module

The agent never reads an archive and never fetches a module. The controller compiles a plan that lists each module by name, version, and tree digest, with the signature and the settings, and the agent already holds the module tree in its modules directory (`contracts/README.md`, Plan rules; `docs/wiki/Agent.md`, Signed modules). Placing the tree there is the operator's step. The end to end test installs the same trees in the controller and in the agent directory for this reason (`docs/wiki/Agent.md`, end to end level).

`modules::load` in `engine/fps-agent/src/modules.rs` runs these steps for each module of a plan, in the standalone mode and for a `prepare` command alike.

1. **Read the tree once.** It reads every file of `<modules directory>/<name>` in one pass, computes the tree digest, and keeps the bytes of `manifest.yaml` and `extension.wasm`. A symbolic link or a path that is not UTF-8 fails the module.
2. **Compare name and version.** The manifest must carry the name and the top level `version` that the plan names.
3. **Compare the digest.** The digest of the tree must equal the plan digest. No setting waives this.
4. **Judge the signature.** With `--module-trust-store FILE`, a module needs a valid signature over the digest, and a failure ends `prepare` with `ERROR_CODE_FAILED_PRECONDITION`. `--dev-modules` lets a failing module through with a warning. Without a trust store the agent checks nothing and logs one warning per plan.
5. **Compile the extension.** Only after steps 3 and 4 does the agent compile `extension.wasm`, when the module has one, and check its catalog against the manifest.
6. **Register native functions.** A module gets native functions only if its name is in the agent registry and its catalog equals the crate catalog. Settings for any other module end the run. The agent also registers the modules that a plan module requires and the plan leaves out, when its registry knows them.
7. **Compile the scenarios.** Each scenario source of the plan must match its digest and compile in the call scope of its module, and its `scenario` name must equal the reference. Mix parameters then override the variables, and constants fold.

A module that fails a step fails the plan, because a run without it would measure another test. The failure reasons are in [Agent](Agent.md#standalone-mode).

You can run a plan without a controller with `fps-agent run --plan FILE --modules DIR` (`docs/wiki/Agent.md`, Standalone mode). The plan is JSON in the canonical mapping of `fps.agent.v1.Plan` and carries your module digest, which `check_plans.py --print-digests` prints. The example `contracts/examples/plans/http_dns_smoke.json` shows the shape, and `check_plans.py` validates a plan file.

## Before you publish

Work through this list for a new version.

- The directory name equals `name`, and `version` is new and stands on a line of its own with no trailing comment.
- `validate.py` reports 0 errors and 0 warnings.
- `license` is on the allow list, which `validate.py` checks, and `provenance` states the real source.
- `parameters` names only variables of the scenarios, and `expected_bytes` matches the default parameters.
- The manifest declares `functions` only when the module ships `extension.wasm`, and it declares no `settings`.
- The tree holds no symbolic link and no file name outside the `relpath` characters, which `validate.py` checks.
- The README names the scenarios, the parameters, and the results that you measured.

## Gaps

We recorded what the repository does not support yet, so that you do not look for it. Each item names the file that shows the gap.

1. **No out of tree publishing.** A registry is a local directory of one controller, there is no remote or shared registry, and no tooling builds or signs a registry index (`controller/internal/registry/doc.go`, `docs/wiki/Controller.md`).
2. **No signing command, and no signed upload in the SDK.** Keys and signatures need your own code, and `install_module` of the Python SDK cannot send the signature headers (`sdk/python/fixingpoint/_generated.py`).
3. **No distribution to agents.** The controller does not send module trees, so each agent needs the tree by other means (`docs/wiki/Agent.md`).
4. **No WebAssembly toolchain for authors.** The host exists, and the repository gives no SDK, example, or instructions. Only `render` and `on-close` run (`engine/fps-runtime/src/extension.rs`).
5. **No settings for third party modules.** The agent refuses them for a module without native functions (`engine/fps-agent/src/modules.rs`).
6. **Native crates are first party only**, and decision D10 still has the status Proposed (`docs/decisions/0010-native-extensions.md`).
7. **Declared and not enforced.** `engine_api` is read by nothing that compares it with an engine version, `assets` and `fixtures[].expect` are never read beyond the existence of a file, and `provides` and the version of `requires` document only (`controller/internal/registry/manifest.go`, `tools/contracts/validate.py`). A search of `controller/`, `engine/`, and `realism/` found no other reader. The module types `tunnel`, `strike`, `evasion`, and `fuzzer` are reserved in the schema text and no reader rejects them.
8. **The registry is looser than the validator in two places.** It accepts the names `profiles`, `dist`, and `all`, which the validator rejects, and it does not check that a key of `expected_bytes` names a scenario (`controller/internal/registry/manifest.go`, `reservedModuleNames`, and `registry.go`, `checkTree`). Both are safe for the author, because a module that validates also installs.
9. **No realism job for third party modules.** The job selects modules that have a native crate and a recording script, and the harness records only the traffic of first party peers (`realism/modules.py`).
10. **The agent reads `version` by line.** A trailing comment on the line makes it differ from the plan (`engine/fps-agent/src/modules.rs`, `version_of`).

## Sources

The files below are the sources of this page. When one of them changes, the change updates this page.

| Topic | Source |
|---|---|
| Manifest and scenario contracts | `contracts/schemas/module-manifest.schema.json`, `contracts/schemas/scenario.schema.json`, `contracts/README.md` |
| Extension interface | `contracts/extension/fps-extension.wit`, `contracts/extension/README.md` |
| Validation | `tools/contracts/validate.py`, `tools/contracts/check_plans.py`, `tools/contracts/requirements.txt` |
| Registry, upload, digest | `controller/internal/registry/` (`registry.go`, `archive.go`, `digest.go`, `http.go`, `manifest.go`, `license.go`, `scenario.go`), `contracts/openapi/fps-api-v1.yaml` |
| Signing and trust | `controller/internal/registry/signing/`, `controller/internal/registry/trust.go`, [Controller](Controller.md) |
| Agent loading | `engine/fps-agent/src/modules.rs`, [Agent](Agent.md) |
| Native crates and the sandbox | `docs/decisions/0010-native-extensions.md`, `docs/decisions/0003-extension-sandbox.md`, [Engine](Engine.md) |
| Fixtures and labels | `realism/README.md`, `realism/schema/labels.schema.json`, [Realism](Realism.md), `modules/dns/fixtures/` |
| First party examples | [Modules](Modules.md), `modules/dns/`, `contracts/examples/modules/` |
| Starting point | `modules/_template/`, `mk/mod-template.mk` |
| Related pages | [Contracts](Contracts.md), [SDK](SDK.md), [Coding Standards](Coding-Standards.md), [Testing](Testing.md) |
