import { useEffect, useRef, useState, type RefObject } from 'react';
import { useStarter, type Starter } from '../api.ts';
import { useEvent } from '../events.ts';
import { Banner, Button, Card, Icon, Link, Mark, Number, Row, RowSkeleton, Skeleton, WeightBar, type IconName } from '../parts/index.ts';
import { useTitle } from '../route.ts';
import { barShare, displayName, fundColour, markLetter, sameAllocation, tradesLine } from '../starter.ts';
import { NotFound } from './System.tsx';

type Check = 'idle' | 'checking' | 'match' | 'mismatch' | 'unavailable';

const CHECKS: Record<Check, { icon: IconName; words: string }> = {
  idle: { icon: 'clock', words: 'Checking the allocation in your browser…' },
  checking: { icon: 'clock', words: 'Checking the allocation in your browser…' },
  match: { icon: 'check', words: 'Checked in your browser: this allocation is exactly what the strategy says.' },
  mismatch: { icon: 'alert', words: 'Warning: your browser worked out a different allocation from the one the server sent.' },
  unavailable: { icon: 'info', words: 'This browser could not run the check.' },
};

/**
 * Re-run the strategy in this browser and compare with the allocation the server sent.
 * The engine is the heaviest thing the app can load, so it is fetched only when the
 * line that reports on it comes into view.
 */
function useAllocationCheck(starter: Starter): [Check, RefObject<HTMLParagraphElement | null>] {
  const [check, setCheck] = useState<Check>('idle');
  const line = useRef<HTMLParagraphElement>(null);
  useEffect(() => {
    let current = true;
    setCheck('idle');
    const run = () => {
      setCheck('checking');
      import('../engine.ts')
        .then(({ strategyWeights }) => strategyWeights(starter.strategy))
        .then((weights) => current && setCheck(sameAllocation(weights, starter.holdings) ? 'match' : 'mismatch'))
        .catch(() => current && setCheck('unavailable'));
    };
    const seen = new IntersectionObserver((entries) => {
      if (!entries.some((entry) => entry.isIntersecting)) return;
      seen.disconnect();
      run();
    });
    if (line.current) seen.observe(line.current);
    return () => {
      current = false;
      seen.disconnect();
    };
  }, [starter]);
  return [check, line];
}

function CheckLine({ starter }: { starter: Starter }) {
  const [check, line] = useAllocationCheck(starter);
  return (
    <p ref={line} className={`check check-${check}`} role="status">
      <Icon name={CHECKS[check].icon} />
      {CHECKS[check].words}
    </p>
  );
}

function Ready({ starter }: { starter: Starter }) {
  const largest = Math.max(0, ...starter.holdings.map((holding) => holding.weight));
  return (
    <div className={`starter fund-${fundColour(starter.accent)}`}>
      <header className="starter-head">
        <h1 className="page-title">
          <span className="fund-dot" aria-hidden="true" />
          {starter.name}
        </h1>
        <p className="page-lead">{starter.summary}</p>
      </header>
      <Card as="aside" className="starter-facts" label="Facts">
        <dl className="facts">
          <div>
            <dt>Holdings</dt>
            <dd>
              <Number kind="quantity" value={starter.holdings.length} />
            </dd>
          </div>
          <div>
            <dt>Trades</dt>
            <dd>{tradesLine(starter.strategy.tradingSettings)}</dd>
          </div>
        </dl>
        <p className="starter-note">Adding money is not open yet.</p>
      </Card>
      <section className="starter-holdings" aria-labelledby="holdings-title">
        <h2 className="section-title" id="holdings-title">
          What it holds
        </h2>
        <ul className="rows" role="list">
          {starter.holdings.map((holding) => (
            <Row
              key={holding.asset_id}
              leading={<Mark letter={markLetter(holding)} />}
              title={displayName(holding)}
              detail={holding.symbol ?? undefined}
              trailing={
                <>
                  <WeightBar percent={barShare(holding.weight, largest)} />
                  <Number kind="percent" value={holding.weight} />
                </>
              }
            />
          ))}
        </ul>
        <CheckLine starter={starter} />
      </section>
    </div>
  );
}

/** The page before its fund has arrived, in the shape of the page. */
function Loading() {
  return (
    <div className="starter" aria-busy="true">
      <header className="starter-head">
        <p className="sr-only" role="status">
          Loading the fund
        </p>
        <h1 className="page-title">
          <Skeleton width="long" />
        </h1>
        <p className="page-lead">
          <Skeleton />
          <Skeleton width="half" />
        </p>
      </header>
      <Card as="aside" className="starter-facts">
        <div className="facts">
          {[0, 1].map((n) => (
            <div key={n}>
              <Skeleton width="short" />
              <Skeleton width="half" />
            </div>
          ))}
        </div>
        <p className="starter-note">
          <Skeleton width="long" />
        </p>
      </Card>
      <section className="starter-holdings">
        <h2 className="section-title">
          <Skeleton width="short" />
        </h2>
        <ul className="rows" role="list">
          {[0, 1, 2, 3, 4].map((n) => (
            <RowSkeleton key={n} />
          ))}
        </ul>
      </section>
    </div>
  );
}

export function StarterScreen({ id }: { id: string }) {
  const [starter, again] = useStarter(id);
  useTitle(starter.state === 'ready' ? `${starter.data.name} · Compounder` : 'Compounder');
  useEvent('starter_opened', 'starter', id, starter.state === 'ready');
  if (starter.state === 'missing') return <NotFound />;
  return (
    <>
      <p className="back-link">
        <Link href="/" variant="standalone" icon="arrow-left">
          All funds
        </Link>
      </p>
      {starter.state === 'loading' && <Loading />}
      {starter.state === 'failed' && (
        <div className="region-notice">
          <Banner variant="error" action={<Button onClick={again}>Try again</Button>}>
            We could not load this fund.
          </Banner>
        </div>
      )}
      {starter.state === 'ready' && <Ready starter={starter.data} />}
    </>
  );
}
