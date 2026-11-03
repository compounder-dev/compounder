import { useState } from 'react';
import { useMarkets, useMe, useOnchain, signIn, type Onchain } from './api.ts';
import { useMids } from './feed.ts';
import { price, signedUsd, usd } from './format.ts';
import { Link, Logo, managerColour, Money, toneOf } from './parts.tsx';
import { fundPath, go } from './route.ts';
import { shareCard } from './share.ts';
import { Ticker } from './Ticker.tsx';
import { ValueChart } from './ValueChart.tsx';
import { explorer, shortAddress, symbolOf, useVenueAccount, type Period } from './venue.ts';
import { You } from './You.tsx';

const PERIODS: { key: Period; label: string; words: string }[] = [
  { key: 'day', label: '1D', words: 'today' },
  { key: 'week', label: '1W', words: 'this week' },
  { key: 'month', label: '1M', words: 'this month' },
  { key: 'allTime', label: 'All', words: 'all time' },
];

const ago = (t: number) => {
  const s = Math.max(1, Math.round((Date.now() - t) / 1000));
  return s < 60 ? `${s}s` : s < 3600 ? `${Math.round(s / 60)}m` : s < 86_400 ? `${Math.round(s / 3600)}h` : `${Math.round(s / 86_400)}d`;
};

function Address({ address, testnet }: { address: string; testnet: boolean }) {
  const [copied, setCopied] = useState(false);
  return (
    <span className="address">
      <button
        className="address-chip num"
        onClick={() =>
          navigator.clipboard?.writeText(address).then(() => {
            setCopied(true);
            window.setTimeout(() => setCopied(false), 1200);
          })
        }
      >
        {copied ? 'Copied' : shortAddress(address)}
      </button>
      <a className="address-chip" href={explorer(address, testnet)} target="_blank" rel="noreferrer">
        Explorer ↗
      </a>
    </span>
  );
}

