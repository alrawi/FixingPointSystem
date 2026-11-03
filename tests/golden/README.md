# Golden file helpers

These helpers compare test output with golden files under `testdata/` and rewrite a file only when `UPDATE_GOLDEN=1` is set. [Testing](https://github.com/alrawi/FixingPointSystem/wiki/Testing#golden-files) explains how to use each one.

| Path | Language | Entry point |
|---|---|---|
| `rust/` | Rust crate `fps-golden` | `fps_golden::assert_golden!("name.txt", output)` |
| `go/` | Go module `github.com/alrawi/FixingPointSystem/tests/golden/go` | `golden.Assert(t, "name.txt", output)` |
| `python/` | pytest fixture in `fps_golden.py` | `golden.check("name.txt", output)` |

We run the linters and tests of all three with one command.

```bash
make lint-qa test-qa
```
