import { useState } from 'react';
import { startFund, useMarkets, type Fund, type Me } from './api.ts';
import { managerColour } from './parts.tsx';

const TEMPERAMENTS = [
  { key: 'steady', name: 'Steady', line: 'Stocks, gold and bitcoin held in balance, with a trend filter that steps aside in a fall.', mix: [['All Weather', 60], ['Trend Compounder', 30], ['Your calls', 10]] },
  { key: 'balanced', name: 'Balanced', line: 'A steady core plus the largest tech names and one fast AI manager.', mix: [['All Weather', 35], ['Magnificent Seven', 25], ['Open Drive', 30], ['Your calls', 10]] },
  { key: 'bold', name: 'Bold', line: 'Mostly fast managers that trade stocks and oil every day.', mix: [['Open Drive', 45], ['Oil Breakout', 20], ['Magnificent Seven', 25], ['Your calls', 10]] },
] as const;

const IDS: Record<string, string> = { 'All Weather': 'all-weather', 'Magnificent Seven': 'magnificent', 'Open Drive': 'open-drive', 'Trend Compounder': 'trend-compounder', 'Oil Breakout': 'oil-breakout' };
const colourOf = (name: string) => (name === 'Your calls' ? '#3a3a3a' : managerColour(IDS[name] ?? ''));

export function MixBar({ mix }: { mix: readonly (readonly [string, number])[] }) {
  return (
    <span className="mixbar" aria-hidden="true">
      {mix.map(([name, share]) => (
        <i key={name} style={{ width: `${share}%`, background: colourOf(name) }} />
      ))}
    </span>
  );
}

/** The venue takes no order worth less than this many dollars. */
const MIN_ORDER = 10;

/** What an amount buys right now under a mix: the orders big enough to place, and the rest. */
function plan(amount: number, mix: readonly (readonly [string, number])[], funds: Fund[]) {
  const byName = new Map(funds.map((f) => [f.name, f]));
  const want = new Map<string, number>();
  const waiting: string[] = [];
  for (const [name, share] of mix) {
    const fund = byName.get(name);
    if (!fund) continue;
    if (fund.holdings.length === 0) waiting.push(name);
    for (const h of fund.holdings) want.set(h.market, (want.get(h.market) ?? 0) + (amount * share * h.weight) / 100);
  }
  const rows = [...want].sort((a, b) => Math.abs(b[1]) - Math.abs(a[1]));
  return { placed: rows.filter(([, v]) => Math.abs(v) >= MIN_ORDER), small: rows.filter(([, v]) => Math.abs(v) < MIN_ORDER), waiting };
}

function Plan({ amount, mix }: { amount: number; mix: readonly (readonly [string, number])[] }) {
  const [data] = useMarkets();
  if (!data) return null;
  const { placed, small, waiting } = plan(amount, mix, data.funds);
  const money = (v: number) => `$${Math.abs(v).toFixed(v >= 100 ? 0 : 2)}`;
  return (
    <div className="plan">
      <span className="plan-title">What ${amount.toLocaleString()} buys right now</span>
      {placed.length > 0 && <span>{placed.map(([m, v]) => `${m} ${money(v)}`).join(' · ')}</span>}
      {small.length > 0 && (
        <span className="muted">
          Under the $10 minimum with real money: {small.map(([m, v]) => `${m} ${money(v)}`).join(', ')}
        </span>
      )}
      {waiting.length > 0 && <span className="muted">Waiting for its signal: {waiting.join(', ')}</span>}
    </div>
  );
}

export function Start({ onStarted }: { onStarted: (me: Me) => void }) {
  const [name, setName] = useState('My Fund');
  const [amount, setAmount] = useState(100);
  const [temperament, setTemperament] = useState<'steady' | 'balanced' | 'bold'>('balanced');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const start = () => {
    setBusy(true);
    setError(null);
    startFund({ name, amount, temperament })
      .then(onStarted)
      .catch((e: Error) => setError(e.message))
      .finally(() => setBusy(false));
  };
  return (
    <div className="start">
      <span className="wordmark">compounder</span>
      <h1 className="start-title">Start your fund</h1>
      <label className="start-name">
        <input value={name} onChange={(e) => setName(e.target.value)} aria-label="Fund name" maxLength={40} />
      </label>
      <div className="start-amount">
        $<input inputMode="decimal" value={amount} onChange={(e) => setAmount(Number(e.target.value.replace(/[^0-9.]/g, '')) || 0)} aria-label="Amount" />
      </div>
      <div className="start-presets">
        {[100, 1000, 10000].map((a) => (
          <button key={a} className={`chip${a === amount ? ' chip-on' : ''}`} onClick={() => setAmount(a)}>
            ${a.toLocaleString()}
          </button>
        ))}
      </div>
      <div className="temperaments" role="radiogroup" aria-label="Temperament">
        {TEMPERAMENTS.map((t) => (
          <button key={t.key} role="radio" aria-checked={t.key === temperament} className={`temperament${t.key === temperament ? ' temperament-on' : ''}`} onClick={() => setTemperament(t.key)}>
            <span className="temperament-name">{t.name}</span>
            <MixBar mix={t.mix} />
            <span className="temperament-line">{t.line}</span>
            <span className="temperament-mix">{t.mix.map(([n, s]) => `${n} ${s}%`).join(' · ')}</span>
            {t.key === temperament && <Plan amount={amount} mix={t.mix} />}
          </button>
        ))}
      </div>
      <div className="mode mode-wide">
        <span className="mode-on">Practice</span>
        <span>Real USDC · soon</span>
      </div>
      {error && <p className="error">{error}</p>}
      <div className="dock">
        <button className="cta cta-up" disabled={busy || amount < 10} onClick={start}>
          {busy ? 'Starting…' : 'Start my fund'}
        </button>
      </div>
    </div>
  );
}