function OnchainFund({ fund }: { fund: Onchain & { address: string } }) {
  const account = useVenueAccount(fund.address, fund.testnet);
  // Live prices are the main network's; a test account uses its own venue's numbers.
  const live = useMids();
  const mids = fund.testnet ? new Map<string, number>() : live;
  const [me] = useMe();
  const [data] = useMarkets();
  const [period, setPeriod] = useState<Period>('day');
  const history = account?.history[period];
  const value = account?.history.day.value.at(-1)?.[1] ?? 0;
  const pnl = history?.pnl.at(-1)?.[1] ?? 0;
  const base = value - pnl;
  const names = new Map((data?.funds ?? []).map((f) => [f.id, f.name]));
  const hasFund = me && me.fund;
  const copy = () => (me === false ? signIn('/you') : go('/you'));
  return (
    <>
      <header className="topbar">
        <span className="wordmark">compounder</span>
        <span className="topbar-right">
          <span className="pill pill-live">{fund.testnet ? 'Testnet' : 'Onchain'}</span>
        </span>
      </header>
      <span className="portfolio-label">{fund.name}</span>
      <Ticker className={`hero-value num${value >= 1e6 ? ' hero-long' : ''}`} value={value} text={account ? <Money value={value} /> : '–'} />
      <span className="hero-sub">
        <span className={`change-${toneOf(base ? pnl / base : 0)}`}>
          {signedUsd(Math.abs(pnl) < 0.005 ? 0 : pnl)} {base > 0 ? `(${((pnl / base) * 100).toFixed(2)}%)` : ''}
        </span>
        <span className="muted"> {PERIODS.find((p) => p.key === period)!.words}</span>
      </span>
      <Address address={fund.address} testnet={fund.testnet} />
      <ValueChart points={history?.value ?? []} start={history?.value[0]?.[1] ?? value} height={180} />
      <nav className="intervals" aria-label="Chart range">
        {PERIODS.map((p) => (
          <button key={p.key} className={`interval${p.key === period ? ' interval-on' : ''}`} onClick={() => setPeriod(p.key)}>
            {p.label}
          </button>
        ))}
      </nav>

      {fund.mix.length > 0 && (
        <>
          <div className="head">
            <h2>Strategies</h2>
            <Link to="/strategies">All</Link>
          </div>
          <span className="mixbar mixbar-big" aria-hidden="true">
            {fund.mix.map((m) => (
              <i key={m.id} style={{ width: `${m.share * 100}%`, background: managerColour(m.id) }} />
            ))}
          </span>
          <span className="legend">
            {fund.mix.map((m) => (
              <Link key={m.id} to={fundPath(m.id)}>
                <i style={{ background: managerColour(m.id) }} />
                {names.get(m.id) ?? m.id} {Math.round(m.share * 100)}%
              </Link>
            ))}
          </span>
        </>
      )}

      <div className="head">
        <h2>Positions</h2>
        <span className="row-sub">{account ? account.positions.length : ''}</span>
      </div>
      {account && account.positions.length === 0 && <p className="empty">All in cash right now.</p>}
      <ul className="list group">
        {account?.positions.map((p) => {
          // The venue's own numbers, moved with the live price between reads.
          const mid = mids.get(p.coin);
          const upnl = mid ? p.size * (mid - p.entry) : p.pnl;
          const value = mid ? Math.abs(p.size * mid) : p.value;
          const symbol = symbolOf(p.coin);
          return (
            <li key={p.coin}>
              <Link to={`/markets/${encodeURIComponent(p.coin)}`} className="row">
                <Logo symbol={symbol} kind="stocks" />
                <span className="row-main">
                  <span className="row-title">{symbol}</span>
                  <span className="row-sub">
                    <span className={p.size > 0 ? 'side side-up' : 'side side-down'}>
                      {p.size > 0 ? 'LONG' : 'SHORT'} {p.leverage}x
                    </span>{' '}
                    · {usd(value)} · in at {price(p.entry)}
                  </span>
                </span>
                <span className="row-end">
                  <span className={`row-price change-${toneOf(upnl)}`}>{signedUsd(upnl)}</span>
                  <span className="row-sub num">{p.margin > 0 ? `${((upnl / p.margin) * 100).toFixed(1)}%` : ''}</span>
                </span>
              </Link>
            </li>
          );
        })}
      </ul>

      <div className="head">
        <h2>Trades</h2>
        <span className="pill pill-live">Live</span>
      </div>
      {account && account.fills.length === 0 && <p className="empty">No trades yet.</p>}
      <ul className="tape">
        {account?.fills.slice(0, 20).map((f) => (
          <li key={`${f.hash}-${f.time}-${f.coin}`}>
            <span className={f.buy ? 'side-up' : 'side-down'}>{f.buy ? 'BUY' : 'SELL'}</span>
            <b>{symbolOf(f.coin)}</b>
            <span className="num">{usd(f.size * f.price)}</span>
            <span className="num muted">@ {price(f.price)}</span>
            <span className={`num tape-pnl change-${toneOf(f.pnl)}`}>{f.pnl ? signedUsd(f.pnl) : ''}</span>
            <span className="num muted">{ago(f.time)}</span>
          </li>
        ))}
      </ul>

      <p className="proof">
        Every number here is read live from the chain. Check it yourself on the explorer.
      </p>

      <div className="dock dock-tabs">
        <div className="sides">
          <button className="cta" onClick={() => shareCard({ title: fund.name, big: signedUsd(pnl), tone: toneOf(pnl), sub: `${usd(value)} onchain · ${shortAddress(fund.address)}` })}>
            Share
          </button>
          <button className="cta cta-up" onClick={copy}>
            {hasFund ? 'Your copy' : 'Copy this fund'}
          </button>
        </div>
      </div>
    </>
  );
}

/** The home page: the onchain fund where one is linked, otherwise your own. */
export function Home() {
  const fund = useOnchain();
  if (!fund) return <div className="loading" aria-busy="true" />;
  if (!fund.address) return <You />;
  return <OnchainFund fund={{ ...fund, address: fund.address }} />;
}
