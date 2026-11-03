/** A line of values over time, with no axes: the shape of a run at a glance. */
export function Sparkline({ points, tone = 'plain', large = false, label }: { points: [number, number][]; tone?: 'gain' | 'loss' | 'plain'; large?: boolean; label?: string }) {
  const width = 100;
  const height = large ? 40 : 30;
  if (points.length < 2) return <span className={`sparkline sparkline-${tone}${large ? ' sparkline-large' : ''}`} aria-hidden="true" />;
  const ts = points.map((p) => p[0]);
  const vs = points.map((p) => p[1]);
  const [t0, t1] = [Math.min(...ts), Math.max(...ts)];
  let [v0, v1] = [Math.min(...vs), Math.max(...vs)];
  if (v1 - v0 < 1e-9) [v0, v1] = [v0 - 1, v1 + 1];
  const x = (t: number) => ((t - t0) / (t1 - t0 || 1)) * width;
  const y = (v: number) => height - 2 - ((v - v0) / (v1 - v0)) * (height - 4);
  const d = points.map(([t, v], i) => `${i ? 'L' : 'M'}${x(t).toFixed(2)} ${y(v).toFixed(2)}`).join('');
  return (
    <svg
      className={`sparkline sparkline-${tone}${large ? ' sparkline-large' : ''}`}
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="none"
      role={label ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      <path d={d} fill="none" stroke="currentColor" strokeWidth="1.5" vectorEffect="non-scaling-stroke" strokeLinejoin="round" />
    </svg>
  );
}
