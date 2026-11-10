# CON-19 Contract gaps found by CAP-02 and RLM-06

Contract. Scenario contract (the core template functions and the rule for variables across `after`), test configuration schema `contracts/schemas/test-config.schema.json`, metric catalog `contracts/metrics/catalog.yaml`, and the rules of `tools/contracts/validate.py`. The change takes findings of CAP-02 and RLM-06. The owner approved it.

Versions.

- The scenario contract moves from v0.5 to v0.6. The change adds two core functions and states a rule that the engine already follows, and it changes no keyword, so every v0.5 scenario keeps its meaning.
- The test configuration schema moves from v0.2 to v0.3. It gains four optional fields, so every v0.2 configuration stays valid.
- The metric catalog moves from 0.2.0 to 0.3.0. It gains one value of the label `property` and documents one value of the label `application`, and every metric keeps its name, type, unit, level, and labels.
- The module manifest schema (v0.3), the controller API (OpenAPI 0.7.0), and the agent protocol (0.2.3) keep their versions. The OpenAPI document refers to the test configuration schema by reference, and the `oasdiff` check of CON-16 replaces that reference by an opaque object, so it cannot see the change. The additive rule holds by construction, because every new property is optional.

## 1. The core functions `int_bytes` and `now`

CAP-02 turns observed length fields and dates into templates, and the contract had no function that writes an integer as bytes and none that reads a clock. The names follow `fill`, `hex`, and `rand_int`: lower case, snake case, no module prefix, positional or keyword arguments with the parameter names below. Neither function draws from the seeded generator, so a template that adds one leaves every other drawn value of the session unchanged.

`int_bytes(value, width, endianness='big')` returns bytes.

- `width` is 1 to 8. `endianness` is `'big'` (the default, network order) or `'little'`. `value` is an unsigned integer that fits the width, and an `int` is a signed 64 bit integer, so width 8 takes 0 to 2^63 - 1.
- A literal argument that breaks a rule is a validator error. A computed argument that breaks a rule fails the session at render time with the existing reason `template_error`. The engine never truncates, wraps, or encodes a negative value, because a length field that silently wraps hides a scenario bug.
- The result has exactly `width` bytes, so a modeled byte count does not depend on the value.

`now(format)` returns text or an int, by format. `format` is a required text argument.

| `format` | Result | Rendering |
|---|---|---|
| `'http_date'` | text | IMF fixdate of RFC 9110 section 5.6.7 in UTC, 29 bytes, for example `Tue, 10 Nov 2026 06:15:04 GMT` |
| `'epoch_s'` | int | Whole seconds since 1970-01-01T00:00:00Z |
| `'epoch_ms'` | int | Whole milliseconds since 1970-01-01T00:00:00Z |

The validator gives the call the result type that a literal format fixes, and `any` to a format that comes from a variable, so `fill(now('epoch_s'))` is valid and `fill(now('http_date'))` is not. An unknown literal format is an error.

Replays and goldens. `now` is the one core function that is not a function of the run seed and the session. The contract therefore says the following, and the full text is in the README.

- Each evaluation reads the clock. A scenario that needs one instant in several places binds it with `set`.
- The engine reads the clock through the same seam as its random source. A production run reads the wall clock of the agent that renders the template, in UTC, and the agents of a run share no clock. A harness, a golden test, or a replay that needs equal bytes pins the clock to a fixed instant (optionally with a fixed step per call). The instant is a parameter of the harness, and a golden file records it. A golden that does not pin the clock must not contain output of `now`.
- The length of the output does not depend on the instant between the years 2001 and 2286 (29 bytes, 10 digits, 13 digits), so a replay without a pinned clock still reproduces every byte count, which is what the CAP-02 replay job compares.
- The compiler never folds a call of `now`, as it never folds `fill` and `rand_int`.

We chose per call evaluation and a pinned clock over a per session snapshot, because a snapshot would need a place in the session state and would still differ between runs. The pinned clock is the same mechanism that tests of the engine need anyway.

## 2. Variables across `after`

Decision: a flow that runs `after` another flow sees the variables that the earlier flow bound. CAP-02 needs it, because a combined scenario that logs in in one flow and uses the token in the next otherwise has to use `spawn`. The engine already behaves this way (`ScenarioInstance` merges the variables of a completed side into the shared set and starts each later flow with a copy), and the validator already accepts a variable that a `set` binds anywhere in the scenario, so the change states the rule and changes no behavior.

The rule has five parts, written in full in the README section "Variables across `after`".

1. Visibility follows `after`, directly and through a chain, with the value that stood when the reading flow started.
2. A flow's own later actions and the flows that it starts with `spawn` see its bindings, a spawned flow up to the `spawn` action of its parent side.
3. Flows that `after` does not order have no defined order, so a flow must not read a variable that only such a flow binds, and the last to complete wins when two flows bind one name.
4. The two roles of a flow run on different agents and share no variables during the run. A variable of the client side reaches the client sides of later flows, and a variable of the server side reaches the server sides. Each role binds a value it needs from its own `last`. This is the choice the engine can support in a distributed run, because there is no channel between the agents for variables, and it is what CAP-02 needs: the client binds a token that it sends, and the server binds the token that it received.
5. A scenario instance starts from `vars` after the test parameter overrides, and bindings never reach another instance.

The scenario schema keeps its shape and its `after` description now names the rule. The validator does not check the order of flows against the bindings, so a read of a variable that only an unordered flow binds is not an error. A later change could add a warning.

## 3. Arrival and size distributions in the test configuration

RLM-06 takes target distributions from its labels file, because the test configuration has no arrival process or size distribution. The change adds, to `profile` and to every `profile.mix` entry, the optional fields `arrival` and `size`.

