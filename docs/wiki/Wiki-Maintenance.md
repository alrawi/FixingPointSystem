# Wiki Maintenance

Agents maintain this wiki as a compiled record of the project, following the [llm-wiki pattern](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f) by Andrej Karpathy. Knowledge enters once, is filed into the right pages, and stays current, so an agent that joins later reads a few pages instead of the whole history.

## Three layers

We separate three layers.

| Layer | Contents | Who edits |
|---|---|---|
| Raw sources | The [specification](https://github.com/alrawi/FixingPointSystem/blob/main/docs/spec/specification.md), decision records, contracts, gate reports, merged handoff files | Owner approved pull requests only; wiki agents read them |
| Wiki | The pages in `docs/wiki/`, published here | Agents, through pull requests |
| Schema | [`AGENTS.md`](https://github.com/alrawi/FixingPointSystem/blob/main/AGENTS.md) and this page | Owner and agents together, with owner approval |

The source of every page is `docs/wiki/` in the repository. The `wiki` workflow publishes that directory to this wiki after each merge to `main`. Edits made directly in the GitHub wiki are overwritten, so agents and humans edit the repository copy.

## Navigation files

[Home](Home.md) is the index. It lists every page with a link and a one line summary, grouped by category. Agents read Home first. Only the coordinator edits Home, after each batch merges, so parallel pull requests never conflict on it.

[Log](Log.md) is the chronological record. Each entry is one file in `docs/wiki/log/`, named `<YYYY-MM-DD>T<HHMM>-<operation>-<slug>.md` with the UTC time, and the publish workflow joins the files into the Log page. Agents add files and never edit earlier ones, so parallel pull requests never conflict on the log. Each entry starts with a fixed prefix so that `grep` can parse the log, and `tools/plan/plan.py lint` checks the name and the first line.

```text
## [2026-11-02] ingest | ENG-04 Flow program interpreter
## [2026-11-03] lint | 0 broken links, 1 stale claim fixed
## [2026-11-03] query | How does the agent bind source addresses
## [2026-11-04] decision | 0010 native extensions accepted
```

The operation is one of `ingest`, `query`, `lint`, `decision`, `gate`, or `plan`.

## Operations

Agents perform three operations on the wiki.

### Ingest

An agent ingests every finished work package in the same pull request that finishes it. The agent completes four steps.

1. Update the component page of the package's workstream, such as [Engine](Engine.md) or [Controller](Controller.md). When a shared page also needs a change, list the change in the log entry under the heading `Shared page notes`, and the coordinator applies it after the batch merges. [Swarm Protocol](Swarm-Protocol.md#conflict-free-merges) names the shared pages.
2. Record facts that later agents need, such as a new seam, a measured number, or a known limitation. Link the evidence, such as the handoff file, a benchmark result, or a test.
3. Mark any claim that the package contradicts and correct it in place. The Log entry names the corrected page.
4. Add one `ingest` entry file under `docs/wiki/log/`. A new page goes on the workstream's component page, and the coordinator links it from [Home](Home.md) after the batch merges.

A package that changes nothing a reader needs still appends its Log entry.

### Query

An agent that answers a question about the project starts from Home and reads the linked pages before reading code. When the answer took real effort and will help again, the agent files it as a page or a section and appends a `query` entry to the Log.

### Lint

A lint pass checks the wiki for broken links, orphan pages, pages that contradict a raw source, claims that a later package made stale, and missing pages for components that exist in the code. The script covers the mechanical checks.

```bash
python3 tools/plan/plan.py lint            # broken links, orphan pages, forbidden punctuation
python3 tools/plan/plan.py render --check  # generated catalog pages are current
```

CI runs both commands on every pull request. A planner agent runs a full lint pass, including the manual checks, at least once per phase and before each gate, and appends a `lint` entry.

## Page rules

Pages follow the documentation voice in [Coding Standards](Coding-Standards.md#documentation). Links between wiki pages use the file name with `.md`, such as `[Testing](Testing.md)`, so they work both in the repository and in the published wiki. Links to repository files use full GitHub URLs. Generated pages carry a comment at the top and change only through `tools/plan/plan.py render`.
