import { useState } from 'react';
import { addMoney, signIn, useMarkets, useMe, type Fund as Strategy, type Me } from './api.ts';
import { useMids } from './feed.ts';
import { signedUsd, usd } from './format.ts';
import { liveEquity, pnlAt, Positions } from './Markets.tsx';
import { Change, Link, managerColour, Money, toneOf } from './parts.tsx';
import { fundPath } from './route.ts';
import { shareCard } from './share.ts';
import { Sheet } from './sheets.tsx';
import { Start } from './Start.tsx';
import { stateOf } from './Strategy.tsx';
import { Ticker } from './Ticker.tsx';
import { ValueChart } from './ValueChart.tsx';

function AddSheet({ onClose, onDone }: { onClose: () => void; onDone: (me: Me) => void }) {
  const [amount, setAmount] = useState(100);
  return (
    <Sheet onClose={onClose}>
      <div className="sheet-head">
        <h3>Add practice money</h3>
      </div>
      <div className="start-presets">
        {[50, 100, 500, 1000].map((a) => (
          <button key={a} className={`chip${a === amount ? ' chip-on' : ''}`} onClick={() => setAmount(a)}>
            ${a}
          </button>
        ))}
      </div>
      <button className="cta cta-up" onClick={() => addMoney(amount).then(onDone)}>
        Add {usd(amount)}
      </button>
    </Sheet>
  );
}

function YourFund({ me, setMe }: { me: Me; setMe: (me: Me) => void }) {
  const [data] = useMarkets();
  const mids = useMids();
  const [adding, setAdding] = useState(false);
  const equity = liveEquity(me, mids);
  const dayAgo = Date.now() - 86_400_000;
  const then = [...me.curve].reverse().find(([t]) => t <= dayAgo)?.[1] ?? me.start;
  const today = equity - then;
  const strategies = new Map((data?.funds ?? []).map((f) => [f.id, f]));
  const trades = me.open.reduce((sum, o) => sum + o.trade.stake + pnlAt(o.trade, mids.get(o.trade.market) ?? o.trade.entry), 0);
  const parts = [
    ...me.backed.map((b) => ({ key: b.fund, name: b.name, value: b.value, colour: managerColour(b.fund) })),
    ...(trades > 0 ? [{ key: 'trades', name: 'Your trades', value: trades, colour: '#ffffff' }] : []),
    { key: 'cash', name: 'Cash', value: Math.max(0, me.balance), colour: '#3a3a3a' },
  ];
  const total = parts.reduce((s, p) => s + p.value, 0) || 1;
  const copying = me.backed.map((b) => strategies.get(b.fund)).filter((m): m is Strategy => !!m);
  return (
    <>
      <header className="topbar">
        <span className="wordmark">Your fund</span>
        <span className="topbar-right">
          <span className="pill">Practice</span>
        </span>
      </header>
      <div className="panel-row">
        <span className="portfolio-label">Value</span>
        <span className="portfolio-label">Cash</span>
      </div>
      <div className="panel-row panel-values">
        <Ticker className="hero-value num" value={equity} text={<Money value={equity} />} />
        <Money className="panel-cash num" value={Math.max(0, me.balance)} />
      </div>
      <span className="hero-sub">
        <span className={`change-${toneOf(then ? today / then : 0)}`}>{signedUsd(Math.abs(today) < 0.005 ? 0 : today)} today</span>
        <span className="muted">
          {' '}
          · <Change value={me.start ? equity / me.start - 1 : 0} /> all time
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

      <div className="head">
        <h2>Copying</h2>
        <Link to="/strategies">Add</Link>
      </div>
      <ul className="list group">
        {me.backed.map((b) => {
          const s = strategies.get(b.fund);
          return (
            <li key={b.fund}>
              <Link to={fundPath(b.fund)} className="row">
                <span className="swatch" style={{ background: managerColour(b.fund) }} />
                <span className="row-main">
                  <span className="row-title">{b.name}</span>
                  <span className="row-sub">{s ? s.note || stateOf(s) : ''}</span>
                </span>
                <span className="row-end">
                  <span className="row-price">{usd(b.value)}</span>
                  <span className={`change change-${toneOf(b.cost ? b.value / b.cost - 1 : 0)}`}>{signedUsd(Math.abs(b.value - b.cost) < 0.005 ? 0 : b.value - b.cost)}</span>
                </span>
              </Link>
            </li>
          );
        })}
      </ul>
      {copying.length === 0 && <p className="empty">Not copying anything. Pick a strategy to put your cash to work.</p>}

      <div className="head">
        <h2>Your trades</h2>
        <Link to="/markets">Trade</Link>
      </div>
      <Positions me={me} onChange={setMe} />

      <div className="dock dock-tabs">
        <div className="sides">
          <button className="cta" onClick={() => shareCard({ title: 'Your fund', big: signedUsd(equity - me.start), tone: toneOf(equity - me.start), sub: `${usd(equity)} · practice` })}>
            Share
          </button>
          <button className="cta cta-up" onClick={() => setAdding(true)}>
            Add money
          </button>
        </div>
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

/** Your own fund: sign in, start it, then watch it. */
export function You() {
  const [me, setMe] = useMe();
  if (me === null) return <div className="loading" aria-busy="true" />;
  if (me === false)
    return (
      <div className="start">
        <span className="wordmark">compounder</span>
        <h1 className="start-title">Copy the fund</h1>
        <p className="about">Sign in to start your own fund with the same strategies.</p>
        <div className="dock dock-tabs">
          <button className="cta cta-up" onClick={() => signIn('/you')}>
            Sign in
          </button>
        </div>
      </div>
    );
  if (!me.fund) return <Start onStarted={setMe} />;
  return <YourFund me={me} setMe={setMe} />;
}
