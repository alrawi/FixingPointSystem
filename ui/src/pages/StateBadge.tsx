// SPDX-License-Identifier: GPL-3.0-only

import type { RunState } from "../api/client";

/** The state of a run as text. The class only adds color, so the word carries the meaning. */
export function StateBadge({ state }: { state: RunState }) {
  return <span className={`state state-${state}`}>{state}</span>;
}
