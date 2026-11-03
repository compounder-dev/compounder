import { useState } from 'react';
import { useMarkets, type Market, type Me } from './api.ts';
import { useMids } from './feed.ts';
import { price, signedUsd, usd } from './format.ts';
import { Change, Link, Logo } from './parts.tsx';
import { tradePath } from './route.ts';
import { cashOut, CashedOut } from './sheets.tsx';
import type { Trade } from './api.ts';

const CHIPS = [
  { key: 'all', label: 'All' },
  { key: 'stocks', label: 'Stocks' },
  { key: 'commodities', label: 'Commodities' },
  { key: 'indices', label: 'Indices' },
  { key: 'crypto', label: 'Crypto' },
] as const;

/** Live value of an open trade at a price. */
export const pnlAt = (t: Trade, px: number) => Math.max(-t.stake, t.stake * t.multiplier * t.side * (px / t.entry - 1));

export function liveEquity(me: Me, mids: Map<string, number>) {
  const trades = me.open.reduce((sum, o) => sum + o.trade.stake + pnlAt(o.trade, mids.get(o.trade.market) ?? o.price ?? o.trade.entry), 0);
  return me.balance + trades + me.backed.reduce((sum, b) => sum + b.value, 0);
}

export function Positions({ me, market, onChange }: { me: Me; market?: string; onChange: (me: Me) => void }) {
  const mids = useMids();
  const [done, setDone] = useState<Trade | null>(null);
  const open = me.open.filter((o) => !market || o.trade.market === market);
  return (
    <>
      {open.length === 0 ? (
        <p className="empty">{market ? 'No calls on this market yet.' : 'No calls yet. Pick a market to make one.'}</p>
      ) : (
        <ul className="list group">
          {open.map(({ trade, symbol }) => {
            const pnl = pnlAt(trade, mids.get(trade.market) ?? trade.entry);
            return (
              <li key={trade.id} className="row">
                <Link to={tradePath(trade.market)} className="row-link">
                  <Logo symbol={symbol} kind="stocks" />
                  <span className="row-main">
                    <span className="row-title">{symbol}</span>
                    <span className="row-sub">
                      <span className={trade.side > 0 ? 'side side-up' : 'side side-down'}>{trade.side > 0 ? 'UP' : 'DOWN'}</span> · {usd(trade.stake)}
                    </span>
                  </span>
                  <span className="row-end">
                    <span className={`row-price ${pnl >= 0 ? 'change-up' : 'change-down'}`}>{signedUsd(pnl)}</span>
                    <span className="row-sub num">{((pnl / trade.stake) * 100).toFixed(1)}%</span>
                  </span>
                </Link>
                <button
                  className={`cashout${pnl > 0 ? ' cashout-up' : ''}`}
                  onClick={() =>
                    cashOut(trade.id).then(({ me: next, trade: closed }) => {
                      onChange(next);
                      if (closed) setDone(closed);
                    })
                  }
                >
                  Close
                </button>
              </li>
            );
          })}
        </ul>
      )}
      {done && <CashedOut trade={done} onClose={() => setDone(null)} />}
    </>
  );
}

function MarketRow({ market, mid }: { market: Market; mid: number | undefined }) {
  const now = mid ?? market.price;
  const prev = market.price && market.change_day != null ? market.price / (1 + market.change_day) : null;
  return (
    <li>
      <Link to={tradePath(market.id)} className="row">
        <Logo symbol={market.symbol} kind={market.kind} />
        <span className="row-main">
          <span className="row-title">{market.symbol}</span>
          <span className="row-sub">{market.name}</span>
        </span>
        <span className="row-end">
          <span className="row-price">${price(now)}</span>
          <Change value={now && prev ? now / prev - 1 : market.change_day} />
        </span>
      </Link>
    </li>
  );
}

export function Markets() {
  const [data] = useMarkets();
  const mids = useMids();
  const [chip, setChip] = useState<(typeof CHIPS)[number]['key']>('all');
  const [query, setQuery] = useState('');
  const q = query.trim().toLowerCase();
  const markets = (data?.markets ?? []).filter((m) => (chip === 'all' || m.kind === chip) && (!q || m.symbol.toLowerCase().includes(q) || m.name.toLowerCase().includes(q)));
  return (
    <>
      <header className="topbar">
        <span className="wordmark">Markets</span>
        <span className="topbar-right">
          <span className="pill pill-live">Open 24/7</span>
        </span>
      </header>
      <label className="search">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
          <circle cx="11" cy="11" r="7" />
          <path d="m20 20-3.5-3.5" />
        </svg>
        <input placeholder="Search markets" value={query} onChange={(e) => setQuery(e.target.value)} />
      </label>
      <nav className="chips" aria-label="Market kinds">
        {CHIPS.map((c) => (
          <button key={c.key} className={`chip${chip === c.key ? ' chip-on' : ''}`} onClick={() => setChip(c.key)}>
            {c.label}
          </button>
        ))}
      </nav>
      {!data && <div className="loading" aria-busy="true" />}
      <ul className="list group">
        {markets.map((m) => (
          <MarketRow key={m.id} market={m} mid={mids.get(m.id)} />
        ))}
      </ul>
      {data && markets.length === 0 && <p className="empty">No markets found.</p>}
    </>
  );
}
