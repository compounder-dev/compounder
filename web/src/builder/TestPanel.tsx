import { useMemo } from 'react';
import { formatPercent } from '../format.ts';
import { Card, Icon, LineChart, Mark, Number, Row, StatBand, WeightBar, type ChartLine } from '../parts/index.ts';
import { changes } from '../parts/chart.ts';
import { asFall, barShare, changeTone, monthYear } from '../strategy.ts';
import type { Path } from './draft.ts';
import type { Answer, Testing } from './testing.ts';

const change = (value: number) => formatPercent(value, { sign: true }) ?? '';

/** The benchmark's yearly figure may stand beside the fund's only when both cover the same days. */
const besideYear = (answer: Answer) => (answer.benchmark && answer.benchmark.first_day === answer.first_day && answer.benchmark.last_day === answer.last_day ? answer.benchmark.stats.annual_return : null);

function Figures({ answer }: { answer: Answer }) {
  const shown = useMemo(() => {
    const { subject, comparison } = changes(answer.values, answer.benchmark?.values ?? [], 0);
    const lines: ChartLine[] = [{ name: 'This fund', values: subject, kind: 'subject' }];
    if (answer.benchmark) lines.push({ name: 'S&P 500', values: comparison, kind: 'comparison' });
    return lines;
  }, [answer]);
  return (
    <>
      <StatBand
        items={[
          { label: 'Per year', value: <Number kind="percent" value={answer.stats.annual_return} size="title2" tone={changeTone(answer.stats.annual_return)} sign /> },
          { label: 'Worst drop', value: <Number kind="percent" value={asFall(answer.stats.max_drawdown)} size="title2" /> },
          { label: 'Sharpe ratio', value: <Number kind="ratio" value={answer.stats.sharpe} size="title2" /> },
          { label: 'S&P 500, per year', value: <Number kind="percent" value={besideYear(answer)} size="title2" sign /> },
        ]}
      />
      <LineChart label="The tested value of this fund over time. Use the arrow keys to read it day by day." days={answer.days} lines={shown} format={change} hold={0} />
      <p className="honest-line">{`Tested on past prices from ${monthYear(answer.first_day)} to ${monthYear(answer.last_day)}, with a cost of ${formatPercent(answer.slippage) ?? ''} taken from each trade. Past results do not promise future results.`}</p>
    </>
  );
}

/** What the test held on its last day, largest first. */
function held(answer: Answer) {
  return answer.assets
    .map((asset, place) => ({ symbol: asset.symbol ?? '', name: asset.name, weight: answer.holdings.weights[place] ?? 0 }))
    .filter((holding) => holding.weight > 0)
    .sort((a, b) => b.weight - a.weight || a.symbol.localeCompare(b.symbol));
}

type PanelProps = {
  testing: Testing;
  /** Open the rule a problem is on. */
  onProblem: (path: Path) => void;
};

/** Beside the rules: what their test came to, what the fund would hold today, and what is left to fix. */
export function TestPanel({ testing, onProblem }: PanelProps) {
  const { state, answer, issues, versions } = testing;
  const holdings = answer ? held(answer) : [];
  const largest = Math.max(0, ...holdings.map((holding) => holding.weight));
  return (
    <div className="builder-panel" id="test">
      <Card as="section" className="builder-test" label="Test">
        <h2 className="section-title">Test</h2>
        <p className="sr-only" role="status">
          {state === 'testing' ? 'Testing' : state === 'ready' ? 'Tested' : state === 'problems' ? `${issues.length} to fix` : ''}
        </p>
        {state === 'empty' && <p className="builder-hint">Add what the fund holds to see how it would have done.</p>}
        {state === 'unavailable' && <p className="builder-hint">This server has no prices to test on.</p>}
        {state === 'failed' && <p className="builder-hint">The test could not be run just now. Change anything to try again.</p>}
        {state === 'problems' && <p className="builder-hint">{issues.length === 1 ? 'Fix 1 thing to see the test.' : `Fix ${issues.length} things to see the test.`}</p>}
        {answer && state !== 'empty' && (
          <div className={state === 'ready' ? 'builder-figures' : 'builder-figures builder-stale'}>
            <Figures answer={answer} />
          </div>
        )}
        {!answer && state === 'testing' && <span className="skeleton skeleton-chart" aria-hidden="true" />}
        {versions > 2 && state !== 'empty' && <p className="builder-hint">{`You have tested ${versions} versions. The more you tune to the past, the less the past tells you.`}</p>}
      </Card>
      {issues.length > 0 && (
        <Card as="section" className="builder-fix" label="To fix">
          <h2 className="section-title">To fix</h2>
          <ul className="fix-list" role="list">
            {issues.map((issue, place) => (
              <li key={place}>
                {issue.path ? (
                  <button type="button" className="fix-line" onClick={() => onProblem(issue.path as Path)}>
                    <Icon name="alert" size="sm" />
                    <span>{`${issue.message} ${issue.fix}`}</span>
                  </button>
                ) : (
                  <p className="fix-line">
                    <Icon name="alert" size="sm" />
                    <span>{`${issue.message} ${issue.fix}`}</span>
                  </p>
                )}
              </li>
            ))}
          </ul>
        </Card>
      )}
      {holdings.length > 0 && state !== 'empty' && (
        <section className="builder-holds" aria-labelledby="holds-title">
          <h2 className="section-title" id="holds-title">
            It would hold today
          </h2>
          <ul className="rows" role="list">
            {holdings.map((holding) => (
              <Row
                key={holding.symbol}
                leading={<Mark letter={holding.symbol.slice(0, 1)} />}
                title={holding.name ?? holding.symbol}
                detail={holding.name ? holding.symbol : undefined}
                trailing={
                  <>
                    <WeightBar percent={barShare(holding.weight, largest)} />
                    <Number kind="percent" value={holding.weight} />
                  </>
                }
              />
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

/** The test in one line, for the bar a phone keeps in view while the rules are edited. */
export function testLine(testing: Testing): string {
  const { state, answer, issues } = testing;
  if (state === 'empty') return 'Add what it holds to see its test';
  if (state === 'problems') return issues.length === 1 ? '1 thing to fix' : `${issues.length} things to fix`;
  if (state === 'unavailable' || state === 'failed') return 'No test just now';
  if (!answer) return 'Testing…';
  const year = answer.stats.annual_return === null ? '–' : change(answer.stats.annual_return);
  const drop = answer.stats.max_drawdown === null ? '–' : (formatPercent(0 - answer.stats.max_drawdown) ?? '');
  return `Per year ${year}. Worst drop ${drop}.`;
}
