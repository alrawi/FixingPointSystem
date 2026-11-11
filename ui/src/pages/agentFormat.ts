// SPDX-License-Identifier: GPL-3.0-only
//
// Text formats of the agent view.

/** A link speed that the API reports in Mbit/s, as Mbit/s below 1000 and as Gbit/s from there. */
export function formatLinkSpeed(speedMbps: number | undefined): string {
  if (speedMbps === undefined) return "not reported";
  if (speedMbps < 1000) return `${String(speedMbps)} Mbit/s`;
  const gbps = new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 }).format(
    speedMbps / 1000,
  );
  return `${gbps} Gbit/s`;
}

/** The labels of an agent as `key=value` pairs in key order, or an empty text without labels. */
export function formatLabels(labels: Record<string, string> | undefined): string {
  return Object.entries(labels ?? {})
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([key, value]) => `${key}=${value}`)
    .join(", ");
}

/** The holder that reserveAgent records for a reservation of the operator. */
export const OPERATOR_HOLDER = "operator";

/**
 * Who holds the reservation of an agent: `none` without a reservation, `operator` for a
 * reservation through reserveAgent, `run` for an agent that a run uses (with the run id), and
 * `unknown` for a `reserved` agent from a controller older than 0.10.0, which sends no holder.
 */
export type Holder =
  { kind: "none" } | { kind: "operator" } | { kind: "run"; id: string } | { kind: "unknown" };

/**
 * Reads the holder of an agent from `reserved_by` and `state`. The field counts whatever the
 * state is, because `offline` and `busy` take precedence over `reserved` in the state while the
 * reservation still exists.
 */
export function holderOf(agent: { state: string; reserved_by?: string | undefined }): Holder {
  const by = agent.reserved_by;
  if (by === undefined || by === "") {
    return agent.state === "reserved" ? { kind: "unknown" } : { kind: "none" };
  }
  return by === OPERATOR_HOLDER ? { kind: "operator" } : { kind: "run", id: by };
}
