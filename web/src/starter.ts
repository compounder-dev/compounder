// The words and figures drawn from one starter. The server writes the same
// ones into the first response, and the files in contract/ hold both to them.

import { FUND_COLOURS, type FundColour, type Holding, type Starter } from './api.ts';
import { formatQuantity } from './format.ts';

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

/** When a fund that follows the strategy trades, in one line. */
export function tradesLine(settings: Starter['strategy']['tradingSettings']): string {
  if (settings.type === 'threshold') return `When a holding drifts ${formatQuantity(settings.triggerPercentage) ?? ''}% from its target`;
  return `On a ${settings.frequency} schedule`;
}

/** The identity colour a starter was given, or the first one when the server sends a name we do not know. */
export function fundColour(accent: string): FundColour {
  return FUND_COLOURS.find((colour) => colour === accent) ?? FUND_COLOURS[0];
}

/** A holding's bar beside the largest holding's, as a whole number from 0 to 100. */
export function barShare(weight: number, largest: number): number {
  return largest > 0 ? Math.floor((weight / largest) * 100 + 0.5) : 0;
}

/** Whether the engine's weights, in parts per billion, are the allocation the server sent. */
export function sameAllocation(weights: Record<string, number>, holdings: Holding[]): boolean {
  return (
    Object.keys(weights).length === holdings.length &&
    holdings.every((holding) => Math.abs((weights[holding.asset_id] ?? -1) / 1e9 - holding.weight) < 1e-9)
  );
}
