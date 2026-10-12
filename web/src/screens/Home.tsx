import { useStrategies } from '../api.ts';
import { useEvent } from '../events.ts';
import { Banner, Button, ButtonLink, CardSkeleton, EmptyState, Link } from '../parts/index.ts';
import { BUILD, EXPLORE, PRICING, useTitle } from '../route.ts';
import { StrategyCard } from './StrategyCard.tsx';

/** The strategy list in its four states: on its way, none, failed, here. */
function Strategies() {
  const [strategies, again] = useStrategies();
  if (strategies.state === 'loading') {
    return (
      <ul className="cards" role="list" aria-busy="true" aria-label="Loading funds">
        {[0, 1, 2].map((n) => (
          <CardSkeleton key={n} />
        ))}
      </ul>
    );
  }
  if (strategies.state !== 'ready') {
    return (
      <div className="region-notice">
        <Banner variant="error" action={<Button onClick={again}>Try again</Button>}>
          We could not load the funds.
        </Banner>
      </div>
    );
  }
  // The home page leads with the few funds a person can begin with. The rest are on Explore.
  const starters = strategies.data.filter((strategy) => strategy.starter);
  if (starters.length === 0) {
    return (
      <div className="region-notice">
        <EmptyState action={<Button onClick={again}>Look again</Button>}>There are no funds to choose from yet.</EmptyState>
      </div>
    );
  }
  return (
    <>
      <ul className="cards" role="list">
        {starters.map((strategy) => (
          <StrategyCard key={strategy.id} strategy={strategy} />
        ))}
      </ul>
      <p className="more-link">
        <Link href={EXPLORE} variant="standalone" icon="search">
          Explore all funds
        </Link>
      </p>
    </>
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
      <p className="home-actions">
        <ButtonLink variant="primary" href={BUILD} icon="pencil">
          Build your own
        </ButtonLink>
        <ButtonLink href={EXPLORE}>Explore funds</ButtonLink>
      </p>
      <h2 className="section-title cards-title">Or start from one of these</h2>
      <Strategies />
    </>
  );
}