- `arrival` is an object with `dist` set to `constant`, `exponential`, `uniform`, `lognormal`, or `pareto`, and the parameters that the scenario `delay` uses: `ms` (with `tolerance_ms`), `mean_ms`, `min_ms` and `max_ms`, `mu` and `sigma` with `unit` (`s` or `ms`), and `alpha` and `scale_ms`.
- `size` is an object with the optional keys `request`, `response`, and `both`, each a distribution in bytes: `value` (with `tolerance`), `mean`, `min` and `max`, `mu` and `sigma`, and `alpha` and `scale`. The parameter names are the ones of the RLM-06 targets, so a declaration can be pasted into a labels file.
- An entry overrides the profile, and a mix entry that names a direction of `size` overrides that direction only.
- The validator rejects a uniform distribution whose minimum is above its maximum, at the pointer of the distribution.

The fields are declarations. They add no command for agents and do not change the plan. This keeps the change additive for the controller and the engine, and it matches the request, which asks for distributions that realism targets can follow. The controller still schedules arrivals from the load objective and the phases, and sizes still come from the scenarios and `params`. A later CTL change can compile `arrival` into the schedule, and the README says that a declaration that disagrees with the objective fails the realism check and not the validator.

## 4. `all` and `both` for the KS metrics

RLM-06 compares inter arrival times and payload sizes over all sessions and over both directions, which the catalog labels could not name.

- The label `application` documents the value `all`, a total over every application of the run. The label has no closed value list, because the values are module names, so the catalog documents the value and the validator reserves the module name: `tools/contracts/validate.py` rejects a manifest named `all`. No module of the repository has that name, and the rule is the same kind as the reserved template roots. A series with `all` sits next to the per application series, so a reader that sums over `application` leaves it out, and a criterion selects it with `where: {application: all}`.
- The label `property` gains the value `both`, the payload sizes of the two directions pooled, after the five values it had. It is a closed list, so the Rust mirror `LABEL_VALUES` in `engine/fps-stats/src/catalog.rs`, which a contract test compares with the catalog file, gains the same value. The catalog descriptions of `realism.ks_statistic` and `realism.ks_p_value` say that they take both.

## Compatibility

Every change is additive. A v0.5 scenario, a v0.2 test configuration, and a catalog 0.2.0 reader stay valid and keep their meaning. The one rejection that is new is the module name `all`, which no module uses.

Until a CTL package serves the new fields, a configuration that uses `arrival` or `size` passes the schema in the configuration store and then fails in the compiler, because `controller/internal/compiler/config.go` decodes with `KnownFields(true)`. No existing configuration uses the fields, so nothing that worked breaks, and the CTL follow up below closes the gap.

The controller embeds three copies, refreshed byte for byte, which the existing tests compare: `controller/internal/registry/schema/scenario.schema.json`, `controller/internal/store/schema/test-config.schema.json`, and `controller/internal/metrics/catalog.yaml`. `tools/contracts/test_contracts.py` also compares them, so the contract job catches a stale copy before the Go tests do.

Generated code. The OpenAPI document did not change, so `sdk/python/fixingpoint/_generated.py` is unchanged (`generate_client.py --check` passes), and `ui/src/api/schema.d.ts` is regenerated, because it inlines the test configuration schema and now holds the new `arrival`, `size`, and `size_distribution` types. The change needs no Go regeneration, because `controller/internal/api/api.gen.go` refers to the test configuration schema by reference and holds no copy of its properties.

## Follow up work, outside the catalog paths

- ENG: implement `int_bytes` and `now` in `fps-flow` (parser, checker, renderer, and the folding rule that `now` never folds) with the clock seam and a pinned clock for tests, and make the interpreter's `template_error` cover the render time range errors. Add shared fixtures to `engine/fps-flow/tests/fixtures/verdicts.yaml` and the value rules of this entry when the engine accepts the functions, because the fixtures run in the Rust test and in `test_fixtures.py`. The rule for variables across `after` needs no engine change, and an ENG test that pins it (a second flow reads a variable that the first flow bound) is worthwhile.
- CTL: `controller/internal/registry/scenario.go` has the table `coreResult` with `fill`, `hex`, and `rand_int`, which only types the result of a core call for an outer `dist` call. A call of `int_bytes` or `now` gets the type `any` and is not rejected, and the table can gain `int_bytes` (bytes) and `now` (the result type of its literal format) to match the validator. The compiler of the test configuration (`controller/internal/compiler/config.go`, `Profile` and `MixConfig`, which decode with `KnownFields(true)`) must accept `arrival` and `size`, and may compile `arrival` into the schedule. The Go types of the test configuration (`controller/internal/store`) and the UI config editor (`ui/src/config`) should show the new fields. The controller metrics package may expose the `all` and `both` label values, and nothing breaks without it.
- RLM: read `profile.arrival` and `size` as the targets of the `temporal` check, with the mappings of the README, emit the `all` and `both` series with the catalog labels, and widen `realism/schema/realism-report.schema.json` (the `property` enum of the temporal metrics and the `application` value) for them.
- CAP: the CAP-02 generator can emit `int_bytes` for length fields, `now('http_date')` and `now('epoch_s')` for dates and timestamps, and `after` instead of `spawn` for combined scenarios, once the engine accepts the functions. Its modeled byte counts stay exact.
- Modules and docs outside `contracts/`: `docs/wiki/Capture.md` says that there is no clock function and that combined scenarios use `spawn`, `docs/wiki/Engine.md` lists the core functions, and `docs/wiki/Realism.md` says the test configuration has no arrival process. A later package for each page can reword those passages.
