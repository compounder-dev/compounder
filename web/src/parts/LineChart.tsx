import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react';
import { axisSteps, dayText, extent, keyMove, linePath, nearest, thinned, timeSteps } from './chart.ts';

export type ChartLine = {
  name: string;
  /** One value per day. Null where the line has none. */
  values: (number | null)[];
  /** The subject takes the fund's colour. A comparison is ink. */
  kind: 'subject' | 'comparison';
};

type LineChartProps = {
  /** What the chart shows, for someone who cannot see it. */
  label: string;
  days: string[];
  lines: ChartLine[];
  /** How a value is written, on the axis and beside the crosshair. */
  format: (value: number) => string;
  /** A value the axis always keeps in view, such as zero. */
  hold?: number;
};

// Room kept clear of the lines, in the chart's own units: above, for the axis words on the right, and for the dates below.
const ROOM = { top: 8, right: 56, bottom: 24 };
// The width a date label needs, so labels never run into each other.
const DATE_WIDTH = 64;

/**
 * Lines over time with a crosshair. The pointer moves the crosshair, and so do the arrow keys:
 * the plot is one control whose value is the day under the crosshair, read out as that day's figures.
 */
export function LineChart({ label, days, lines, format, hold }: LineChartProps) {
  const plot = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState<{ width: number; height: number } | null>(null);
  const [at, setAt] = useState<number | null>(null);

  useEffect(() => {
    const element = plot.current;
    if (!element) return;
    const measure = () => setSize({ width: element.clientWidth, height: element.clientHeight });
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(element);
    return () => watch.disconnect();
  }, []);

  // The crosshair belongs to the days it was set on.
  useEffect(() => setAt(null), [days]);

  const drawn = useMemo(() => {
    if (!size || size.width <= ROOM.right || days.length === 0) return null;
    const inner = { width: size.width - ROOM.right, height: size.height - ROOM.top - ROOM.bottom };
    // Two points for each unit of width are as many as a screen can show.
    const kept = thinned(days.length, Math.max(2, Math.floor(inner.width) * 2));
    const [low, high] = extent(lines.map((line) => line.values), hold);
    const span = high - low || 1;
    const x = (place: number) => (days.length > 1 ? (place / (days.length - 1)) * inner.width : 0);
    const y = (value: number) => ROOM.top + (1 - (value - low) / span) * inner.height;
    const xs = kept.map(x);
    return {
      kept,
      xs,
      inner,
      y,
      paths: lines.map((line) => linePath(xs, kept.map((place) => (line.values[place] === null ? null : y(line.values[place] as number))))),
      steps: axisSteps(low, high),
      dates: timeSteps(days, Math.max(1, Math.floor(inner.width / DATE_WIDTH))).map((step) => ({ ...step, x: x(step.place) })),
    };
  }, [size, days, lines, hold]);

  const last = days.length - 1;
  const shown = at ?? last;
  const figures = lines.map((line) => ({ ...line, value: line.values[shown] ?? null }));
  const said = `${dayText(days[shown] ?? '')}: ${figures.map((line) => `${line.name} ${line.value === null ? 'no value' : format(line.value)}`).join(', ')}`;

  const toPointer = (event: PointerEvent<HTMLDivElement>) => {
    if (!drawn) return;
    const left = event.currentTarget.getBoundingClientRect().left;
    setAt(drawn.kept[nearest(drawn.xs, event.clientX - left)]);
  };
  const onKey = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!drawn) return;
    // The keys step through the points that are drawn, so each press moves the crosshair.
    const from = at === null ? drawn.kept.length - 1 : nearest(drawn.kept, at);
    const to = keyMove(event.key, from, drawn.kept.length);
    if (to === null) return;
    event.preventDefault();
    setAt(drawn.kept[to]);
  };

  return (
    <div className="line-chart">
      <p className="chart-readout" aria-hidden="true">
        <span className="chart-day">{dayText(days[shown] ?? '')}</span>
        {figures.map((line) => (
          <span key={line.name} className="chart-figure">
            <span className={`chart-key chart-key-${line.kind}`} />
            <span className="chart-name">{line.name}</span>
            <span className="number">{line.value === null ? '–' : format(line.value)}</span>
          </span>
        ))}
      </p>
      <div
        ref={plot}
        className="chart-plot"
        role="slider"
        tabIndex={0}
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={Math.max(0, last)}
        aria-valuenow={shown}
        aria-valuetext={said}
        onPointerDown={toPointer}
        onPointerMove={toPointer}
        onPointerLeave={() => setAt(null)}
        onKeyDown={onKey}
      >
        {drawn && size && (
          <svg className="chart-drawing" width={size.width} height={size.height} aria-hidden="true">
            {drawn.steps.map((step) => (
              <g key={step}>
                <line className="chart-grid" x1={0} x2={drawn.inner.width} y1={drawn.y(step)} y2={drawn.y(step)} />
                <text className="chart-label" x={size.width} y={drawn.y(step)} textAnchor="end" dominantBaseline="middle">
                  {format(step)}
                </text>
              </g>
            ))}
            {drawn.dates.map((date) => (
              <text key={date.place} className="chart-label" x={date.x} y={size.height - 4} textAnchor={date.x < DATE_WIDTH / 2 ? 'start' : 'middle'}>
                {date.text}
              </text>
            ))}
            {/* Comparisons first, so the subject is drawn over them. */}
            {[...lines.keys()]
              .sort((a, b) => Number(lines[a].kind === 'subject') - Number(lines[b].kind === 'subject'))
              .map((index) => (
                <path key={lines[index].name} className={`chart-line chart-line-${lines[index].kind}`} d={drawn.paths[index]} />
              ))}
            {at !== null && (
              <g>
                <line className="chart-crosshair" x1={drawn.xs[nearest(drawn.kept, at)]} x2={drawn.xs[nearest(drawn.kept, at)]} y1={ROOM.top} y2={ROOM.top + drawn.inner.height} />
                {figures.map((line) => line.value !== null && <circle key={line.name} className={`chart-dot chart-dot-${line.kind}`} r={4} cx={drawn.xs[nearest(drawn.kept, at)]} cy={drawn.y(line.value)} />)}
              </g>
            )}
          </svg>
        )}
      </div>
    </div>
  );
}
