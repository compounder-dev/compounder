import { useEffect, useState, type ReactNode } from 'react';
import { closeTrade, copyStrategy, openTrade, type Me, type Trade } from './api.ts';
import { price, signedUsd, usd } from './format.ts';
import { shareCard } from './share.ts';

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

const LEVERAGE = [1, 2, 5, 10, 20];
/** The fee on a trade's size, as the server charges it. */
const FEE = 0.0009;
const cents = (n: number) => Math.max(0, Math.floor(n * 100) / 100);

/** Pick a side, an amount and leverage, and place it. */
export function TradeSheet({ market, symbol, available, maxLeverage, onClose, onPlaced }: { market: string; symbol: string; available: number; maxLeverage: number; onClose: () => void; onPlaced: (me: Me) => void }) {
  const [side, setSide] = useState<'up' | 'down' | null>(null);
  const [wanted, setAmount] = useState(Math.max(0, Math.floor(available / 2)));
  const [multiplier, setMultiplier] = useState(1);
  // The most you can put in leaves room for the fee on the trade's size.
  const most = cents(available / (1 + multiplier * FEE));
  const amount = Math.min(wanted, most);
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
        <h3>Trade {symbol}</h3>
        <span className="num">{usd(available)} available</span>
      </div>
      <div className="sides">
        <button className={`go go-up${side === 'up' ? ' go-on' : side ? ' go-dim' : ''}`} onClick={() => setSide('up')}>
          Long
        </button>
        <button className={`go go-down${side === 'down' ? ' go-on' : side ? ' go-dim' : ''}`} onClick={() => setSide('down')}>
          Short
        </button>
      </div>
      <div className="field">
        <div className="field-head">
          <label htmlFor="amount">Amount</label>
          <span className="field-value">
            $<input id="amount" inputMode="decimal" value={amount} onChange={(e) => setAmount(cents(Number(e.target.value.replace(/[^0-9.]/g, '')) || 0))} />
          </span>
        </div>
        <input className="slider" type="range" min={0} max={Math.max(1, most)} step={0.01} value={amount} style={fill(amount, 0, most)} onChange={(e) => setAmount(Number(e.target.value))} aria-label="Amount" />
      </div>
      <div className="start-presets">
        {LEVERAGE.filter((x) => x <= maxLeverage).map((x) => (
          <button key={x} className={`chip${x === multiplier ? ' chip-on' : ''}`} onClick={() => setMultiplier(x)}>
            {x}x
          </button>
        ))}
      </div>
      <div className="summary">
        <span>
          If {symbol} moves 5% your way<b>+{usd(amount * multiplier * 0.05)}</b>
        </span>
        <span style={{ textAlign: 'right' }}>
          Most you can lose<b>{usd(amount)}</b>
        </span>
      </div>
      {error && <p className="error">{error}</p>}
      <button className={`cta${side === 'up' ? ' cta-up' : ''}`} disabled={!side || busy || amount < 1} onClick={place}>
        {busy ? 'Placing…' : side ? `${side === 'up' ? 'Long' : 'Short'} ${symbol} ${multiplier}x` : 'Pick Long or Short'}
      </button>
    </Sheet>
  );
}

export function Receipt({ trade, symbol, onClose }: { trade: Trade; symbol: string; onClose: () => void }) {
  return (
    <Modal onClose={onClose}>
      <h3>Trade open</h3>
      <p>Your {symbol} trade is in your fund.</p>
      <dl className="receipt">
        <div>
          <dt>Side</dt>
          <dd className={trade.side > 0 ? 'side-up' : 'side-down'}>{trade.side > 0 ? 'LONG' : 'SHORT'} {trade.multiplier}x</dd>
        </div>
        <div>
          <dt>Entry</dt>
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
      <h3>Trade closed</h3>
      <span className={`big change-${pnl > 0 ? 'up' : pnl < 0 ? 'down' : 'flat'}`}>{signedUsd(pnl)}</span>
      <p>
        {usd(trade.stake + pnl)} back from {usd(trade.stake)} · {minutes < 60 ? `${minutes}m` : `${Math.floor(minutes / 60)}h ${minutes % 60}m`}
      </p>
      <div style={{ height: 20 }} />
      <div className="sides">
        <button className="cta" onClick={() => shareCard({ title: `${trade.side > 0 ? 'Long' : 'Short'} ${trade.market.split(':').pop()} ${trade.multiplier}x`, big: signedUsd(pnl), tone: pnl > 0 ? 'up' : pnl < 0 ? 'down' : 'flat', sub: `${((pnl / trade.stake) * 100).toFixed(1)}% on ${usd(trade.stake)}` })}>
          Share
        </button>
        <button className="cta cta-up" onClick={onClose}>
          Done
        </button>
      </div>
    </Modal>
  );
}

/** Close a trade, then show what came back. */
export async function cashOut(id: string): Promise<{ me: Me; trade?: Trade }> {
  const me = await closeTrade(id);
  navigator.vibrate?.(12);
  return { me, trade: me.history.find((h) => h.trade.id === id)?.trade };
}

/** Copy a strategy with an amount, change it, or stop. */
export function CopySheet({ fund, name, available, held, onClose, onDone }: { fund: string; name: string; available: number; held: number; onClose: () => void; onDone: (me: Me) => void }) {
  const [amount, setAmount] = useState(cents(available));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const go = (credits: number) => {
    setBusy(true);
    copyStrategy(fund, credits)
      .then(onDone)
      .catch((e: Error) => setError(e.message))
      .finally(() => setBusy(false));
  };
  return (
    <Sheet onClose={onClose}>
      <div className="sheet-head">
        <h3>Copy {name}</h3>
        <span className="num">{usd(available)} available</span>
      </div>
      <div className="field">
        <div className="field-head">
          <label htmlFor="back-amount">Amount</label>
          <span className="field-value">
            $<input id="back-amount" inputMode="decimal" value={amount} onChange={(e) => setAmount(Math.min(cents(available), cents(Number(e.target.value.replace(/[^0-9.]/g, '')) || 0)))} />
          </span>
        </div>
        <input className="slider" type="range" min={0} max={Math.max(1, cents(available))} step={0.01} value={amount} style={fill(amount, 0, cents(available))} onChange={(e) => setAmount(Number(e.target.value))} aria-label="Amount" />
      </div>
      <div className="summary">
        <span>
          Copying now<b>{usd(held)}</b>
        </span>
        <span style={{ textAlign: 'right' }}>
          After<b>{usd(held + amount)}</b>
        </span>
      </div>
      {error && <p className="error">{error}</p>}
      <div className="sides">
        {held > 0 && (
          <button className="cta" style={{ background: 'var(--surface)', color: '#fff' }} disabled={busy} onClick={() => go(0)}>
            Stop copying
          </button>
        )}
        <button className="cta cta-up" disabled={busy || amount < 1} onClick={() => go(amount)}>
          {busy ? 'Copying…' : `Copy with ${usd(amount)}`}
        </button>
      </div>
    </Sheet>
  );
}
