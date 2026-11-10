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
