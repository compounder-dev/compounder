import { useState } from 'react';
import { useMarkets, useMe, type Fund } from './api.ts';
import { stateOf } from './Fund.tsx';
import { pct, signedUsd, usd } from './format.ts';
import { Change, Link, ManagerMark, Spark, toneOf, TopBar } from './parts.tsx';
import { fundPath } from './route.ts';
import { BackSheet } from './sheets.tsx';
import { ValueChart } from './ValueChart.tsx';

const kind = (f: { model?: string; speed: string }) => (f.model ? 'AI model' : f.speed === 'velocity' ? 'Fast' : 'Long-term');
const daysLive = (f: { curve: [number, number][] }) => Math.max(1, Math.ceil((Date.now() - (f.curve[0]?.[0] ?? Date.now())) / 86_400_000));
const day = (t: number) => new Date(t).toLocaleDateString(undefined, { day: 'numeric', month: 'short' });

export function Managers() {
  const [data] = useMarkets();
  const [me] = useMe();
  return (
    <>
      <header className="topbar">
        <span className="wordmark">Managers</span>
      </header>
      {!data && <div className="loading" aria-busy="true" />}
      {data?.funds.map((fund) => {
        const hired = me?.backed.some((b) => b.fund === fund.id);
        return (
          <Link key={fund.id} to={fundPath(fund.id)} className="strategy-card">
            <span className="strategy-top">
              <ManagerMark id={fund.id} name={fund.name} />
              <span className="row-main">
                <span className="row-title">
                  {fund.name}
                  {hired && <span className="tag tag-on">Hired</span>}
                </span>
                <span className="row-sub">
                  {kind(fund)} · {stateOf(fund)}
                </span>
              </span>
              <Spark points={fund.curve} tone={toneOf(fund.change)} />
              <Change value={fund.change} />
            </span>
            <p className="strategy-note">{fund.thesis}</p>
            <span className="row-sub">Day {daysLive(fund)} live on paper</span>
          </Link>
        );
      })}
    </>
  );
}

function Record({ fund }: { fund: Fund }) {
  return (
    <>
      <div className="head">
        <h2>Its record</h2>
        <span className="pill pill-live">Day {daysLive(fund)} live</span>
      </div>
      <ValueChart points={fund.curve} start={fund.capital} height={160} />
      <dl className="stats">
        <div>
          <dt>Live</dt>
          <dd className={`change-${toneOf(fund.change)}`}>{pct(fund.change)}</dd>
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
        <span className="row-sub">{stateOf(fund)}</span>
      </div>
      <p className="about">{fund.note || 'Waiting for its first decision.'}</p>
      {fund.holdings.length > 0 && (
        <ul className="list">
          {fund.holdings.map((h) => (
            <li key={h.market} className="row">
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
      <p className="footnote">
        {fund.verified ? `Every decision and trade is on a tamper-proof record, ${fund.entries} entries so far.` : 'Its record does not check out.'}
        {fund.test ? ` Tested on past prices from ${day(fund.test.start)} to ${day(fund.test.end)}: ${pct(fund.test.annual_return)} a year. Past tests are not live results.` : ''}
      </p>
    </>
  );
}

export function ManagerPage({ id }: { id: string }) {
  const [data] = useMarkets();
  const [me, setMe] = useMe();
  const [sheet, setSheet] = useState(false);
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
      {held ? (
        <>
          <span className="portfolio-label">Your money in {fund.name}</span>
          <span className="hero-value num">{usd(held.value)}</span>
          <span className={`hero-sub change-${toneOf(held.cost ? held.value / held.cost - 1 : 0)}`}>
            {signedUsd(Math.abs(held.value - held.cost) < 0.005 ? 0 : held.value - held.cost)} on {usd(held.cost)}
          </span>
          <ul className="list settings">
            <li>
              <button className="row" onClick={() => setSheet(true)}>
                <span className="row-main">
                  <span className="row-title">Change amount</span>
                </span>
                <span className="chev">›</span>
              </button>
            </li>
          </ul>
        </>
      ) : (
        <div className="quote">
          <span className="quote-price">{pct(fund.change)}</span>
          <span className="row-sub">since it started trading live on paper</span>
        </div>
      )}
      <Record fund={fund} />
      {!held && (
        <div className="dock">
          <button className="cta cta-up" disabled={!me} onClick={() => setSheet(true)}>
            Hire
          </button>
        </div>
      )}
      {sheet && me && (
        <BackSheet
          fund={id}
          name={fund.name}
          available={me.balance}
          held={held?.value ?? 0}
          onClose={() => setSheet(false)}
          onDone={(next) => {
            setMe(next);
            setSheet(false);
          }}
        />
      )}
    </>
  );
}
