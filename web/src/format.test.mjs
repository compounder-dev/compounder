import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { formatMoney, formatNumber, formatPercent } from './format.ts';

// The same cases hold the server to the same text, so a page reads the same before and after scripts run.
const cases = JSON.parse(readFileSync(new URL('../contract/numbers.json', import.meta.url), 'utf8'));

test('every number prints as the contract says', () => {
  for (const { kind, value, sign, text } of cases) {
    assert.equal(formatNumber(kind, value, { sign }), text, `${kind} ${value}`);
  }
});

test('zero is never negative and never carries a sign', () => {
  for (const value of [0, -0, 1e-9, -1e-9]) {
    assert.equal(formatPercent(value, { sign: true }), '0%');
  }
  assert.equal(formatMoney(-0), '$0.00');
});

test('a value that is not a number has no text', () => {
  for (const kind of ['money', 'percent', 'ratio', 'quantity', 'countdown']) {
    assert.equal(formatNumber(kind, null), null);
    assert.equal(formatNumber(kind, NaN), null);
    assert.equal(formatNumber(kind, Infinity), null);
  }
});

test('the same amount never prints two ways', () => {
  // Values that sit on either side of a change of form still agree with themselves.
  assert.equal(formatPercent(0.1), formatPercent(0.1 + 1e-12));
  assert.equal(formatMoney(999999.999), '$1.00M');
  assert.equal(formatMoney(1e6), '$1.00M');
});
