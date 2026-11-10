// SPDX-License-Identifier: GPL-3.0-only
//
// Number and time formats of the run view. They use fixed locale rules, so the
// same value reads the same in every browser and in the tests.

/** A count with thousands separators, or a dash when the value is unknown. */
export function formatCount(value: number | undefined): string {
  if (value === undefined) return "-";
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 }).format(value);
}

/** A rate or any other value with at most one decimal. */
export function formatDecimal(value: number | undefined): string {
  if (value === undefined) return "-";
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 }).format(value);
}

/** Microseconds as milliseconds, the unit of the latency metrics in the catalog is us. */
export function formatMicros(value: number | undefined): string {
  if (value === undefined) return "-";
  if (value < 1000) return `${formatDecimal(value)} µs`;
  return `${new Intl.NumberFormat("en-US", { maximumFractionDigits: 1, minimumFractionDigits: 1 }).format(value / 1000)} ms`;
}

const BIT_UNITS = ["bit/s", "kbit/s", "Mbit/s", "Gbit/s", "Tbit/s"] as const;

/** Bits per second with a decimal unit prefix. */
export function formatBitsPerSecond(value: number | undefined): string {
  if (value === undefined) return "-";
  let scaled = value;
  let unit = 0;
  while (scaled >= 1000 && unit < BIT_UNITS.length - 1) {
    scaled /= 1000;
    unit += 1;
  }
  return `${new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 }).format(scaled)} ${BIT_UNITS[unit] ?? "bit/s"}`;
}

const BYTE_UNITS = ["B", "kB", "MB", "GB", "TB"] as const;

/** Bytes with a decimal unit prefix. */
export function formatBytes(value: number | undefined): string {
  if (value === undefined) return "-";
  let scaled = value;
  let unit = 0;
  while (scaled >= 1000 && unit < BYTE_UNITS.length - 1) {
    scaled /= 1000;
    unit += 1;
  }
  return `${new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 }).format(scaled)} ${BYTE_UNITS[unit] ?? "B"}`;
}

/** A ratio as a percentage. */
export function formatPercent(value: number | undefined): string {
  if (value === undefined) return "-";
  return `${new Intl.NumberFormat("en-US", { maximumFractionDigits: 2 }).format(value * 100)} %`;
}

/** Milliseconds since the epoch as the UTC time of day, `10:00:05`. */
export function formatClock(ms: number): string {
  return new Date(ms).toISOString().slice(11, 19);
}

/** An ISO timestamp as `2026-11-01 10:00:05` in UTC, or the text itself when it does not parse. */
export function formatTimestamp(iso: string | undefined): string {
  if (iso === undefined) return "";
  const time = new Date(iso);
  return Number.isNaN(time.getTime()) ? iso : time.toISOString().replace("T", " ").slice(0, 19);
}
