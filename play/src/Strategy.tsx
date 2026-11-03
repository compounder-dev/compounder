import { useState } from 'react';
import { signIn, useMarkets, useMe, type Fund as Strategy } from './api.ts';
import { pct, signedUsd, usd } from './format.ts';
import { Change, Link, ManagerMark, toneOf, TopBar } from './parts.tsx';
import { fundPath } from './route.ts';
import { CopySheet } from './sheets.tsx';
import { ValueChart } from './ValueChart.tsx';

const kind = (f: { model?: string; speed: string }) => (f.model ? 'AI' : f.speed === 'velocity' ? 'Intraday' : 'Long-term');
const daysLive = (f: { curve: [number, number][] }) => Math.max(1, Math.ceil((Date.now() - (f.curve[0]?.[0] ?? Date.now())) / 86_400_000));
const day = (t: number) => new Date(t).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' });

/** Strategies that do not take money yet: paused, or no edge after costs in their tests. */
const PAUSED = new Set(['agent-deepseek', 'daily-reversal', 'chip-rotation']);

/** A strategy's state in a few words, read from what it last decided. */
export function stateOf(m: Strategy): string {
  if (PAUSED.has(m.id) || m.note.startsWith('Paused')) return 'Paused';
  if (m.note.startsWith('Out until')) return 'Waits for 10:00 NY';
  if (m.note.startsWith('Holding the morning')) return 'In until 16:00 NY';
  if (m.holdings.length === 0) return 'In cash';
  return m.holdings.length === 1 ? 'Holding 1 market' : `Holding ${m.holdings.length} markets`;
}

/** Every strategy, best live return first. */
export function Strategies() {
  const [data] = useMarkets();
  const [me] = useMe();
  const copying = new Set(me ? me.backed.map((b) => b.fund) : []);
  const ranked = [...(data?.funds ?? [])].sort((a, b) => Number(PAUSED.has(a.id)) - Number(PAUSED.has(b.id)) || b.change - a.change);
  return (
    <>
      <header className="topbar">
        <span className="wordmark">Strategies</span>
        <span className="topbar-right">{ranked[0] && <span className="pill pill-live">Day {daysLive(ranked[0])} live</span>}</span>
      </header>
      {!data && <div className="loading" aria-busy="true" />}
      <ul className="list group">
        {ranked.map((s, i) => (
          <li key={s.id}>
            <Link to={fundPath(s.id)} className="row">
              <span className="rank num">{i + 1}</span>
              <ManagerMark id={s.id} name={s.name} />
              <span className="row-main">
                <span className="row-title">
                  {s.name}
                  {copying.has(s.id) && <span className="tag tag-on">Copying</span>}
                </span>
                <span className="row-sub">
                  {kind(s)} · {stateOf(s)}
                </span>
              </span>
              <span className="row-end">
                <Change value={s.change} />
                <span className="row-sub num">{s.fills} trades</span>
              </span>
            </Link>
          </li>
        ))}
      </ul>
    </>
  );
}

export function StrategyPage({ id }: { id: string }) {
  const [data] = useMarkets();
  const [me, setMe] = useMe();
  const [sheet, setSheet] = useState(false);
  const s = data?.funds.find((f) => f.id === id);
  if (!s) return <TopBar backTo="/strategies" />;
  const held = me ? me.backed.find((b) => b.fund === id) : undefined;
  return (
    <>
      <TopBar
        backTo="/strategies"
        title={
          <>
            <b>{s.name}</b>
            <span>
              {kind(s)} · {stateOf(s)}
            </span>
          </>
        }
      />
      <div className="quote">
        <span className={`quote-price change-${toneOf(s.change)}`}>{pct(s.change)}</span>
        <span className="row-sub">
          {daysLive(s)} {daysLive(s) === 1 ? 'day' : 'days'} live · {s.fills} trades · worst drop {pct(-s.worst_drop)}
        </span>
      </div>
      <ValueChart points={s.curve} start={s.capital} height={180} />
      {held && (
        <div className="held">
          <span className="row-sub">You copy with</span>
          <b className="num">{usd(held.value)}</b>
          <span className={`change change-${toneOf(held.value - held.cost)}`}>{signedUsd(Math.abs(held.value - held.cost) < 0.005 ? 0 : held.value - held.cost)}</span>
        </div>
      )}
      <div className="head">
        <h2>Now</h2>
      </div>
      <p className="about">{s.note || 'Waiting for its first trade.'}</p>
      {s.holdings.length > 0 && (
        <ul className="list">
          {s.holdings.map((h) => (
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
        <h2>Rules</h2>
      </div>
      <p className="about">{s.rules}</p>
      <p className="footnote">
        {s.verified ? `${s.entries} entries on a tamper-proof record.` : 'Its record does not check out.'}
        {s.test ? ` Tested ${day(s.test.start)} to ${day(s.test.end)}: ${pct(s.test.annual_return)} a year. Tests are not live results.` : ''}
      </p>
      <div className="dock">
        <button className="cta cta-up" disabled={me === null || PAUSED.has(id)} onClick={() => (me === false ? signIn() : setSheet(true))}>
          {PAUSED.has(id) ? 'Paused' : held ? 'Change amount' : 'Copy'}
        </button>
      </div>
      {sheet && me && (
        <CopySheet
          fund={id}
          name={s.name}
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
