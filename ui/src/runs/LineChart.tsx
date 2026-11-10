// SPDX-License-Identifier: GPL-3.0-only
//
// A small line chart in SVG. The charts of the run view have at most a few
// series of a few hundred points each, so plain SVG is enough, adds no
// dependency, and stays testable in jsdom, which has no canvas. The chart
// carries its data in three ways: the polylines, a text legend with the
// latest value of each series, and a table behind a disclosure that lists the
// newest samples, so the picture is never the only way to read a value.

import { useId } from "react";
import { formatClock } from "./format";
import type { Point } from "./metricsStore";

/** One line of a chart. */
export interface ChartSeries {
  id: string;
  label: string;
  points: Point[];
}

export interface LineChartProps {
  title: string;
  series: ChartSeries[];
  /** Formats a value for the axis, the legend, and the table. */
  format: (value: number) => string;
  /** How many of the newest points the chart draws. */
  window?: number;
  /** How many rows the table behind the disclosure shows. */
  tableRows?: number;
}

const WIDTH = 640;
const HEIGHT = 200;
const PAD = { left: 76, right: 12, top: 10, bottom: 26 };

/** Strokes differ in color and in dash pattern, so no series depends on color alone. */
const STROKES = [
  { color: "var(--chart-1)", dash: undefined },
  { color: "var(--chart-2)", dash: "6 3" },
  { color: "var(--chart-3)", dash: "2 3" },
] as const;

/** The geometry of one series, in SVG units. */
export interface Plotted {
  id: string;
  /** The `points` attribute of the polyline. */
  path: string;
  last: { x: number; y: number } | undefined;
}

/** Maps the series into the plot area. The y axis starts at zero and ends at a round number. */
export function plot(series: ChartSeries[], window: number) {
  const visible = series.map((line) => ({ ...line, points: line.points.slice(-window) }));
  const all = visible.flatMap((line) => line.points);
  const tMin = Math.min(...all.map((p) => p.t));
  const tMax = Math.max(...all.map((p) => p.t));
  const vMax = niceCeiling(Math.max(0, ...all.map((p) => p.v)));
  const plotW = WIDTH - PAD.left - PAD.right;
  const plotH = HEIGHT - PAD.top - PAD.bottom;
  const x = (t: number) =>
    PAD.left + (tMax === tMin ? plotW : ((t - tMin) / (tMax - tMin)) * plotW);
  const y = (v: number) => PAD.top + plotH - (vMax === 0 ? 0 : (v / vMax) * plotH);
  const lines: Plotted[] = visible.map((line) => {
    const coords = line.points.map((p) => ({ x: round(x(p.t)), y: round(y(p.v)) }));
    return {
      id: line.id,
      path: coords.map((c) => `${String(c.x)},${String(c.y)}`).join(" "),
      last: coords[coords.length - 1],
    };
  });
  return { lines, tMin, tMax, vMax, visible };
}

function round(value: number): number {
  return Math.round(value * 10) / 10;
}

/** The smallest of 1, 2, 2.5, 5, and 10 times a power of ten that is at least the value. */
function niceCeiling(value: number): number {
  if (value <= 0) return 0;
  const power = Math.pow(10, Math.floor(Math.log10(value)));
  for (const step of [1, 2, 2.5, 5, 10]) if (step * power >= value) return step * power;
  return 10 * power;
}

/** A line chart of time series with a legend and a table alternative. */
export function LineChart({ title, series, format, window = 300, tableRows = 10 }: LineChartProps) {
  const id = useId();
  const drawn = series.filter((line) => line.points.length > 0);
  if (drawn.length === 0) {
    return (
      <figure className="chart">
        <figcaption>{title}</figcaption>
        <p className="hint">Waiting for the first sample.</p>
      </figure>
    );
  }
  const { lines, tMin, tMax, vMax, visible } = plot(drawn, window);
  const latest = visible
    .map((line) => `${line.label} ${format(line.points[line.points.length - 1]?.v ?? 0)}`)
    .join(", ");
  const count = visible[0]?.points.length ?? 0;
  const times = [...new Set(visible.flatMap((line) => line.points.map((p) => p.t)))]
    .sort((a, b) => b - a)
    .slice(0, tableRows);
  return (
    <figure className="chart">
      <figcaption id={`${id}-title`}>{title}</figcaption>
      <svg
        viewBox={`0 0 ${String(WIDTH)} ${String(HEIGHT)}`}
        role="img"
        aria-labelledby={`${id}-title ${id}-desc`}
        className="chart-svg"
      >
        <desc id={`${id}-desc`}>
          {`Line chart of the last ${String(count)} seconds. Latest: ${latest}.`}
        </desc>
        {[0, 0.5, 1].map((share) => {
          const y = PAD.top + (HEIGHT - PAD.top - PAD.bottom) * (1 - share);
          return (
            <g key={share}>
              <line x1={PAD.left} x2={WIDTH - PAD.right} y1={y} y2={y} className="chart-grid" />
              <text x={PAD.left - 6} y={y + 4} textAnchor="end" className="chart-label">
                {format(vMax * share)}
              </text>
            </g>
          );
        })}
        <text x={PAD.left} y={HEIGHT - 6} textAnchor="start" className="chart-label">
          {formatClock(tMin)}
        </text>
        <text x={WIDTH - PAD.right} y={HEIGHT - 6} textAnchor="end" className="chart-label">
          {formatClock(tMax)}
        </text>
        {lines.map((line, index) => {
          const stroke = STROKES[index % STROKES.length] ?? STROKES[0];
          const values = visible[index]?.points.map((p) => p.v) ?? [];
          return (
            <g key={line.id}>
              <polyline
                fill="none"
                stroke={stroke.color}
                strokeWidth={2}
                {...(stroke.dash === undefined ? {} : { strokeDasharray: stroke.dash })}
                points={line.path}
                data-series={line.id}
                data-values={values.join(",")}
              />
              {line.last && <circle cx={line.last.x} cy={line.last.y} r={3} fill={stroke.color} />}
            </g>
          );
        })}
      </svg>
      <ul className="chart-legend">
        {visible.map((line, index) => {
          const stroke = STROKES[index % STROKES.length] ?? STROKES[0];
          return (
            <li key={line.id}>
              <svg width="28" height="10" aria-hidden="true">
                <line
                  x1="0"
                  x2="28"
                  y1="5"
                  y2="5"
                  stroke={stroke.color}
                  strokeWidth={2}
                  {...(stroke.dash === undefined ? {} : { strokeDasharray: stroke.dash })}
                />
              </svg>{" "}
              <span>{line.label}</span>{" "}
              <strong data-legend={line.id}>
                {format(line.points[line.points.length - 1]?.v ?? 0)}
              </strong>
            </li>
          );
        })}
      </ul>
      <details>
        <summary>{`Values of ${title}`}</summary>
        <table aria-label={`Newest values of ${title}`}>
          <thead>
            <tr>
              <th scope="col">Time (UTC)</th>
              {visible.map((line) => (
                <th key={line.id} scope="col">
                  {line.label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {times.map((t) => (
              <tr key={t}>
                <th scope="row">{formatClock(t)}</th>
                {visible.map((line) => {
                  const point = line.points.find((p) => p.t === t);
                  return <td key={line.id}>{point === undefined ? "-" : format(point.v)}</td>;
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </figure>
  );
}
