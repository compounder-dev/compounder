import { useState } from 'react';
import { useMarkets, useMe, type Trade } from './api.ts';
import { Chart, RANGES, type Range } from './Chart.tsx';
import { useMid } from './feed.ts';
import { price } from './format.ts';
import { Positions } from './Home.tsx';
import { Change, TopBar } from './parts.tsx';
import { Receipt, TradeSheet } from './sheets.tsx';

const ABOUT: Record<string, string> = {
  commodities: 'Trades around the clock, priced from the world market.',
  indices: 'The whole index in one trade, around the clock.',
  stocks: 'A tokenised share that trades 24/7, even when Wall Street is shut.',
  crypto: 'Trades around the clock.',
};

export function Asset({ id }: { id: string }) {
  const [data] = useMarkets();
  const [me, setMe] = useMe();
  const market = data?.markets.find((m) => m.id === id);
  const mid = useMid(id);
  const now = mid ?? market?.price ?? null;
  const [range, setRange] = useState<Range>('1D');
  const [start, setStart] = useState<number | null>(null);
  const [trading, setTrading] = useState(false);
  const [placed, setPlaced] = useState<Trade | null>(null);
  const symbol = market?.symbol ?? id.split(':').pop() ?? id;
  const latest = me?.open.find((o) => o.trade.market === id)?.trade;
  const change = now && start ? now / start - 1 : null;
  const holders = (data?.funds ?? []).filter((m) => m.holdings.some((h) => h.market === symbol));
  return (
    <>
      <TopBar
        backTo="/markets"
        title={
          <>
            <b>{symbol}</b>
            <span>{market?.name ?? ''}</span>
          </>
        }
      />
      <div className="quote">
        <span className="quote-price">{price(now)}</span>
        <Change value={change} />
      </div>
      <Chart key={`${id}-${range}`} coin={id} range={range} entry={latest?.entry} onFirst={setStart} />
      <nav className="intervals" aria-label="Chart range">
        {(Object.keys(RANGES) as Range[]).map((r) => (
          <button key={r} className={`interval${r === range ? ' interval-on' : ''}`} onClick={() => setRange(r)}>
            {r}
          </button>
        ))}
      </nav>
      <div className="head">
        <h2>Your calls</h2>
      </div>
      {me && <Positions me={me} market={id} onChange={setMe} />}
      <div className="head">
        <h2>About {market?.name ?? symbol}</h2>
      </div>
      <p className="about">{market ? ABOUT[market.kind] : ''}</p>
      {holders.length > 0 && (
        <>
          <div className="head">
            <h2>Managers holding it</h2>
          </div>
          <ul className="list">
            {holders.map((m) => (
              <li key={m.id} className="row">
                <span className="row-main">
                  <span className="row-title">{m.name}</span>
                  <span className="row-sub">{m.holdings.find((h) => h.market === symbol)!.weight > 0 ? 'Long' : 'Short'}</span>
                </span>
              </li>
            ))}
          </ul>
        </>
      )}
      <div className="dock">
        <button className="cta" disabled={!me || !now} onClick={() => setTrading(true)}>
          Make a call on {symbol}
        </button>
      </div>
      {trading && me && (
        <TradeSheet
          market={id}
          symbol={symbol}
          available={me.balance}
          onClose={() => setTrading(false)}
          onPlaced={(next) => {
            setMe(next);
            setTrading(false);
            setPlaced(next.open.find((o) => o.trade.market === id)?.trade ?? null);
          }}
        />
      )}
      {placed && <Receipt trade={placed} symbol={symbol} onClose={() => setPlaced(null)} />}
    </>
  );
}
