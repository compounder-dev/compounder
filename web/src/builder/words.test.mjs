import assert from 'node:assert/strict';
import { test } from 'node:test';
import { blockWords, comparisonWords, conditionWords, fixedWords, measureWords, tradesWords } from './words.ts';

const asset = (symbol) => ({ market: 'equities', symbol });
const read = (indicator, symbol, window) => (window === undefined ? { indicator, asset: asset(symbol) } : { indicator, window, asset: asset(symbol) });

test('a measure names its days when it has them', () => {
  assert.equal(measureWords({ indicator: 'price' }), 'price');
  assert.equal(measureWords({ indicator: 'average_price', window: 200 }), '200-day average price');
  assert.equal(measureWords({ indicator: 'relative_strength', window: 10 }), '10-day relative strength');
});

test('a comparison on one asset reads as one thought', () => {
  assert.equal(comparisonWords({ compare: 'above', left: read('price', 'SPY'), right: read('average_price', 'SPY', 200) }), "SPY's price is above its 200-day average price");
  assert.equal(comparisonWords({ compare: 'at_least', left: read('cumulative_return', 'SPY', 252), right: read('cumulative_return', 'BIL', 252) }), "SPY's 252-day return is at least BIL's 252-day return");
});

test('a fixed number is written in the scale of what it is compared with', () => {
  assert.equal(comparisonWords({ compare: 'below', left: read('relative_strength', 'QQQ', 10), right: { fixed: 0.8 } }), "QQQ's 10-day relative strength is below 80%");
  assert.equal(comparisonWords({ compare: 'above', left: read('price', 'GLD'), right: { fixed: 180.5 } }), "GLD's price is above $180.5");
  assert.equal(fixedWords('max_drawdown', 0.075), '7.5%');
});

test('a group of conditions says how many must hold', () => {
  const one = { compare: 'above', left: read('price', 'SPY'), right: { fixed: 100 } };
  assert.equal(conditionWords({ all: [one, one] }), 'all of 2 conditions are true');
  assert.equal(conditionWords({ any: [one, one, one] }), 'any of 3 conditions is true');
  // A group of one is the one.
  assert.equal(conditionWords({ all: [one] }), "SPY's price is above $100");
});

test('each kind of block says what it does', () => {
  const held = [{ block: 'asset', asset: asset('VTI') }];
  assert.equal(blockWords(held[0]), 'VTI');
  assert.equal(blockWords({ block: 'weight', weighting: { type: 'equal' }, children: held }), 'Split equally between');
  assert.equal(blockWords({ block: 'weight', name: 'Safe side', weighting: { type: 'specified', shares: [1] }, children: held }), 'Safe side: split in set shares between');
  assert.equal(blockWords({ block: 'weight', weighting: { type: 'inverse_volatility', window: 63 }, children: held }), 'Split between these, with more in whichever has been calmer over 63 days');
  assert.equal(blockWords({ block: 'filter', sort_by: { indicator: 'cumulative_return', window: 126 }, take: 'top', count: 3, children: held }), 'Of these, hold the 3 with the highest 126-day return');
  assert.equal(blockWords({ block: 'filter', sort_by: { indicator: 'return_deviation', window: 20 }, take: 'bottom', count: 1, children: held }), 'Of these, hold the one with the lowest 20-day volatility');
  assert.equal(blockWords({ block: 'if', condition: { compare: 'above', left: read('price', 'SPY'), right: read('average_price', 'SPY', 200) }, then: held, else: [] }), "If SPY's price is above its 200-day average price");
});

test('when it trades is said in a few words', () => {
  assert.equal(tradesWords({ type: 'calendar', frequency: 'daily' }), 'Checked every trading day');
  assert.equal(tradesWords({ type: 'calendar', frequency: 'quarterly' }), 'Every quarter');
  assert.equal(tradesWords({ type: 'threshold', corridor: 0.075 }), 'When a holding drifts 7.5% from its target');
});
