// The words and figures drawn from one strategy. The server writes the same
// ones into the first response, and the files in contract/ hold both to them.

import { FUND_COLOURS, type FundColour, type Holding, type Strategy, type TestFigures } from './api.ts';
import { formatPercent } from './format.ts';

/** The ticker a person recognises: "NVDAon" becomes "NVDA". */
export function ticker(holding: Holding): string {
  return (holding.symbol ?? holding.asset_id.slice(0, 6)).replace(/on$/, '');
}

/** The company or fund name, without the issuer's suffix. */
export function displayName(holding: Holding): string {
  return (holding.name ?? holding.symbol ?? holding.asset_id).replace(/\s*\([^)]*\)\s*$/, '');
}

/** The one letter that stands for an asset until it has a mark of its own. */
export function markLetter(holding: Holding): string {
  return Array.from(ticker(holding))[0] ?? '';
}

const HOLDS_SHOWN = 4;

/** The first few tickers of a fund in one line: "AAPL, MSFT, AMZN, NVDA and 3 more". */
export function holdsLine(holdings: Holding[]): string {
  // One ticker is never left as "and 1 more" when it could just be named.
  const shown = holdings.length <= HOLDS_SHOWN + 1 ? holdings : holdings.slice(0, HOLDS_SHOWN);
  const names = shown.map(ticker).join(', ');
  const rest = holdings.length - shown.length;
  return rest > 0 ? `${names} and ${rest} more` : names;
}

/** The identity colour a strategy was given, or the first one when the server sends a name we do not know. */
export function fundColour(accent: string): FundColour {
  return FUND_COLOURS.find((colour) => colour === accent) ?? FUND_COLOURS[0];
}

/** A holding's bar beside the largest holding's, as a whole number from 0 to 100. */
export function barShare(weight: number, largest: number): number {
  return largest > 0 ? Math.floor((weight / largest) * 100 + 0.5) : 0;
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** A day written "2012-05-18" as "May 2012". Anything else is shown as it stands. */
export function monthYear(day: string): string {
  const parts = /^(\d{4})-(\d{2})-\d{2}$/.exec(day);
  const month = parts && MONTHS[Number(parts[2]) - 1];
  return parts && month ? `${month} ${parts[1]}` : day;
}

/** The year of a day written "2012-05-18". */
export function yearOf(day: string): string {
  return day.slice(0, 4);
}

/** A fall, given as a share of the high it fell from, as the change it is: 0.5 is -0.5. */
export function asFall(drop: number | null): number | null {
  return drop === null ? null : 0 - drop;
}

/** The colour a change is read in. One that prints as zero has none. */
export function changeTone(change: number | null): 'gain' | 'loss' | 'plain' {
  if (change === null || formatPercent(change) === '0%') return 'plain';
  return change > 0 ? 'gain' : 'loss';
}

/** The one line under a test's figures that says what they are and what they are not. */
export function testedLine(test: TestFigures): string {
  return `Tested on past prices from ${monthYear(test.first_day)} to ${monthYear(test.last_day)}, with a cost of ${formatPercent(test.cost) ?? ''} taken from each trade. This fund is not live yet. Past results do not promise future results.`;
}

/** The funds whose name, sentence or holdings have every word of what was typed. Nothing typed keeps them all. */
export function matching(strategies: Strategy[], typed: string, kind: 'all' | Strategy['kind']): Strategy[] {
  const words = typed.toLowerCase().split(/\s+/).filter(Boolean);
  return strategies.filter((strategy) => {
    if (kind !== 'all' && strategy.kind !== kind) return false;
    const text = [strategy.name, strategy.summary, ...strategy.holdings.flatMap((holding) => [ticker(holding), displayName(holding)]), ...(strategy.outline ?? []).flatMap((line) => (line.kind === 'asset' ? [line.text, line.name ?? ''] : []))]
      .join(' ')
      .toLowerCase();
    return words.every((word) => text.includes(word));
  });
}

/** Whether the engine's weights, in parts per billion, are the allocation the server sent. */
export function sameAllocation(weights: Record<string, number>, holdings: Holding[]): boolean {
  return (
    Object.keys(weights).length === holdings.length &&
    holdings.every((holding) => Math.abs((weights[holding.asset_id] ?? -1) / 1e9 - holding.weight) < 1e-9)
  );
}
