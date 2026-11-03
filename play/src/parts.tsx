import type { ReactNode } from 'react';
import { back, go } from './route.ts';
import { pct } from './format.ts';

/** A manager's own colour, the same everywhere it appears. */
export const MANAGER_COLOURS: Record<string, string> = {
  'all-weather': '#00ec4b',
  magnificent: '#8cc8ff',
  'open-drive': '#ffb01f',
  'trend-compounder': '#c9b5ff',
  'oil-breakout': '#ff8f5a',
  'chip-rotation': '#5ee7df',
  'daily-reversal': '#ff8fab',
  'agent-deepseek': '#9db4ff',
};
export const managerColour = (id: string) => MANAGER_COLOURS[id] ?? '#bfbfbf';

export function ManagerMark({ id, name }: { id: string; name: string }) {
  const colour = managerColour(id);
  return (
    <span className="logo" style={{ background: `${colour}22`, color: colour, boxShadow: `inset 0 0 0 1px ${colour}55` }} aria-hidden="true">
      {name
        .split(' ')
        .map((w) => w[0])
        .join('')
        .slice(0, 2)
        .toUpperCase()}
    </span>
  );
}

export function Logo({ symbol, kind }: { symbol: string; kind?: string }) {
  return (
    <span className={`logo logo-${kind ?? 'fund'}`} aria-hidden="true">
      {symbol.replace(/[^A-Z0-9]/gi, '').slice(0, 2).toUpperCase()}
    </span>
  );
}

export function Change({ value }: { value: number | null | undefined }) {
  const tone = toneOf(value);
  return <span className={`change change-${tone}`}>{pct(value)}</span>;
}

export function TopBar({ title, right, backTo }: { title?: ReactNode; right?: ReactNode; backTo?: string }) {
  return (
    <header className="topbar">
      {backTo !== undefined ? (
        <button className="icon-button" onClick={() => back(backTo)} aria-label="Back">
          <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M15 18l-6-6 6-6" />
          </svg>
        </button>
      ) : (
        <span className="wordmark">Compounder</span>
      )}
      {title !== undefined && <span className="topbar-title">{title}</span>}
      <span className="topbar-right">{right}</span>
    </header>
  );
}

export function Link({ to, className, children }: { to: string; className?: string; children: ReactNode }) {
  return (
    <a
      href={to}
      className={className}
      onClick={(event) => {
        if (event.metaKey || event.ctrlKey) return;
        event.preventDefault();
        go(to);
      }}
    >
      {children}
    </a>
  );
}

export function Spark({ points, tone }: { points: [number, number][]; tone: 'up' | 'down' | 'flat' }) {
  if (points.length < 2) return <svg className={`spark spark-${tone}`} viewBox="0 0 100 32" />;
  const ts = points.map((p) => p[0]);
  const vs = points.map((p) => p[1]);
  const [t0, t1] = [Math.min(...ts), Math.max(...ts)];
  let [v0, v1] = [Math.min(...vs), Math.max(...vs)];
  if (v1 - v0 < 1e-9) [v0, v1] = [v0 - 1, v1 + 1];
  const d = points.map(([t, v], i) => `${i ? 'L' : 'M'}${(((t - t0) / (t1 - t0 || 1)) * 100).toFixed(2)} ${(30 - ((v - v0) / (v1 - v0)) * 28).toFixed(2)}`).join('');
  return (
    <svg className={`spark spark-${tone}`} viewBox="0 0 100 32" preserveAspectRatio="none" aria-hidden="true">
      <path d={d} fill="none" stroke="currentColor" strokeWidth="2" vectorEffect="non-scaling-stroke" strokeLinejoin="round" />
    </svg>
  );
}

export const toneOf = (n: number | null | undefined) => (n == null || Math.abs(n) < 0.00005 ? 'flat' : n > 0 ? 'up' : 'down');
