import { useEffect, useState, type ReactNode } from 'react';
import { backFund, closeTrade, openTrade, type Me, type Trade } from './api.ts';
import { price, signedUsd, usd } from './format.ts';

export function Sheet({ onClose, children }: { onClose: () => void; children: ReactNode }) {
  useEffect(() => {
    const key = (e: KeyboardEvent) => e.key === 'Escape' && onClose();
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [onClose]);
  return (
    <>
      <div className="backdrop" onClick={onClose} />
      <div className="sheet" role="dialog" aria-modal="true">
        {children}
      </div>
    </>
  );
}

export function Modal({ onClose, children }: { onClose: () => void; children: ReactNode }) {
  return (
    <div className="modal" onClick={onClose} role="dialog" aria-modal="true">
      <div className="modal-card" onClick={(e) => e.stopPropagation()}>
        {children}
      </div>
    </div>
  );
}

const fill = (value: number, min: number, max: number) => ({ '--fill': `${max > min ? ((value - min) / (max - min)) * 100 : 0}%` }) as React.CSSProperties;

/** Pick a side, an amount and a multiplier, and place it. */
export function TradeSheet({ market, symbol, available, onClose, onPlaced }: { market: string; symbol: string; available: number; onClose: () => void; onPlaced: (me: Me) => void }) {
  const [side, setSide] = useState<'up' | 'down' | null>(null);
  const [amount, setAmount] = useState(Math.max(0, Math.floor(available / 2)));
  const multiplier = 1;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const place = () => {
    if (!side) return;
    setBusy(true);
    setError(null);
    openTrade({ market, side, stake: amount, multiplier })
      .then((me) => {
        navigator.vibrate?.(12);
        onPlaced(me);
      })
      .catch((e: Error) => setError(e.message))
      .finally(() => setBusy(false));
  };
  return (
    <Sheet onClose={onClose}>
      <div className="sheet-head">
        <h3>Your call on {symbol}</h3>
        <span className="num">{usd(available)} available</span>
      </div>
      <div className="sides">
        <button className={`go go-up${side === 'up' ? ' go-on' : side ? ' go-dim' : ''}`} onClick={() => setSide('up')}>
          ▲ Up
        </button>
        <button className={`go go-down${side === 'down' ? ' go-on' : side ? ' go-dim' : ''}`} onClick={() => setSide('down')}>
          ▼ Down
        </button>
      </div>
      <div className="field">
        <div className="field-head">
          <label htmlFor="amount">Amount</label>
          <span className="field-value">
            $<input id="amount" inputMode="decimal" value={amount} onChange={(e) => setAmount(Math.min(available, Number(e.target.value.replace(/[^0-9.]/g, '')) || 0))} />
          </span>
        </div>
        <input className="slider" type="range" min={0} max={Math.max(1, Math.floor(available))} step={1} value={amount} style={fill(amount, 0, Math.floor(available))} onChange={(e) => setAmount(Number(e.target.value))} aria-label="Amount" />
      </div>
      <div className="summary">
        <span>
          If {symbol} moves 5%<b>{side === 'down' ? '' : '+'}{usd(amount * 0.05)}</b>
        </span>
        <span style={{ textAlign: 'right' }}>
          Most you can lose<b>{usd(amount)}</b>
        </span>
      </div>
      {error && <p className="error">{error}</p>}
      <button className={`cta${side === 'up' ? ' cta-up' : ''}`} disabled={!side || busy || amount < 1} onClick={place}>
        {busy ? 'Placing…' : side ? 'Make the call' : 'Pick Up or Down'}
      </button>
    </Sheet>
  );
}

export function Receipt({ trade, symbol, onClose }: { trade: Trade; symbol: string; onClose: () => void }) {
  return (
    <Modal onClose={onClose}>
      <h3>Call placed</h3>
      <p>Your {symbol} call is in your fund.</p>
      <dl className="receipt">
        <div>
          <dt>Direction</dt>
          <dd className={trade.side > 0 ? 'side-up' : 'side-down'}>{trade.side > 0 ? 'UP' : 'DOWN'}</dd>
        </div>
        <div>
          <dt>Entry Price</dt>
          <dd>{price(trade.entry)}</dd>
        </div>
        <div>
          <dt>Amount</dt>
          <dd>{usd(trade.stake)}</dd>
        </div>
      </dl>
      <button className="cta" onClick={onClose}>
        Done
      </button>
    </Modal>
  );
}

export function CashedOut({ trade, onClose }: { trade: Trade; onClose: () => void }) {
  const pnl = trade.close?.pnl ?? 0;
  const minutes = Math.max(1, Math.round(((trade.close?.at ?? Date.now()) - trade.opened) / 60000));
  return (
    <Modal onClose={onClose}>
      <h3>Call closed</h3>
      <span className={`big ${pnl >= 0 ? 'change-up' : 'change-down'}`}>{signedUsd(pnl)}</span>
      <p>
        {usd(trade.stake + pnl)} back from {usd(trade.stake)} · {minutes < 60 ? `${minutes}m` : `${Math.floor(minutes / 60)}h ${minutes % 60}m`}
      </p>
      <div style={{ height: 20 }} />
      <button className="cta" onClick={onClose}>
        Done
      </button>
    </Modal>
  );
}

/** Close a trade, then show what came back. */
export async function cashOut(id: string): Promise<{ me: Me; trade?: Trade }> {
  const me = await closeTrade(id);
  navigator.vibrate?.(12);
  return { me, trade: me.history.find((h) => h.trade.id === id)?.trade };
}

/** Put credits into an AI fund. */
export function BackSheet({ fund, name, available, held, onClose, onDone }: { fund: string; name: string; available: number; held: number; onClose: () => void; onDone: (me: Me) => void }) {
  const [amount, setAmount] = useState(Math.floor(available));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const go = (credits: number) => {
    setBusy(true);
    backFund(fund, credits)
      .then(onDone)
      .catch((e: Error) => setError(e.message))
      .finally(() => setBusy(false));
  };
  return (
    <Sheet onClose={onClose}>
      <div className="sheet-head">
        <h3>Hire {name}</h3>
        <span className="num">{usd(available)} available</span>
      </div>
      <div className="field">
        <div className="field-head">
          <label htmlFor="back-amount">Amount</label>
          <span className="field-value">
            $<input id="back-amount" inputMode="decimal" value={amount} onChange={(e) => setAmount(Math.min(available, Number(e.target.value.replace(/[^0-9.]/g, '')) || 0))} />
          </span>
        </div>
        <input className="slider" type="range" min={0} max={Math.max(1, Math.floor(available))} step={1} value={amount} style={fill(amount, 0, Math.floor(available))} onChange={(e) => setAmount(Number(e.target.value))} aria-label="Amount" />
      </div>
      <div className="summary">
        <span>
          Already in<b>{usd(held)}</b>
        </span>
        <span style={{ textAlign: 'right' }}>
          After<b>{usd(held + amount)}</b>
        </span>
      </div>
      {error && <p className="error">{error}</p>}
      <div className="sides">
        {held > 0 && (
          <button className="cta" style={{ background: 'var(--surface)', color: '#fff' }} disabled={busy} onClick={() => go(0)}>
            Fire
          </button>
        )}
        <button className="cta cta-up" disabled={busy || amount < 1} onClick={() => go(amount)}>
          {busy ? 'Hiring…' : `Give it ${usd(amount)}`}
        </button>
      </div>
    </Sheet>
  );
}
