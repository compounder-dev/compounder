import { useMemo, useState } from 'react';
import { useStarterTest, type TestSeries } from '../api.ts';
import { formatPercent } from '../format.ts';
import { Banner, Button, LineChart, Segmented, type ChartLine } from '../parts/index.ts';
import { byYear, changes, startOf } from '../parts/chart.ts';

const SUBJECT = 'This fund';
/** What a person calls the benchmark. One the list does not know goes by its ticker. */
const BENCHMARK_NAMES: Record<string, string> = { SPY: 'S&P 500' };

const PERIODS = [
  { value: '1', label: '1Y' },
  { value: '3', label: '3Y' },
  { value: '5', label: '5Y' },
  { value: 'max', label: 'Max' },
];

const change = (value: number) => formatPercent(value, { sign: true }) ?? '';

function Ready({ test }: { test: TestSeries }) {
  const [period, setPeriod] = useState('max');
  const beside = test.benchmark ? (BENCHMARK_NAMES[test.benchmark.symbol] ?? test.benchmark.symbol) : null;
  // A period is offered when the test reaches further back than it does.
  const periods = useMemo(() => PERIODS.filter((each) => each.value === 'max' || startOf(test.days, Number(each.value)) > 0), [test.days]);
  const shown = useMemo(() => {
    const from = period === 'max' ? 0 : startOf(test.days, Number(period));
    const { subject, comparison } = changes(test.values, test.benchmark?.values ?? [], from);
    const lines: ChartLine[] = [{ name: SUBJECT, values: subject, kind: 'subject' }];
    if (beside !== null) lines.push({ name: beside, values: comparison, kind: 'comparison' });
    return { days: test.days.slice(from), lines };
  }, [test, period, beside]);
  const years = useMemo(() => {
    const others = test.benchmark ? byYear(test.days, test.benchmark.values) : [];
    return byYear(test.days, test.values).map((row, index) => ({ ...row, beside: others[index]?.change ?? null }));
  }, [test]);

  return (
    <div className="test-chart">
      <Segmented label="Period" name="test-period" items={periods} value={period} onChange={setPeriod} />
      <LineChart label="The tested value of this fund over time. Use the arrow keys to read it day by day." days={shown.days} lines={shown.lines} format={change} hold={0} />
      <details className="chart-table">
        <summary>Year by year</summary>
        <table className="table">
          <thead>
            <tr>
              <th scope="col">Year</th>
              <th scope="col">{SUBJECT}</th>
              {beside !== null && <th scope="col">{beside}</th>}
            </tr>
          </thead>
          <tbody>
            {years.map((row) => (
              <tr key={row.year}>
                <th scope="row">{row.year}</th>
                <td>{row.change === null ? '–' : change(row.change)}</td>
                {beside !== null && <td>{row.beside === null ? '–' : change(row.beside)}</td>}
              </tr>
            ))}
          </tbody>
        </table>
        <p className="table-note">The first and the last year are part years.</p>
      </details>
    </div>
  );
}

/** A fund's test as a chart, fetched after the page: the figures above it arrived with the page. */
export function TestChart({ id }: { id: string }) {
  const [test, again] = useStarterTest(id);
  if (test.state === 'ready') return <Ready test={test.data} />;
  if (test.state === 'loading') {
    return (
      <div className="test-chart">
        <span className="skeleton skeleton-chart" aria-hidden="true" />
      </div>
    );
  }
  return (
    <div className="test-chart">
      <Banner variant="error" action={<Button onClick={again}>Try again</Button>}>
        We could not draw this test just now.
      </Banner>
    </div>
  );
}
