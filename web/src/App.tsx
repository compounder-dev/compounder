import { useEffect, useState, type CSSProperties, type MouseEvent } from 'react';
import { ApiError, chainName, displayName, fetchStarters, hue, percent, ticker, type Holding, type Starter } from './api.ts';

type Load = { state: 'loading' } | { state: 'failed'; message: string } | { state: 'ready'; starters: Starter[] };

/** The portfolio id in the address bar, from `#/portfolio/<id>`. */
function useSelectedId(): string | null {
  const read = () => window.location.hash.match(/^#\/portfolio\/([\w-]+)$/)?.[1] ?? null;
  const [id, setId] = useState(read);
  useEffect(() => {
    const onChange = () => setId(read());
    window.addEventListener('hashchange', onChange);
    return () => window.removeEventListener('hashchange', onChange);
  }, []);
  return id;
}

function useStarters(): [Load, () => void] {
  const [load, setLoad] = useState<Load>({ state: 'loading' });
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    setLoad({ state: 'loading' });
    fetchStarters(controller.signal)
      .then((starters) => setLoad({ state: 'ready', starters }))
      .catch((error: unknown) => {
        if (controller.signal.aborted) return;
        setLoad({ state: 'failed', message: error instanceof ApiError ? error.message : 'Something went wrong loading portfolios.' });
      });
    return () => controller.abort();
  }, [attempt]);
  return [load, () => setAttempt((n) => n + 1)];
}

function Mark() {
  return (
    <svg className="mark" viewBox="0 0 32 32" aria-hidden="true">
      <path d="M5 25c8 0 12.5-2.4 15.7-7.2S25 9 26 5" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
      <circle cx="26" cy="5" r="3" fill="currentColor" />
    </svg>
  );
}

const STEPS = ['Choose portfolio', 'Deposit', 'Compound'];

function Header() {
  return (
    <header className="topbar">
      <a className="wordmark" href="#/" aria-label="Compounder, home">
        <span>Compounder</span>
        <Mark />
      </a>
      <ol className="steps" aria-label="Getting started">
        {STEPS.map((label, index) => (
          <li key={label} className="step" aria-current={index === 0 ? 'step' : undefined}>
            <span className="step-number">{index + 1}</span>
            <span className="step-label">{label}</span>
          </li>
        ))}
      </ol>
      <span className="topbar-end" />
    </header>
  );
}

function sphereStyle(label: string): CSSProperties {
  return { '--hue': hue(label) } as CSSProperties;
}

/** The card's picture: the portfolio's name over a row of its tickers. */
function Artwork({ starter }: { starter: Starter }) {
  const shown = starter.holdings.slice(0, 7);
  return (
    <div className="art" aria-hidden="true">
      <p className="art-title">{starter.name}</p>
      <div className="spheres">
        {shown.map((holding) => (
          <span key={holding.asset_id} className="sphere" style={sphereStyle(ticker(holding))}>
            {ticker(holding)}
          </span>
        ))}
      </div>
    </div>
  );
}

function HoldingRow({ holding }: { holding: Holding }) {
  return (
    <li className="row">
      <span className="dot" style={sphereStyle(ticker(holding))}>
        {ticker(holding).slice(0, 1)}
      </span>
      <span className="row-name">{displayName(holding)}</span>
      <span className="row-weight">{percent(holding.weight)}</span>
    </li>
  );
}

function StarterCard({ starter }: { starter: Starter }) {
  const href = `#/portfolio/${starter.id}`;
  // The whole card opens the portfolio; the heading link is what a keyboard and a screen reader use.
  const open = (event: MouseEvent) => {
    if ((event.target as HTMLElement).closest('a')) return;
    if (window.getSelection()?.toString()) return;
    window.location.hash = href;
  };
  return (
    <article className={`card accent-${starter.accent}`} onClick={open}>
      <Artwork starter={starter} />
      <h2 className="card-title">
        <a href={href}>{starter.name}</a>
      </h2>
      <p className="card-summary">{starter.summary}</p>
      <div className="table-head">
        <span>Tokens</span>
        <span>Allocation</span>
      </div>
      <ul className="rows" tabIndex={0} aria-label={`Holdings of ${starter.name}`}>
        {starter.holdings.map((holding) => (
          <HoldingRow key={holding.asset_id} holding={holding} />
        ))}
      </ul>
    </article>
  );
}

function Choose({ load, retry }: { load: Load; retry: () => void }) {
  return (
    <main className="page">
      <h1 className="headline">Choose your portfolio</h1>
      {load.state === 'loading' && (
        <div className="cards" aria-busy="true" aria-label="Loading portfolios">
          {[0, 1, 2].map((n) => (
            <div key={n} className="card skeleton" />
          ))}
        </div>
      )}
      {load.state === 'failed' && (
        <div className="notice" role="alert">
          <p>{load.message}</p>
          <button className="button" onClick={retry}>
            Try again
          </button>
        </div>
      )}
      {load.state === 'ready' && (
        <div className="cards">
          {load.starters.map((starter) => (
            <StarterCard key={starter.id} starter={starter} />
          ))}
        </div>
      )}
    </main>
  );
}

function Detail({ starter }: { starter: Starter }) {
  const chain = chainName(starter.holdings);
  const largest = Math.max(...starter.holdings.map((holding) => holding.weight));
  const settings = starter.strategy.tradingSettings;
  const rebalances = settings.type === 'threshold' ? `When a holding drifts ${settings.triggerPercentage}% from its target` : `On a ${settings.frequency} schedule`;
  return (
    <main className={`page detail accent-${starter.accent}`}>
      <a className="back" href="#/" aria-label="Back to all portfolios">
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M19 12H5m6-6-6 6 6 6" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </a>
      <div className="detail-grid">
        <section className="detail-intro">
          <h1 className="headline">{starter.name}</h1>
          <p className="lede">{starter.summary}</p>
          <dl className="facts">
            <div>
              <dt>Holdings</dt>
              <dd>{starter.holdings.length}</dd>
            </div>
            <div>
              <dt>Rebalances</dt>
              <dd>{rebalances}</dd>
            </div>
            {chain && (
              <div>
                <dt>Network</dt>
                <dd>{chain}</dd>
              </div>
            )}
          </dl>
          <p className="status">Deposits are not connected yet.</p>
        </section>
        <section className="panel" aria-label="Target allocation">
          <div className="table-head">
            <span>Tokens</span>
            <span>Allocation</span>
          </div>
          <ul className="rows rows-open">
            {starter.holdings.map((holding) => (
              <li key={holding.asset_id} className="row row-tall">
                <span className="dot" style={sphereStyle(ticker(holding))}>
                  {ticker(holding).slice(0, 1)}
                </span>
                <span className="row-name">
                  {displayName(holding)}
                  <small>{holding.symbol}</small>
                </span>
                <span className="bar" aria-hidden="true">
                  <span style={{ width: `${(holding.weight / largest) * 100}%` }} />
                </span>
                <span className="row-weight">{percent(holding.weight)}</span>
              </li>
            ))}
          </ul>
        </section>
      </div>
    </main>
  );
}

export function App() {
  const selectedId = useSelectedId();
  const [load, retry] = useStarters();
  const selected = load.state === 'ready' ? load.starters.find((starter) => starter.id === selectedId) : undefined;

  useEffect(() => {
    document.title = selected ? `${selected.name} · Compounder` : 'Compounder';
    window.scrollTo(0, 0);
  }, [selected]);

  return (
    <>
      <Header />
      {selected ? <Detail starter={selected} /> : <Choose load={load} retry={retry} />}
    </>
  );
}
