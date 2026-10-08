import { useId } from "react";

export interface TrendSeries {
  label: string;
  /** Any CSS color (a design token value resolved via `color`). */
  color: string;
}

export interface TrendPoint {
  label: string;
  /** One value per series; `null` is a gap (unknown, not zero). */
  values: (number | null)[];
}

/**
 * A compact, dependency-free line chart.
 *
 * * Starts the value axis at zero (bar/line comparisons must not distort).
 * * `null` values break the line rather than plotting a false zero.
 * * The underlying data is also rendered as a visually-hidden table so the
 *   chart is readable without relying on color or vision.
 */
export function TrendChart({
  title,
  series,
  points,
  formatValue,
  valueSuffix = "",
  emptyHint = "Not enough data to chart yet.",
  height = 190,
}: {
  title: string;
  series: TrendSeries[];
  points: TrendPoint[];
  formatValue: (n: number) => string;
  valueSuffix?: string;
  emptyHint?: string;
  height?: number;
}) {
  const titleId = useId();
  const W = 640;
  const H = 220;
  const padL = 52;
  const padR = 14;
  const padT = 14;
  const padB = 30;

  const plotted = points.filter((p) => p.values.some((v) => v !== null));
  const max = Math.max(
    1,
    ...plotted.flatMap((p) => p.values.map((v) => (v === null ? 0 : v))),
  );
  const n = plotted.length;

  const x = (i: number) =>
    n <= 1 ? padL + (W - padL - padR) / 2 : padL + (i * (W - padL - padR)) / (n - 1);
  const y = (v: number) => padT + (1 - v / max) * (H - padT - padB);

  // Four horizontal gridlines from zero to max.
  const ticks = [0, 0.25, 0.5, 0.75, 1].map((f) => ({
    value: max * f,
    y: y(max * f),
  }));

  return (
    <section className="card rounded-xl p-4">
      <header className="mb-2 flex items-center justify-between gap-3">
        <h3 className="text-[13px] font-semibold text-ink">{title}</h3>
        <div className="flex items-center gap-3">
          {series.map((s) => (
            <span
              key={s.label}
              className="flex items-center gap-1.5 text-[11px] text-ink-muted"
            >
              <span
                className="h-2 w-2 rounded-full"
                style={{ background: s.color }}
                aria-hidden="true"
              />
              {s.label}
            </span>
          ))}
        </div>
      </header>

      {n === 0 ? (
        <p className="py-10 text-center text-[12px] text-ink-subtle">{emptyHint}</p>
      ) : (
        <svg
          viewBox={`0 0 ${W} ${H}`}
          className="h-auto w-full"
          style={{ maxHeight: height + 40 }}
          role="img"
          aria-labelledby={titleId}
        >
          <title id={titleId}>{title}</title>
          {ticks.map((t) => (
            <g key={t.value}>
              <line
                x1={padL}
                x2={W - padR}
                y1={t.y}
                y2={t.y}
                stroke="var(--color-line)"
                strokeWidth={1}
              />
              <text
                x={padL - 8}
                y={t.y + 3.5}
                textAnchor="end"
                className="fill-[var(--color-ink-subtle)]"
                fontSize={10}
              >
                {formatValue(t.value)}
                {valueSuffix}
              </text>
            </g>
          ))}

          {series.map((s, si) => {
            // Split runs of defined values into separate segments so a gap is
            // never bridged by a straight line.
            const segments: [number, number][][] = [];
            let current: [number, number][] = [];
            plotted.forEach((p, i) => {
              const v = p.values[si];
              if (v === null || v === undefined) {
                if (current.length) segments.push(current);
                current = [];
              } else {
                current.push([x(i), y(v)]);
              }
            });
            if (current.length) segments.push(current);
            return (
              <g key={s.label}>
                {segments.map((seg, i) => (
                  <polyline
                    key={i}
                    points={seg.map(([px, py]) => `${px},${py}`).join(" ")}
                    fill="none"
                    stroke={s.color}
                    strokeWidth={2}
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                ))}
                {plotted.map((p, i) =>
                  p.values[si] === null || p.values[si] === undefined ? null : (
                    <circle
                      key={i}
                      cx={x(i)}
                      cy={y(p.values[si] as number)}
                      r={2.5}
                      fill={s.color}
                    />
                  ),
                )}
              </g>
            );
          })}

          {plotted.map((p, i) => (
            <text
              key={p.label + i}
              x={x(i)}
              y={H - 8}
              textAnchor="middle"
              className="fill-[var(--color-ink-subtle)]"
              fontSize={10}
            >
              {p.label}
            </text>
          ))}
        </svg>
      )}

      {/* Accessible alternative to the chart. */}
      <table className="sr-only">
        <caption>{title}</caption>
        <thead>
          <tr>
            <th scope="col">Day</th>
            {series.map((s) => (
              <th key={s.label} scope="col">
                {s.label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {plotted.map((p, i) => (
            <tr key={p.label + i}>
              <th scope="row">{p.label}</th>
              {p.values.map((v, si) => (
                <td key={si}>{v === null ? "unavailable" : formatValue(v)}</td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
