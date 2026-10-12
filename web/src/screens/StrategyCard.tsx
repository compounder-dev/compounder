import type { Strategy, TestFigures } from '../api.ts';
import { Card, CardTitle, Number } from '../parts/index.ts';
import { strategyAddress } from '../route.ts';
import { asFall, changeTone, fundColour, holdsLine, yearOf } from '../strategy.ts';

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

/** One fund in a list: its name, one sentence, what it holds and what its test came to. */
export function StrategyCard({ strategy }: { strategy: Strategy }) {
  return (
    <Card variant="strategy" as="li" fund={fundColour(strategy.accent)}>
      <CardTitle href={strategyAddress(strategy.id)} dot>
        {strategy.name}
      </CardTitle>
      <p className="card-summary">{strategy.summary}</p>
      {strategy.holdings.length > 0 && (
        <dl className="card-facts">
          <div>
            <dt>Holds</dt>
            <dd>{holdsLine(strategy.holdings)}</dd>
          </div>
        </dl>
      )}
      {strategy.test && <Figures test={strategy.test} />}
    </Card>
  );
}
