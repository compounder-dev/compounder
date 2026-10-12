import { useEffect, useRef, useState, type CSSProperties, type RefObject } from 'react';
import { useStrategy, type OutlineLine, type Strategy, type TestFigures } from '../api.ts';
import { useEvent } from '../events.ts';
import { Banner, Button, ButtonLink, Card, Icon, Link, Mark, Number, Row, RowSkeleton, Skeleton, StatBand, WeightBar, type IconName } from '../parts/index.ts';
import { BUILD, EXPLORE, useTitle } from '../route.ts';
import { asFall, barShare, changeTone, displayName, fundColour, markLetter, sameAllocation, testedLine, ticker } from '../strategy.ts';
import { NotFound } from './System.tsx';
import { TestChart } from './TestChart.tsx';

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
function useAllocationCheck(strategy: Strategy): [Check, RefObject<HTMLParagraphElement | null>] {
  const [check, setCheck] = useState<Check>('idle');
  const line = useRef<HTMLParagraphElement>(null);
  useEffect(() => {
    let current = true;
    setCheck('idle');
    const run = () => {
      setCheck('checking');
      import('../engine.ts')
        .then(({ strategyWeights }) => strategyWeights(strategy.fixed))
        .then((weights) => current && setCheck(sameAllocation(weights, strategy.holdings) ? 'match' : 'mismatch'))
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
  }, [strategy]);
  return [check, line];
}

function CheckLine({ strategy }: { strategy: Strategy }) {
  const [check, line] = useAllocationCheck(strategy);
  return (
    <p ref={line} className={`check check-${check}`} role="status">
      <Icon name={CHECKS[check].icon} />
      {CHECKS[check].words}
    </p>
  );
}

/** What the fund's test over past prices came to: the chart, four figures, and the line that says what they are. */
function Results({ id, test }: { id: string; test: TestFigures }) {
  return (
    <section className="strategy-results" aria-labelledby="results-title">
      <h2 className="section-title" id="results-title">
        How it would have done
      </h2>
      <StatBand
        items={[
          { label: 'Per year', value: <Number kind="percent" value={test.per_year} size="title2" tone={changeTone(test.per_year)} sign /> },
          { label: 'Worst drop', value: <Number kind="percent" value={asFall(test.worst_drop)} size="title2" /> },
          { label: 'Sharpe ratio', value: <Number kind="ratio" value={test.sharpe} size="title2" /> },
          { label: 'S&P 500, per year', value: <Number kind="percent" value={test.benchmark_per_year} size="title2" sign /> },
        ]}
      />
      <TestChart id={id} />
      <p className="honest-line">{testedLine(test)}</p>
    </section>
  );
}

/** The rules in the server's words, each line set in as deep as it sits in the strategy. */
function Rules({ lines }: { lines: OutlineLine[] }) {
  return (
    <section className="strategy-rules" aria-labelledby="rules-title">
      <h2 className="section-title" id="rules-title">
        The rules
      </h2>
      <ol className="outline" role="list">
        {lines.map((line, place) => (
          <li key={place} className={`outline-line outline-${line.kind}`} style={{ '--depth': line.depth } as CSSProperties}>
            {line.kind === 'asset' ? (
              <>
                <span className="ticker">{line.text}</span>
                {line.name !== undefined && <span className="outline-name">{line.name}</span>}
                {line.share !== undefined && <Number kind="percent" value={line.share} />}
              </>
            ) : (
              line.text
            )}
          </li>
        ))}
      </ol>
    </section>
  );
}

function Ready({ strategy }: { strategy: Strategy }) {
  const largest = Math.max(0, ...strategy.holdings.map((holding) => holding.weight));
  return (
    <div className={`strategy fund-${fundColour(strategy.accent)}`}>
      <header className="strategy-head">
        <h1 className="page-title">
          <span className="fund-dot" aria-hidden="true" />
          {strategy.name}
        </h1>
        <p className="page-lead">{strategy.summary}</p>
      </header>
      {strategy.test && <Results id={strategy.id} test={strategy.test} />}
      {strategy.outline !== undefined && strategy.outline.length > 0 && <Rules lines={strategy.outline} />}
      {strategy.holdings.length > 0 && (
        <section className="strategy-holdings" aria-labelledby="holdings-title">
          <h2 className="section-title" id="holdings-title">
            {strategy.starter ? 'What it holds' : 'What it holds now'}
          </h2>
          <ul className="rows" role="list">
            {strategy.holdings.map((holding) => (
              <Row
                key={holding.asset_id}
                leading={<Mark letter={markLetter(holding)} />}
                title={displayName(holding)}
                detail={holding.symbol === null ? undefined : ticker(holding)}
                trailing={
                  <>
                    <WeightBar percent={barShare(holding.weight, largest)} />
                    <Number kind="percent" value={holding.weight} />
                  </>
                }
              />
            ))}
          </ul>
          {strategy.fixed !== undefined && <CheckLine strategy={strategy} />}
        </section>
      )}
      <Card as="aside" className="strategy-facts" label="Facts">
        <dl className="facts">
          {strategy.holdings.length > 0 && (
            <div>
              <dt>Holdings</dt>
              <dd>
                <Number kind="quantity" value={strategy.holdings.length} />
              </dd>
            </div>
          )}
          <div>
            <dt>Trades</dt>
            <dd>{strategy.trades}</dd>
          </div>
        </dl>
        <ButtonLink href={`${BUILD}?from=${strategy.id}`} icon="pencil" fullWidth>
          Change the rules
        </ButtonLink>
        <p className="strategy-note">Adding money is not open yet.</p>
      </Card>
    </div>
  );
}

/** The page before its fund has arrived, in the shape of the page. */
function Loading() {
  return (
    <div className="strategy" aria-busy="true">
      <header className="strategy-head">
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
      <section className="strategy-holdings">
        <h2 className="section-title">
          <Skeleton width="short" />
        </h2>
        <ul className="rows" role="list">
          {[0, 1, 2, 3, 4].map((n) => (
            <RowSkeleton key={n} />
          ))}
        </ul>
      </section>
      <Card as="aside" className="strategy-facts">
        <div className="facts">
          {[0, 1].map((n) => (
            <div key={n}>
              <Skeleton width="short" />
              <Skeleton width="half" />
            </div>
          ))}
        </div>
        <p className="strategy-note">
          <Skeleton width="long" />
        </p>
      </Card>
    </div>
  );
}

export function StrategyScreen({ id }: { id: string }) {
  const [strategy, again] = useStrategy(id);
  useTitle(strategy.state === 'ready' ? `${strategy.data.name} · Compounder` : 'Compounder');
  useEvent('strategy_opened', 'strategy', id, strategy.state === 'ready');
  if (strategy.state === 'missing') return <NotFound />;
  return (
    <>
      <p className="back-link">
        <Link href={EXPLORE} variant="standalone" icon="arrow-left">
          All funds
        </Link>
      </p>
      {strategy.state === 'loading' && <Loading />}
      {strategy.state === 'failed' && (
        <div className="region-notice">
          <Banner variant="error" action={<Button onClick={again}>Try again</Button>}>
            We could not load this fund.
          </Banner>
        </div>
      )}
      {strategy.state === 'ready' && <Ready strategy={strategy.data} />}
    </>
  );
}
