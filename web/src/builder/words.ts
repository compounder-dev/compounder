// The words on a rule: what each block says on its pill, and what each choice in
// an editor is called. The server says the same things about a finished fund.

import { formatPercent, formatQuantity } from '../format.ts';
import { symbolOf, type Block, type Comparator, type Comparison, type Condition, type Frequency, type Indicator, type Measure, type Reading, type Rebalance, type WeightBlock } from './draft.ts';

/** What each measure is called in a list of choices, and one line on what it is. */
export const MEASURES: Record<Indicator, { name: string; line: string; windowed: boolean; scale: 'dollars' | 'percent' }> = {
  price: { name: 'Price', line: 'The latest closing price.', windowed: false, scale: 'dollars' },
  average_price: { name: 'Average price', line: 'The average closing price over the days.', windowed: true, scale: 'dollars' },
  exponential_average_price: { name: 'Weighted average price', line: 'An average price that leans on the latest days.', windowed: true, scale: 'dollars' },
  price_deviation: { name: 'Spread of prices', line: 'How far prices have strayed from their average.', windowed: true, scale: 'dollars' },
  cumulative_return: { name: 'Return', line: 'The gain or loss over the days.', windowed: true, scale: 'percent' },
  average_return: { name: 'Average daily return', line: 'The average gain or loss of one day.', windowed: true, scale: 'percent' },
  return_deviation: { name: 'Volatility', line: 'How much the daily gains and losses swing.', windowed: true, scale: 'percent' },
  max_drawdown: { name: 'Worst drop', line: 'The largest fall from a high inside the days.', windowed: true, scale: 'percent' },
  relative_strength: { name: 'Relative strength', line: 'How much of the recent movement was upward, from 0% to 100%.', windowed: true, scale: 'percent' },
};

const MEASURE_WORDS: Record<Indicator, string> = {
  price: 'price',
  average_price: 'average price',
  exponential_average_price: 'weighted average price',
  price_deviation: 'spread of prices',
  cumulative_return: 'return',
  average_return: 'average daily return',
  return_deviation: 'volatility',
  max_drawdown: 'worst drop',
  relative_strength: 'relative strength',
};

/** A measure as it reads after a ticker or after "its": "200-day average price". */
export function measureWords(measure: Measure): string {
  const words = MEASURE_WORDS[measure.indicator];
  return MEASURES[measure.indicator].windowed ? `${measure.window ?? 0}-day ${words}` : words;
}

export const COMPARATOR_WORDS: Record<Comparator, string> = {
  above: 'is above',
  at_least: 'is at least',
  equal: 'equals',
  below: 'is below',
  at_most: 'is at most',
};

const readingWords = (reading: Reading) => `${symbolOf(reading.asset)}'s ${measureWords(reading)}`;

/** A fixed number in the scale of the measure it is compared with: a share as a percent, a price in dollars. */
export function fixedWords(indicator: Indicator, fixed: number): string {
  return MEASURES[indicator].scale === 'percent' ? (formatPercent(fixed) ?? '') : `$${formatQuantity(fixed) ?? ''}`;
}

export function comparisonWords(comparison: Comparison): string {
  const { left, right } = comparison;
  let against: string;
  if ('fixed' in right) against = fixedWords(left.indicator, right.fixed);
  // The same asset on both sides is "its", so the line reads as one thought.
  else if (symbolOf(right.asset) === symbolOf(left.asset)) against = `its ${measureWords(right)}`;
  else against = readingWords(right);
  return `${readingWords(left)} ${COMPARATOR_WORDS[comparison.compare]} ${against}`;
}

/** A whole condition in one line. A group names how many of its parts must hold. */
export function conditionWords(condition: Condition): string {
  if ('any' in condition) return condition.any.length === 1 ? conditionWords(condition.any[0]) : `any of ${condition.any.length} conditions is true`;
  if ('all' in condition) return condition.all.length === 1 ? conditionWords(condition.all[0]) : `all of ${condition.all.length} conditions are true`;
  return comparisonWords(condition);
}

/** The comparisons of a condition, one level deep: a single comparison, or the parts of its group. */
export function partsOf(condition: Condition): Condition[] {
  if ('any' in condition) return condition.any;
  if ('all' in condition) return condition.all;
  return [condition];
}

export function splitWords(block: WeightBlock): string {
  const how =
    block.weighting.type === 'equal'
      ? 'Split equally between'
      : block.weighting.type === 'specified'
        ? 'Split in set shares between'
        : `Split between these, with more in whichever has been calmer over ${block.weighting.window} days`;
  return block.name ? `${block.name}: ${how[0].toLowerCase()}${how.slice(1)}` : how;
}

/** What a block says on its pill. An asset says its ticker. */
export function blockWords(block: Block): string {
  switch (block.block) {
    case 'asset':
      return symbolOf(block.asset);
    case 'weight':
      return splitWords(block);
    case 'if':
      return `If ${conditionWords(block.condition)}`;
    case 'filter':
      return `Of these, hold ${block.count === 1 ? 'the one' : `the ${block.count}`} with the ${block.take === 'top' ? 'highest' : 'lowest'} ${measureWords(block.sort_by)}`;
  }
}

export const FREQUENCY_WORDS: Record<Frequency, string> = {
  daily: 'Checked every trading day',
  weekly: 'Every week',
  monthly: 'Every month',
  quarterly: 'Every quarter',
  yearly: 'Every year',
};

/** When a fund that follows the draft trades, in one line. */
export function tradesWords(rebalance: Rebalance): string {
  return rebalance.type === 'calendar' ? FREQUENCY_WORDS[rebalance.frequency] : `When a holding drifts ${formatPercent(rebalance.corridor) ?? ''} from its target`;
}

/** The kinds of block a person can add, with one plain line each. */
export const BLOCK_KINDS = [
  { kind: 'asset', name: 'Asset', line: 'A stock or a fund.' },
  { kind: 'weight', name: 'Split', line: 'How to share money between the rules under it.' },
  { kind: 'if', name: 'If / Otherwise', line: 'Hold one thing while a condition is true, another while it is not.' },
  { kind: 'filter', name: 'Pick', line: 'Keep the best or worst few from a list.' },
] as const;
