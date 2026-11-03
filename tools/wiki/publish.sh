#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Publish docs/wiki to the GitHub wiki repository.
# Usage: publish.sh WIKI_REMOTE_URL [--dry-run]
# Links of the form (Page.md) and (Page.md#anchor) become (Page) and (Page#anchor),
# which is the link form that GitHub wikis resolve.
set -euo pipefail

work=""
cleanup() { if [[ -n "${work}" ]]; then rm -rf "${work}"; fi; }
trap cleanup EXIT

usage() { echo "usage: $0 WIKI_REMOTE_URL [--dry-run]" >&2; exit 2; }

main() {
  [[ $# -ge 1 ]] || usage
  local remote="$1" dry_run="${2:-}"
  local src
  src="$(cd "$(dirname "$0")/../../docs/wiki" && pwd)"
  work="$(mktemp -d)"

  git clone --quiet --depth 1 "${remote}" "${work}/wiki"
  find "${work}/wiki" -maxdepth 1 -name '*.md' -delete
  # Log.md in the repository is a stub; the real page is built from docs/wiki/log/.
  python3 "$(dirname "$0")/build_log.py" --out "${work}/Log.md"
  for page in "${src}"/*.md; do
    local from="${page}"
    if [[ "$(basename "${page}")" == "Log.md" ]]; then from="${work}/Log.md"; fi
    sed -E 's/\]\(([A-Za-z0-9_-]+)\.md(#[^)]*)?\)/](\1\2)/g' "${from}" \
      >"${work}/wiki/$(basename "${page}")"
  done

  cd "${work}/wiki"
  git add -A
  if git diff --cached --quiet; then
    echo "wiki is up to date"
    return 0
  fi
  git -c user.name="fps-wiki-bot" -c user.email="fps-wiki-bot@users.noreply.github.com" \
    commit --quiet -m "Publish wiki from ${GITHUB_SHA:-$(git -C "${src}" rev-parse --short HEAD)}"
  if [[ "${dry_run}" == "--dry-run" ]]; then
    git show --stat HEAD
    return 0
  fi
  git push --quiet origin HEAD
  echo "wiki published"
}

main "$@"
