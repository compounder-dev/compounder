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

const LOGOS = new Set(['AAPL', 'AMD', 'AMZN', 'BTC', 'COIN', 'ETH', 'GOOGL', 'HOOD', 'HYPE', 'META', 'MSFT', 'MSTR', 'MU', 'NVDA', 'PLTR', 'SOL', 'TSLA']);

/** Marks drawn here for what has no company logo: metals, oil and the indices. */
const MARKS: Record<string, { label?: string; colour: string; path?: string }> = {
  GOLD: { colour: '#ffb01f', path: 'M5 16h14l-2.5-7h-9z' },
  SILVER: { colour: '#c8ccd2', path: 'M5 16h14l-2.5-7h-9z' },
  CL: { colour: '#ff8f5a', path: 'M12 4c3 4 5 7 5 9.5a5 5 0 0 1-10 0C7 11 9 8 12 4z' },
  BRENTOIL: { colour: '#ff8f5a', path: 'M12 4c3 4 5 7 5 9.5a5 5 0 0 1-10 0C7 11 9 8 12 4z' },
  SP500: { colour: '#8cc8ff', label: '500' },
  XYZ100: { colour: '#8cc8ff', label: '100' },
};

export function Logo({ symbol, kind }: { symbol: string; kind?: string }) {
  if (LOGOS.has(symbol)) {
    return (
      <span className="logo logo-img" aria-hidden="true">
        <img src={`/logos/${symbol}.webp`} alt="" width="26" height="26" loading="lazy" decoding="async" />
      </span>
    );
  }
  const mark = MARKS[symbol];
  if (mark) {
    return (
      <span className="logo logo-img" style={{ color: mark.colour }} aria-hidden="true">
        {mark.path ? (
          <svg viewBox="0 0 24 24" width="24" height="24" fill="currentColor">
            <path d={mark.path} />
          </svg>
        ) : (
          <b className="logo-label">{mark.label}</b>
        )}
      </span>
    );
  }
  return (
    <span className={`logo logo-${kind ?? 'fund'}`} aria-hidden="true">
      {symbol.replace(/[^A-Z0-9]/gi, '').slice(0, 2).toUpperCase()}
    </span>
  );
}

/** Dollars with the cents dimmer than the whole, as a balance reads at a glance. */
export function Money({ value, className }: { value: number; className?: string }) {
  const [whole, cents] = new Intl.NumberFormat('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 }).format(Math.abs(value)).split('.');
  return (
    <span className={className}>
      {value < 0 ? '−' : ''}${whole}
      <span className="cents">.{cents}</span>
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
