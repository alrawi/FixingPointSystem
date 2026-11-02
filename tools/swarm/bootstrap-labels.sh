#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Create the labels of the Swarm Protocol. Existing labels are left unchanged.
# Usage: bootstrap-labels.sh [owner/repo]
set -euo pipefail

repo="${1:-alrawi/FixingPointSystem}"

labels=(
  "status:backlog|cfd3d7|Dependencies open or epic"
  "status:ready|0e8a16|Claimable now"
  "status:in-progress|1d76db|An agent holds an active lease"
  "status:blocked|b60205|Waiting on a named blocker"
  "status:stale|e99695|Lease expired; any agent may take over"
  "status:review|5319e7|Pull request waits for review"
  "status:done|0b3d0b|Merged and closed"
  "type:wp|0052cc|Work package from the catalog"
  "type:bug|d73a4a|Defect"
  "type:followup|c5def5|Work for later triage"
  "critical-path|ff9f1c|Gates the current phase"
  "needs:owner|fbca04|Requires an owner decision or action"
  "needs:split|f9d0c4|Epic; split before claiming"
  "has-handoff|bfdadc|Earlier progress exists on the branch"
  "wip|ededed|Pull request is not ready to merge"
  "flaky|ffffff|Quarantined flaky test"
  "e2e|c2e0c6|Run end to end suites on this pull request"
  "phase:1|f1e05a|Phase 1 core alpha"
  "phase:2|f1e05a|Phase 2 library"
  "phase:3|f1e05a|Phase 3 scale"
  "size:S|e6e6e6|Up to two hours"
  "size:M|d4d4d4|Up to five hours"
  "size:L|bdbdbd|Epic"
)
for ws in con ops qa eng trn hst agt ctl cli mod rlm cap img ui; do
  labels+=("ws:${ws}|006b75|Workstream ${ws^^}")
done

existing="$(gh api --paginate "repos/${repo}/labels?per_page=100" --jq '.[].name')"
for entry in "${labels[@]}"; do
  IFS='|' read -r name color description <<<"${entry}"
  if grep -qxF -- "${name}" <<<"${existing}"; then
    echo "exists: ${name}"
    continue
  fi
  gh api "repos/${repo}/labels" -f name="${name}" -f color="${color}" \
    -f description="${description}" --silent
  echo "created: ${name}"
done
