import { useState } from 'react';
import { signIn, useMarkets, useMe, useTokens, type Trade } from './api.ts';
import { Chart, RANGES, type Range } from './Chart.tsx';
import { useMid } from './feed.ts';
import { price } from './format.ts';
import { Positions } from './Markets.tsx';
import { Change, Link, TopBar } from './parts.tsx';
import { fundPath } from './route.ts';
import { BuySheet } from './Buy.tsx';
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
  const [buying, setBuying] = useState(false);
  // The same company as a tokenised share, when this market has one behind it.
  const token = useTokens().find((each) => each.ticker === market?.symbol && each.chain_id === 56);
  const symbol = market?.symbol ?? id.split(':').pop() ?? id;
  const latest = me ? me.open.find((o) => o.trade.market === id)?.trade : undefined;
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
      {me && (
        <>
          <div className="head">
            <h2>Your trades</h2>
          </div>
          <Positions me={me} market={id} onChange={setMe} />
        </>
      )}
      <div className="head">
        <h2>About {market?.name ?? symbol}</h2>
      </div>
      <p className="about">{market ? ABOUT[market.kind] : ''}</p>
      {holders.length > 0 && (
        <>
          <div className="head">
            <h2>Strategies holding it</h2>
          </div>
          <ul className="list">
            {holders.map((m) => (
              <li key={m.id}>
                <Link to={fundPath(m.id)} className="row">
                  <span className="row-main">
                    <span className="row-title">{m.name}</span>
                    <span className="row-sub">{m.holdings.find((h) => h.market === symbol)!.weight > 0 ? 'Long' : 'Short'}</span>
                  </span>
                </Link>
              </li>
            ))}
          </ul>
        </>
      )}
      <div className="dock">
        <div className="dock-stack">
          {token && (
            <button className="cta cta-up" onClick={() => setBuying(true)}>
              Buy {token.symbol} — real
            </button>
          )}
          <button className={`cta${token ? '' : ' cta-up'}`} disabled={me === null || !now} onClick={() => (me === false ? signIn() : setTrading(true))}>
            {token ? `Trade ${symbol} · practice` : `Trade ${symbol}`}
          </button>
        </div>
      </div>
      {buying && token && <BuySheet token={token} onClose={() => setBuying(false)} />}
      {trading && me && (
        <TradeSheet
          market={id}
          symbol={symbol}
          available={me.balance}
          maxLeverage={market?.max_multiplier ?? 1}
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
