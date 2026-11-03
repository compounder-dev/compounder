import { useState } from 'react';
import { useLive } from '../api.ts';
import { useEvent } from '../events.ts';
import { Banner, Button, Number, Row, Segmented, Sparkline } from '../parts/index.ts';
import { liveAddress, useTitle } from '../route.ts';
import { kindWord, stateWord } from '../live.ts';
import { changeTone } from '../strategy.ts';

type Filter = 'all' | 'velocity' | 'compounding';

const FILTERS: { value: Filter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'velocity', label: 'Velocity' },
  { value: 'compounding', label: 'Compounding' },
];

/** Every fund trading live, best since it opened first. */
export function Live() {
  useTitle('Live · Compounder');
  useEvent('live_seen', 'live');
  const [funds, again] = useLive();
  const [filter, setFilter] = useState<Filter>('all');
  if (funds.state === 'loading') {
    return (
      <>
        <h1 className="page-title">Live</h1>
        <div className="live-waiting" aria-busy="true">
          <span className="skeleton skeleton-chart" aria-hidden="true" />
        </div>
      </>
    );
  }
  if (funds.state !== 'ready') {
    return (
      <>
        <h1 className="page-title">Live</h1>
        <div className="region-notice">
          <Banner variant="error" action={<Button onClick={again}>Try again</Button>}>
            We could not load the funds.
          </Banner>
        </div>
      </>
    );
  }
  const shown = funds.data.filter((fund) => filter === 'all' || fund.speed === filter);
  return (
    <>
      <h1 className="page-title">Live</h1>
      <div className="live-filter">
        <Segmented label="Which funds" name="live-filter" items={FILTERS} value={filter} onChange={setFilter} />
      </div>
      {shown.length === 0 ? (
        <p className="live-empty">No funds running yet.</p>
      ) : (
        <ul className="rows live-rows" role="list">
          {shown.map((fund, place) => (
            <Row
              key={fund.id}
              variant="fund"
              href={liveAddress(fund.id)}
              leading={<span className="live-rank number">{place + 1}</span>}
              title={fund.name}
              detail={`${kindWord(fund)} · ${stateWord(fund)}`}
              trailing={
                <>
                  <Sparkline points={fund.curve} tone={changeTone(fund.change)} />
                  <span className={`live-pill live-pill-${changeTone(fund.change)}`}>
                    <Number kind="percent" value={fund.change} sign />
                  </span>
                </>
              }
            />
          ))}
        </ul>
      )}
    </>
  );
}
