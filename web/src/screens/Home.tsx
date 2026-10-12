import { useStarters, type Starter } from '../api.ts';
import { useEvent } from '../events.ts';
import { Banner, Button, Card, CardSkeleton, CardTitle, EmptyState } from '../parts/index.ts';
import { starterAddress, useTitle } from '../route.ts';
import { fundColour, holdsLine } from '../starter.ts';

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
      <h1 className="page-title home-title">Choose your fund</h1>
      <p className="page-lead">Start from a fund that is already built. Open one to see what it holds and when it trades.</p>
      <Starters />
    </>
  );
}
