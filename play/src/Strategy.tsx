import { useState } from 'react';
import { useMarkets, useMe } from './api.ts';
import { pct, usd } from './format.ts';
import { Change, Link, Logo, Spark, toneOf, TopBar } from './parts.tsx';
import { fundPath } from './route.ts';
import { BackSheet } from './sheets.tsx';

const kind = (f: { model?: string; speed: string }) => (f.model ? 'AI model' : f.speed === 'velocity' ? 'Fast' : 'Long-term');
const daysLive = (f: { curve: [number, number][] }) => Math.max(1, Math.ceil((Date.now() - (f.curve[0]?.[0] ?? Date.now())) / 86_400_000));

export function Managers() {
  const [data] = useMarkets();
  const [me] = useMe();
  return (
    <>
      <header className="topbar">
        <span className="wordmark">Managers</span>
      </header>
      {!data && <div className="loading" aria-busy="true" />}
      {data?.funds.map((fund) => (
        <Link key={fund.id} to={fundPath(fund.id)} className="strategy-card">
          <span className="strategy-top">
            <Logo symbol={fund.name} />
            <span className="row-main">
              <span className="row-title">{fund.name}</span>
              <span className="row-sub">
                {kind(fund)} · day {daysLive(fund)} live{me?.backed.some((b) => b.fund === fund.id) ? ' · Hired' : ''}
              </span>
            </span>
            <Spark points={fund.curve} tone={toneOf(fund.change)} />
            <Change value={fund.change} />
          </span>
          <p className="strategy-note">{fund.thesis}</p>
        </Link>
      ))}
    </>
  );
}

export function ManagerPage({ id }: { id: string }) {
  const [data] = useMarkets();
  const [me, setMe] = useMe();
  const [adding, setAdding] = useState(false);
  const fund = data?.funds.find((f) => f.id === id);
  if (!fund) return <TopBar backTo="/managers" />;
  const held = me?.backed.find((b) => b.fund === id);
  return (
    <>
      <TopBar
        backTo="/managers"
        title={
          <>
            <b>{fund.name}</b>
            <span>{kind(fund)}</span>
          </>
        }
      />
      <div className="quote">
        <span className="pill pill-live">Day {daysLive(fund)} live on paper</span>
        <span className="quote-price" style={{ marginTop: 8 }}>
          {pct(fund.change)}
        </span>
        <span className="row-sub">{usd(fund.equity)} from {usd(fund.capital)}</span>
      </div>
      <div className="fund-chart">
        <Spark points={fund.curve} tone={toneOf(fund.change)} />
      </div>
      <dl className="stats">
        <div>
          <dt>Tested yearly</dt>
          <dd>{fund.test ? pct(fund.test.annual_return) : '–'}</dd>
        </div>
        <div>
          <dt>Worst drop</dt>
          <dd>{pct(-fund.worst_drop)}</dd>
        </div>
        <div>
          <dt>Trades</dt>
          <dd>{fund.fills}</dd>
        </div>
      </dl>
      <div className="head">
        <h2>Right now</h2>
      </div>
      <p className="about">{fund.note || 'Waiting for its first decision.'}</p>
      {fund.holdings.length > 0 && (
        <ul className="list">
          {fund.holdings.map((h) => (
            <li key={h.market} className="row">
              <Logo symbol={h.market} kind="stocks" />
              <span className="row-main">
                <span className="row-title">{h.market}</span>
                <span className={`row-sub ${h.weight > 0 ? 'side-up' : 'side-down'}`}>{h.weight > 0 ? 'Long' : 'Short'}</span>
              </span>
              <span className="row-end row-price">{pct(Math.abs(h.weight), false)}</span>
            </li>
          ))}
        </ul>
      )}
      <div className="head">
        <h2>How it trades</h2>
      </div>
      <p className="about">{fund.rules}</p>
      <p className="about" style={{ fontSize: 13 }}>
        {fund.verified ? `Every decision and trade is on a tamper-proof record: ${fund.entries} entries.` : 'Its record does not check out.'}
      </p>
      <div className="dock">
        <button className="cta cta-up" disabled={!me} onClick={() => setAdding(true)}>
          {held ? `Running ${usd(held.value)} of your fund` : 'Hire'}
        </button>
      </div>
      {adding && me && (
        <BackSheet
          fund={id}
          name={fund.name}
          available={me.balance}
          held={held?.value ?? 0}
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
