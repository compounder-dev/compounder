import assert from 'node:assert/strict';
import { test } from 'node:test';
import { barShare, displayName, fundColour, holdsLine, markLetter, sameAllocation, ticker, tradesLine } from './starter.ts';

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

test('the trades line says when a fund trades', () => {
  assert.equal(tradesLine({ type: 'threshold', triggerPercentage: 10 }), 'When a holding drifts 10% from its target');
  assert.equal(tradesLine({ type: 'threshold', triggerPercentage: 7.5 }), 'When a holding drifts 7.5% from its target');
  assert.equal(tradesLine({ type: 'calendar', frequency: 'weekly' }), 'On a weekly schedule');
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
