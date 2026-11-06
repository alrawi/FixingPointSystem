# CON-10 Behavior profiles and distributions in the scenario contract

Contract. Scenario schema `schemas/scenario.schema.json`, the template grammar of `contracts/README.md`, and the example module `examples/modules/http1`. The change applies decision D12 item 18.

Versions. The scenario contract moves from v0.2 to v0.3. The module manifest schema stays at v0.3. Every change is additive. A v0.2 scenario has no `profiles` key and no `dist` call, so it validates and means what it meant before. No existing key, type, or pattern changed.

Compatibility.

- Scenario schema consumers. The new optional top level key `profiles` is the only schema addition. A consumer that validates against the v0.2 schema rejects a v0.3 scenario that uses it, because the root has `additionalProperties: false`, and the engine template parser does not know the two roots, so the engine and the controller cannot run such a scenario until a package teaches them the syntax. The specification example is therefore Phase 2 syntax, as D12 item 18 says.
- Template grammar. `profiles.<name>` and `dist.<name>(...)` were invalid in v0.2 and are valid in v0.3. No valid v0.2 template changes meaning.
- Reserved names. `profiles` and `dist` join `vars`, `session`, `last`, `true`, and `false` as names that a module may not take. No shipped module uses either name. A third party module with one of these names must rename, which is the only incompatibility, and it affects a name that v0.2 did not yet reserve.
- Engine and controller. Neither compiles the new syntax yet. The example that uses it lives under `scenarios-pending/`, which the manifest does not list and the engine tests do not read. The Rust template parser and the controller compiler do not change in this package.

Changes.

- `profiles` is an optional top level object. Each key is a profile name that follows the identifier pattern, and each value is a non empty object that maps an entry name (`[A-Za-z0-9][A-Za-z0-9_-]*`) to a string, number, or boolean. A string value holds no CR or LF, so a profile cannot inject a header line.
- The template reference `profiles.<name>` has type `text`. It renders each entry, in declaration order, as `Name: value` followed by CRLF. A module function takes it as an ordinary text argument. The validator reports a name that the scenario does not declare, and a bare `profiles` or a reference with more than one field.
- The functions `dist.pareto(alpha, scale, cap)`, `dist.lognormal(mu, sigma, cap)`, `dist.exponential(mean, cap)`, and `dist.uniform(low, high)` draw from the run's seeded generator, so one seed repeats a run. Each returns an `int`, which is the draw rounded to the nearest integer and limited to `cap`. The parameters take `float` and bind an `int` argument too. `cap` takes an `int`, is optional, and defaults to 2,147,483,647, so a heavy tailed draw cannot overflow a size or a count. The functions need no module manifest.
- The validator checks the distribution name, the argument types, the required parameters, and the range of literal arguments (`alpha`, `scale`, `sigma`, `mean`, and `cap` above 0, `low` and `high` at least 0, and `low` not above `high`).
- The `delay` action keeps its object forms with a `dist` field. A template such as `delay: { ms: "{{ dist.lognormal(3.0, 0.8) }}" }` is the call form of the same idea.
- The `http1` manifest adds the optional parameter `headers` (type `text`, default empty) to `request`, so the specification call `http1.request('GET', '/', headers=profiles.chrome)` has a parameter to bind. A call without it behaves as before. The digest of the `http1` module changed, because the manifest and the new example file changed, so the example plan `plans/http_dns_smoke.json` and the controller compiler golden plans carry the new digest.
- The new example `examples/modules/http1/scenarios-pending/web_profile.yaml` is the specification example in v0.3 syntax. The specification example also calls `content.html(size=...)`, which no version defines, so the example builds the body with `fill`. The validator now checks every `scenarios-pending/*.yaml` of a module, and the manifest does not list them.
- Tooling. `tools/contracts/validate.py` knows the two roots, the `dist` catalog, and the pending directory, and `tools/contracts/test_contracts.py` covers them.

Grammar text for `contracts/README.md`, which the coordinator folds because the file is outside this package. In the grammar, `reference` gains `| "profiles" "." ident` and `name` gains `| "dist" "." ident`. Rule 5 lists `profiles` and `dist` among the reserved names, and the reference table gains the rows `profiles.<name>` (text) and `dist.<name>(...)` (int). The file table row for the scenario schema reads v0.3.
