import { useState } from 'react';
import { startFund, useMarkets, useOnchain, type Me } from './api.ts';
import { managerColour } from './parts.tsx';

/** One amount, one tap: a fund that copies the onchain fund's strategies. */
export function Start({ onStarted }: { onStarted: (me: Me) => void }) {
  const [amount, setAmount] = useState(1000);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const onchain = useOnchain();
  const [data] = useMarkets();
  const names = new Map((data?.funds ?? []).map((f) => [f.id, f.name]));
  const mix = onchain?.mix ?? [];
  const start = () => {
    setBusy(true);
    setError(null);
    startFund(amount)
      .then(onStarted)
      .catch((e: Error) => setError(e.message))
      .finally(() => setBusy(false));
  };
  return (
    <div className="start">
      <span className="wordmark">compounder</span>
      <h1 className="start-title">Copy the fund</h1>
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
      {mix.length > 0 && (
        <>
          <span className="mixbar mixbar-big" aria-hidden="true">
            {mix.map((m) => (
              <i key={m.id} style={{ width: `${m.share * 100}%`, background: managerColour(m.id) }} />
            ))}
            <i style={{ flex: 1, background: '#3a3a3a' }} />
          </span>
          <span className="legend">
            {mix.map((m) => (
              <span key={m.id}>
                <i style={{ background: managerColour(m.id) }} />
                {names.get(m.id) ?? m.id} ${Math.round(amount * m.share).toLocaleString()}
              </span>
            ))}
            <span>
              <i style={{ background: '#3a3a3a' }} />
              Cash ${Math.round(amount * (1 - mix.reduce((s, m) => s + m.share, 0))).toLocaleString()}
            </span>
          </span>
        </>
      )}
      <p className="about">Practice money for now. Same strategies, same trades.</p>
      {error && <p className="error">{error}</p>}
      <div className="dock dock-tabs">
        <button className="cta cta-up" disabled={busy || amount < 10} onClick={start}>
          {busy ? 'Starting…' : `Copy with $${amount.toLocaleString()}`}
        </button>
      </div>
    </div>
  );
}
