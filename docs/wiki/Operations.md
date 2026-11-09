# Operations

This page describes the repository automation that the OPS workstream runs on GitHub, beyond the build and test workflows. It grows with each OPS package. The [Swarm Protocol](Swarm-Protocol.md) describes the scanner, and [Project Board Setup](Project-Board-Setup.md) describes the board itself.

## Project board status sync

The workflow `.github/workflows/project-sync.yml` copies the status label of each work item into the Status field of the owner's user project, so the Board views that group by Status follow the labels. It calls `tools/swarm/project_sync.py`, which needs only the Python standard library and talks to the GraphQL API.

| Trigger | What the job syncs |
|---|---|
| An issue is opened, reopened, labeled, or unlabeled | The one issue of the event, read fresh from the API |
| The schedule at minutes 7 and 37 of every hour | Every open and closed issue, which covers the relabels of the [scanner](Swarm-Protocol.md#the-scanner) a few minutes after each scanner run |
| A manual run of **Run workflow** | Every issue, and with `dry_run` set, which is the default, the job prints the planned changes and changes nothing |

The scanner relabels with the repository token, and GitHub starts no workflow for events from that token. The schedule therefore moves a card that the scanner relabeled within about 7 minutes, while a label change made by a person or an agent with its own token moves the card within one minute.

For each issue with exactly one status label, the script looks up the Status option with the same name and changes the item only when its current option differs. It adds an issue that is not yet on the board, so the sync works even before **Auto-add to project** catches the issue. The script maps the labels as follows.

| Label | Status option |
|---|---|
| `status:backlog` | Backlog |
| `status:ready` | Ready |
| `status:in-progress` | In progress |
| `status:blocked` | Blocked |
| `status:stale` | Stale |
| `status:review` | Review |
| `status:done` | Done |

The script exits with 0 when the sync succeeds and also when the secret `FPS_PROJECT_TOKEN` is absent, in which case it prints a notice and makes no request. It exits with 1 when an issue carries more than one status label or the board lacks the matching option, after it has synced every other issue. It exits with 2 when it finds no project, more than one candidate project, no Status field, or a token without the needed access.

### Owner setup

The owner completes four steps once, after the board exists as [Project Board Setup](Project-Board-Setup.md) describes.

1. Create a fine grained personal access token at https://github.com/settings/personal-access-tokens/new with the resource owner `alrawi`. Under **Repository access**, select only `alrawi/FixingPointSystem` and grant **Issues** read only access. Under **Account permissions**, grant **Projects** read and write access. A classic token with the scopes `project` and `repo` also works.
2. Store the token as the repository secret `FPS_PROJECT_TOKEN` under **Settings**, **Secrets and variables**, **Actions**.
3. Optionally store the project number as the repository variable `FPS_PROJECT_NUMBER` on the **Variables** tab of the same page. The number is the last part of the project URL, for example `1` in `https://github.com/users/alrawi/projects/1`. Without the variable the script uses the one project of `alrawi` that is linked to the repository, and it stops with an error when it finds more than one.
4. Open **Actions**, select **project-sync**, and select **Run workflow** with `dry_run` set. The log names the project and lists each planned change. Run it again with `dry_run` cleared to apply the changes, then relabel a test issue and check that its card moves within one minute.

### Running it by hand

An agent or the owner can run the same command locally with a token, for example to check one issue.

```bash
export FPS_PROJECT_TOKEN=...            # never commit or paste the token
python3 tools/swarm/project_sync.py --issue 13 --dry-run
python3 tools/swarm/project_sync.py     # sync every issue
```

Agents in sandboxed sessions cannot reach the GraphQL API, so they never run it. The tests in `tools/swarm/test_project_sync.py` replay GraphQL responses from `tools/swarm/testdata/project_sync/` and never reach the network.

## Benchmark runner

The nightly benchmark of QA-08 runs on a self hosted runner on the owner's Linux server with the label `fps-bench`. The owner registers it with the steps in [`bench/README.md`, section Registering the owner host](https://github.com/alrawi/FixingPointSystem/blob/main/bench/README.md#registering-the-owner-host), and [Benchmarks](Benchmarks.md) explains the comparison.

## Release workflow

The workflow `.github/workflows/release.yml` builds the versioned artifacts of a release, writes `SHA256SUMS` and the release notes, and creates the GitHub release. Only the coordinator edits this file, as with every workflow.

| Trigger | Result |
|---|---|
| A push of a tag `v*` | A real release. The tag must look like `v1.2.3` or `v1.2.3-rc.1`, and a tag with a suffix becomes a pre-release |
| A manual run of **Run workflow** with `dry_run` set, which is the default | The same build on the chosen branch or tag. It keeps everything as the workflow artifact `release-<version>` and creates no release. A branch run is versioned `0.0.0-dryrun.<short sha>` |
| A manual run on a tag with `dry_run` cleared | A real release, as for a pushed tag |

### What a release holds

| File | Built as |
|---|---|
| `fps-agent-<version>-linux-x86_64` | `cargo build --release --locked --target x86_64-unknown-linux-musl -p fps-agent`, stripped, and checked with `file` to be static |
| `fps-controller-<version>-linux-amd64` | `go build -trimpath -ldflags '-s -w'` with `CGO_ENABLED=0`, checked to be static, and started with `-version` |
| `fps-module-<name>-<module version>.tar.gz` | One archive for each module under `modules/` that has a `manifest.yaml`, with the tree under `<name>/` and without build output. The module version comes from the manifest. The archive is reproducible for one commit |
| `fixingpoint-<version>.tar.gz` and `fixingpoint-<version>-py3-none-any.whl` | `python -m build` in `sdk/python/`. The version is the one in `pyproject.toml`, and the job warns when it differs from the tag |
| `SHA256SUMS` | `sha256sum` of every file above, checked with `sha256sum -c` before upload |

The release notes (`RELEASE_NOTES.md`) hold the sections of `contracts/CHANGELOG.md` that are newer than the previous `v*` tag (all of them for the first release, and the Unreleased section always), followed by the titles of the pull requests merged on the first parent history since that tag. The workflow runs no module signing. CTL-11 signs module archives in the registry when it publishes an index.

The controller prints the module version that the Go toolchain stamps into the binary, and `controller/` has no variable that `-ldflags -X` could set. The version of a controller binary is therefore its file name and its line in `SHA256SUMS`. A later CTL package can add such a variable.

### Rules that keep the release safe

- Every job runs on `ubuntu-24.04` of GitHub. The repository is public, so no release job may use the self hosted `fps-bench` runner.
- The workflow has no `pull_request` trigger. It holds `contents: read` everywhere except the `publish` job, which holds `contents: write`, runs no repository code, and only downloads the checked bundle and calls `gh release create`.
- Tag names and pull request titles reach scripts through `env`, never inside `run:` text. The notes script shortens each title to one line and defuses `@` mentions.
- It uses only `actions/*` and `dtolnay/rust-toolchain`, pinned like the other workflows. A third party action must be pinned by commit.

### Cutting a release

1. Check that `contracts/CHANGELOG.md` describes the contract changes, and bump `sdk/python/pyproject.toml` when the Python package should carry the release number.
2. Run **Run workflow** for `release` on `main` with `dry_run` set, and download the artifact `release-<version>` to inspect it.
3. Create and push an annotated tag, for example `git tag -a v0.1.0-rc.1 -m "Release candidate 1"` and `git push origin v0.1.0-rc.1`. The workflow creates the release.

### Attaching the image by hand

The workflow does not build the VM image, because the Packer build needs KVM on the owner host. After the release exists, the owner builds the image on that host with `image/build.sh` from the tagged commit (see [`image/README.md`](https://github.com/alrawi/FixingPointSystem/blob/main/image/README.md)) and attaches it with its checksum.

```bash
git checkout v0.1.0
image/build.sh                       # writes image/output/fps-base.qcow2 and fps-base.qcow2.sha256
cd image/output
mv fps-base.qcow2 fps-image-0.1.0-x86_64.qcow2
sha256sum fps-image-0.1.0-x86_64.qcow2 > fps-image-0.1.0-x86_64.qcow2.sha256
gh release upload v0.1.0 fps-image-0.1.0-x86_64.qcow2 fps-image-0.1.0-x86_64.qcow2.sha256
```

`SHA256SUMS` of the release stays the list of the workflow files. The `.sha256` file next to the image is the checksum of the image, and the owner may also append its line to `SHA256SUMS` and replace the release asset with `gh release upload --clobber`.
