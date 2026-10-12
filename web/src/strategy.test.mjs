import assert from 'node:assert/strict';
import { test } from 'node:test';
import { barShare, displayName, fundColour, holdsLine, markLetter, matching, sameAllocation, ticker } from './strategy.ts';

const holding = (symbol, name = null, weight = 0) => ({ asset_id: '0xabcdef0123:56', symbol, name, weight });

test('a ticker drops the issuer ending and nothing else', () => {
  assert.equal(ticker(holding('NVDAon')), 'NVDA');
  assert.equal(ticker(holding('AMZN')), 'AMZN');
  assert.equal(ticker(holding('ONON')), 'ONON');
  assert.equal(ticker(holding(null)), '0xabcd');
});

test('a closing note in brackets is dropped from a name', () => {
  // The server's tests hold it to these same names.
  assert.equal(displayName(holding(null, 'Gold Trust (Tokenized)')), 'Gold Trust');
  assert.equal(displayName(holding(null, 'Class A (b (c)  ')), 'Class A');
  assert.equal(displayName(holding(null, 'Open (a) b)')), 'Open (a) b)');
  assert.equal(displayName(holding(null, '(Only)')), '');
  assert.equal(displayName(holding(null, 'Plain ')), 'Plain ');
  assert.equal(displayName(holding('SYMon')), 'SYMon');
  assert.equal(displayName(holding(null)), '0xabcdef0123:56');
});

test('the mark is the first letter of the ticker, whole', () => {
  assert.equal(markLetter(holding('NVDAon')), 'N');
  assert.equal(markLetter(holding('\u{1F600}X')), '\u{1F600}');
  assert.equal(markLetter(holding('')), '');
});

test('the holds line names up to five and counts the rest', () => {
  const of = (count) => Array.from({ length: count }, (_, index) => holding(`T${index + 1}on`));
  assert.equal(holdsLine(of(0)), '');
  assert.equal(holdsLine(of(1)), 'T1');
  assert.equal(holdsLine(of(5)), 'T1, T2, T3, T4, T5');
  assert.equal(holdsLine(of(6)), 'T1, T2, T3, T4 and 2 more');
  assert.equal(holdsLine(of(13)), 'T1, T2, T3, T4 and 9 more');
});

test('a search finds a fund by its name, its sentence, what it holds and what its rules name', () => {
  const fund = (id, kind, summary, held, outline) => ({ id, name: id, summary, accent: 'blue', kind, starter: false, trades: '', holdings: held, outline });
  const funds = [
    fund('Hard Assets', 'basket', 'Gold first.', [holding('GLDon', 'SPDR Gold Shares (Ondo Tokenized)')]),
    fund('Trend Switch', 'rules', 'Holds the S&P 500 or bills.', [holding('SPY', 'S&P 500')], [{ depth: 1, kind: 'asset', text: 'BIL', name: 'Short-term US government bills' }]),
  ];
  const found = (typed, kind = 'all') => matching(funds, typed, kind).map((each) => each.id);
  assert.deepEqual(found(''), ['Hard Assets', 'Trend Switch']);
  assert.deepEqual(found('  gold '), ['Hard Assets']);
  assert.deepEqual(found('GLD'), ['Hard Assets']);
  assert.deepEqual(found('bills trend'), ['Trend Switch']);
  // A fund is found by what its rules can hold, not only by what it holds today.
  assert.deepEqual(found('bil'), ['Trend Switch']);
  assert.deepEqual(found('s&p', 'basket'), []);
  assert.deepEqual(found('', 'rules'), ['Trend Switch']);
  assert.deepEqual(found('nothing here'), []);
});

test('a colour outside the five falls back to the first', () => {
  assert.equal(fundColour('teal'), 'teal');
  assert.equal(fundColour('amber'), 'blue');
  assert.equal(fundColour(''), 'blue');
});

test('a bar is a whole share of the largest holding', () => {
  assert.equal(barShare(0.4, 0.4), 100);
  assert.equal(barShare(0.1, 0.4), 25);
  assert.equal(barShare(0.05, 0.4), 13);
  assert.equal(barShare(0, 0.4), 0);
  assert.equal(barShare(0.1, 0), 0);
});

test('the browser check passes only on the same assets with the same shares', () => {
  const holdings = [{ ...holding('A'), asset_id: 'a', weight: 0.7 }, { ...holding('B'), asset_id: 'b', weight: 0.3 }];
  assert.equal(sameAllocation({ a: 700_000_000, b: 300_000_000 }, holdings), true);
  assert.equal(sameAllocation({ a: 700_000_001, b: 299_999_999 }, holdings), false);
  assert.equal(sameAllocation({ a: 700_000_000 }, holdings), false);
  assert.equal(sameAllocation({ a: 700_000_000, b: 300_000_000, c: 0 }, holdings), false);
  assert.equal(sameAllocation({ a: 700_000_000, c: 300_000_000 }, holdings), false);
});

test('a day is written as its month and year, and anything else as it stands', async () => {
  const { monthYear, yearOf } = await import('./strategy.ts');
  assert.equal(monthYear('2012-05-18'), 'May 2012');
  assert.equal(monthYear('2026-12-01'), 'Dec 2026');
  for (const odd of ['2012-13-18', '2012-00-18', '12-05-18', '2012-5-18', 'soon', '']) assert.equal(monthYear(odd), odd);
  assert.equal(yearOf('2012-05-18'), '2012');
});

test('a change that prints as zero has no colour', async () => {
  const { changeTone, asFall } = await import('./strategy.ts');
  assert.deepEqual([0.361, -0.02, 0.00001, -0.00001, null].map(changeTone), ['gain', 'loss', 'plain', 'plain', 'plain']);
  assert.deepEqual([asFall(0.5), asFall(null), Object.is(asFall(0), 0)], [-0.5, null, true]);
});
