import { useState } from 'react';
import { useStrategies, type Strategy } from '../api.ts';
import { useEvent } from '../events.ts';
import { Banner, Button, CardSkeleton, EmptyState, Field, Segmented } from '../parts/index.ts';
import { useTitle } from '../route.ts';
import { matching } from '../strategy.ts';
import { StrategyCard } from './StrategyCard.tsx';

type Kind = 'all' | Strategy['kind'];

const KINDS: { value: Kind; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'basket', label: 'Baskets' },
  { value: 'rules', label: 'Rule based' },
];

/** Every fund there is, to search and to narrow by kind. */
export function Explore() {
  useTitle('Explore · Compounder');
  useEvent('explore_seen', 'explore');
  const [strategies, again] = useStrategies();
  const [typed, setTyped] = useState('');
  const [kind, setKind] = useState<Kind>('all');
  const found = strategies.state === 'ready' ? matching(strategies.data, typed, kind) : [];
  const showAll = () => {
    setTyped('');
    setKind('all');
  };

  return (
    <>
      <h1 className="page-title">Explore</h1>
      <p className="page-lead">Funds to start from. Each one is tested on years of real prices.</p>
      <div className="explore-tools">
        <Field label="Search funds" type="search" icon="search" placeholder="Search funds and holdings" value={typed} onChange={setTyped} />
        <Segmented label="Kind of fund" name="explore-kind" items={KINDS} value={kind} onChange={setKind} />
      </div>
      {strategies.state === 'loading' && (
        <ul className="cards" role="list" aria-busy="true" aria-label="Loading funds">
          {[0, 1, 2, 3, 4, 5].map((n) => (
            <CardSkeleton key={n} />
          ))}
        </ul>
      )}
      {(strategies.state === 'failed' || strategies.state === 'missing') && (
        <div className="region-notice">
          <Banner variant="error" action={<Button onClick={again}>Try again</Button>}>
            We could not load the funds.
          </Banner>
        </div>
      )}
      {strategies.state === 'ready' && (
        <>
          <p className="sr-only" role="status">
            {found.length === 1 ? '1 fund' : `${found.length} funds`}
          </p>
          {found.length === 0 ? (
            <div className="region-notice">
              <EmptyState action={<Button onClick={showAll}>Show all</Button>}>Nothing matches. Try a shorter word.</EmptyState>
            </div>
          ) : (
            <ul className="cards" role="list">
              {found.map((strategy) => (
                <StrategyCard key={strategy.id} strategy={strategy} />
              ))}
            </ul>
          )}
        </>
      )}
    </>
  );
}
