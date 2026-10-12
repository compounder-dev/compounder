import { useStarters, type Starter, type TestFigures } from '../api.ts';
import { useEvent } from '../events.ts';
import { Banner, Button, Card, CardSkeleton, CardTitle, EmptyState, Link, Number } from '../parts/index.ts';
import { PRICING, starterAddress, useTitle } from '../route.ts';
import { asFall, changeTone, fundColour, holdsLine, yearOf } from '../starter.ts';

/** The three figures of a test that a card can carry. Each says what it is, and the first says it was tested. */
function Figures({ test }: { test: TestFigures }) {
  return (
    <dl className="card-figures">
      <div>
        <dt>Per year, tested</dt>
        <dd>
          <Number kind="percent" value={test.per_year} tone={changeTone(test.per_year)} sign />
        </dd>
      </div>
      <div>
        <dt>Worst drop</dt>
        <dd>
          <Number kind="percent" value={asFall(test.worst_drop)} />
        </dd>
      </div>
      <div>
        <dt>Tested since</dt>
        <dd>
          <span className="number">{yearOf(test.first_day)}</span>
        </dd>
      </div>
    </dl>
  );
}

function StarterCard({ starter }: { starter: Starter }) {
  return (
    <Card variant="strategy" as="li" fund={fundColour(starter.accent)}>
      <CardTitle href={starterAddress(starter.id)} dot>
        {starter.name}
      </CardTitle>
      <p className="card-summary">{starter.summary}</p>
      <dl className="card-facts">
        <div>
          <dt>Holds</dt>
          <dd>{holdsLine(starter.holdings)}</dd>
        </div>
      </dl>
      {starter.test && <Figures test={starter.test} />}
    </Card>
  );
}

/** The starter list in its four states: on its way, none, failed, here. */
function Starters() {
  const [starters, again] = useStarters();
  if (starters.state === 'loading') {
    return (
      <ul className="cards" role="list" aria-busy="true" aria-label="Loading funds">
        {[0, 1, 2].map((n) => (
          <CardSkeleton key={n} />
        ))}
      </ul>
    );
  }
  if (starters.state !== 'ready') {
    return (
      <div className="region-notice">
        <Banner variant="error" action={<Button onClick={again}>Try again</Button>}>
          We could not load the funds.
        </Banner>
      </div>
    );
  }
  if (starters.data.length === 0) {
    return (
      <div className="region-notice">
        <EmptyState action={<Button onClick={again}>Look again</Button>}>There are no funds to choose from yet.</EmptyState>
      </div>
    );
  }
  return (
    <ul className="cards" role="list">
      {starters.data.map((starter) => (
        <StarterCard key={starter.id} starter={starter} />
      ))}
    </ul>
  );
}

export function Home() {
  useTitle('Compounder');
  useEvent('home_seen', 'home');
  return (
    <>
      <h1 className="page-title home-title">Build your own wealth fund.</h1>
      <p className="page-lead">Pick a fund that is already built and see how it would have done over years of real prices.</p>
      <p className="price-line">
        Free to build and test. Your first fund is free to run. More funds are $20 a month. <Link href={PRICING}>See pricing</Link>
      </p>
      <h2 className="section-title cards-title">Start from one of these</h2>
      <Starters />
    </>
  );
}
