import { useState } from 'react';
import { addMoney, useMarkets, useMe, type Fund as Manager, type Me } from './api.ts';
import { useMids } from './feed.ts';
import { signedUsd, usd } from './format.ts';
import { liveEquity, Positions } from './Home.tsx';
import { Change, Link, Logo, Spark, toneOf } from './parts.tsx';
import { fundPath } from './route.ts';
import { Sheet } from './sheets.tsx';
import { Ticker } from './Ticker.tsx';
import { ValueChart } from './ValueChart.tsx';

export const SLEEVE_COLOURS = ['#00ec4b', '#8cc8ff', '#ffb01f', '#c9b5ff', '#ff8fab'];

/** A manager's state in a few words, read from what it last decided. */
export function stateOf(m: Manager): string {
  if (m.note.startsWith('Out until')) return 'Waits for 10:00 NY';
  if (m.note.startsWith('Holding the morning')) return 'In until 16:00 NY';
  if (m.note.startsWith('Paused')) return 'Paused';
  if (m.holdings.length === 0) return 'In cash';
  return m.holdings.length === 1 ? 'Holding 1 market' : `Holding ${m.holdings.length} markets`;
}

function AddSheet({ onClose, onDone }: { onClose: () => void; onDone: (me: Me) => void }) {
  const [amount, setAmount] = useState(100);
  return (
    <Sheet onClose={onClose}>
      <div className="sheet-head">
        <h3>Add practice money</h3>
        <span>Real USDC soon</span>
      </div>
      <div className="start-presets">
        {[50, 100, 500, 1000].map((a) => (
          <button key={a} className={`chip${a === amount ? ' chip-on' : ''}`} onClick={() => setAmount(a)}>
            ${a}
          </button>
        ))}
      </div>
      <p className="about">It lands as cash in your fund. Hire a manager to put it to work.</p>
      <button className="cta cta-up" onClick={() => addMoney(amount).then(onDone)}>
        Add {usd(amount)}
      </button>
    </Sheet>
  );
}

export function FundHome({ me, setMe }: { me: Me; setMe: (me: Me) => void }) {
  const [data] = useMarkets();
  const mids = useMids();
  const [adding, setAdding] = useState(false);
  const equity = liveEquity(me, mids);
  const dayAgo = Date.now() - 86_400_000;
  const then = [...me.curve].reverse().find(([t]) => t <= dayAgo)?.[1] ?? me.start;
  const today = equity - then;
  const managers = new Map((data?.funds ?? []).map((f) => [f.id, f]));
  const calls = me.open.reduce((sum, o) => sum + o.trade.stake + Math.max(-o.trade.stake, o.trade.stake * o.trade.multiplier * o.trade.side * ((mids.get(o.trade.market) ?? o.trade.entry) / o.trade.entry - 1)), 0);
  const parts = [
    ...me.backed.map((b, i) => ({ key: b.fund, name: b.name, value: b.value, colour: SLEEVE_COLOURS[i % SLEEVE_COLOURS.length] })),
    ...(calls > 0 ? [{ key: 'calls', name: 'Your calls', value: calls, colour: '#ffffff' }] : []),
    { key: 'cash', name: 'Cash for your calls', value: Math.max(0, me.balance), colour: '#3a3a3a' },
  ];
  const total = parts.reduce((s, p) => s + p.value, 0) || 1;
  const hired = me.backed.map((b) => managers.get(b.fund)).filter((m): m is Manager => !!m);
  const fees = (data?.funds ?? []).length ? me.history.reduce((s, h) => s + h.trade.fee, 0) + me.open.reduce((s, o) => s + o.trade.fee, 0) : 0;
  return (
    <>
      <header className="topbar">
        <span className="fund-name">
          {me.fund}
          <span className="pill">Practice</span>
        </span>
      </header>
      <Ticker className="hero-value num" value={equity} text={usd(equity)} />
      <span className="hero-sub">
        <span className={today >= 0 ? 'change-up' : 'change-down'}>
          {signedUsd(today)} (<Change value={then ? today / then : 0} />) today
        </span>
        <span className="muted">
          {' '}
          · <Change value={me.start ? equity / me.start - 1 : 0} /> since start
        </span>
      </span>
      <ValueChart points={me.curve} start={me.start} />

      <div className="alloc">
        <span className="mixbar mixbar-big" aria-hidden="true">
          {parts.map((p) => (
            <i key={p.key} style={{ width: `${(p.value / total) * 100}%`, background: p.colour }} />
          ))}
        </span>
        <span className="legend">
          {parts.map((p) => (
            <span key={p.key}>
              <i style={{ background: p.colour }} />
              {p.name} {Math.round((p.value / total) * 100)}%
            </span>
          ))}
        </span>
      </div>

      {hired.length > 0 && (
        <section className="today">
          <h2>Today</h2>
          {hired.map((m) => (
            <p key={m.id}>
              <b>{m.name}.</b> {m.note || 'Waiting for its first decision.'}
            </p>
          ))}
        </section>
      )}

      <div className="head">
        <h2>Managers</h2>
        <Link to="/managers">Hire</Link>
      </div>
      <ul className="list">
        {me.backed.map((b, i) => {
          const m = managers.get(b.fund);
          return (
            <li key={b.fund}>
              <Link to={fundPath(b.fund)} className="row">
                <span className="swatch" style={{ background: SLEEVE_COLOURS[i % SLEEVE_COLOURS.length] }} />
                <span className="row-main">
                  <span className="row-title">{b.name}</span>
                  <span className="row-sub">
                    {Math.round((b.value / total) * 100)}% · {m ? stateOf(m) : ''}
                  </span>
                </span>
                {m && <Spark points={m.curve} tone={toneOf(m.change)} />}
                <span className="row-end">
                  <span className="row-price">{usd(b.value)}</span>
                  <span className={`change ${b.value >= b.cost ? 'change-up' : 'change-down'}`}>{signedUsd(b.value - b.cost)}</span>
                </span>
              </Link>
            </li>
          );
        })}
      </ul>
      {me.backed.length === 0 && <p className="empty">No managers yet. Hire one to put your cash to work.</p>}

      {me.open.length > 0 && (
        <>
          <div className="head">
            <h2>Your calls</h2>
            <Link to="/markets">Markets</Link>
          </div>
          <Positions me={me} onChange={setMe} />
        </>
      )}

      <p className="proof">Live prices from Hyperliquid · every manager trade on a verified record · fees {usd(fees)}</p>

      <div className="dock dock-tabs">
        <button className="cta cta-up" onClick={() => setAdding(true)}>
          Add money
        </button>
      </div>
      {adding && (
        <AddSheet
          onClose={() => setAdding(false)}
          onDone={(next) => {
            setMe(next);
            setAdding(false);
          }}
        />
      )}
    </>
  );
}

export function FundScreen() {
  const [me, setMe] = useMe();
  if (!me) return <div className="loading" aria-busy="true" />;
  return <FundHome me={me} setMe={setMe} />;
}

export { Logo };
