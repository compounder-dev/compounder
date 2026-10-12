import assert from 'node:assert/strict';
import { test } from 'node:test';
import { searched } from './universe.ts';

const assets = [
  { symbol: 'SPY', name: 'S&P 500', first_day: '1993-01-29' },
  { symbol: 'GLD', name: 'Gold', first_day: '2004-11-18' },
  { symbol: 'NEM', name: 'Newmont', first_day: '1990-01-02' },
  { symbol: 'GOOGL', name: 'Alphabet Class A', first_day: '2004-08-19' },
  { symbol: 'G', name: null, first_day: '2000-01-03' },
];
const found = (typed, most) => searched(assets, typed, most).map((asset) => asset.symbol);

test('nothing typed offers the list as it comes, well-known first', () => {
  assert.deepEqual(found(''), ['SPY', 'GLD', 'NEM', 'GOOGL', 'G']);
  assert.deepEqual(found('  ', 2), ['SPY', 'GLD']);
});

test('a search finds by ticker or by name, and puts the ticker that was typed first', () => {
  assert.deepEqual(found('g'), ['G', 'GLD', 'GOOGL']);
  assert.deepEqual(found('gold'), ['GLD']);
  assert.deepEqual(found('alphabet a'), ['GOOGL']);
  assert.deepEqual(found('new'), ['NEM']);
  assert.deepEqual(found('zzz'), []);
});
