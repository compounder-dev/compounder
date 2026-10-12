import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mayClose, nextPhase } from './sheet.ts';

const run = (events, from = 'closed') => events.reduce(nextPhase, from);

test('a sheet opens, rests, leaves and is gone', () => {
  assert.equal(run(['show']), 'opening');
  assert.equal(run(['show', 'settled']), 'open');
  assert.equal(run(['show', 'settled', 'hide']), 'closing');
  assert.equal(run(['show', 'settled', 'hide', 'settled']), 'closed');
});

test('asking again changes nothing', () => {
  assert.equal(run(['show', 'settled', 'show']), 'open');
  assert.equal(run(['hide']), 'closed');
  assert.equal(run(['settled']), 'closed');
  assert.equal(run(['show', 'settled', 'settled']), 'open');
});

test('a sheet asked for while it leaves comes back, and one dismissed while it arrives leaves', () => {
  assert.equal(run(['show', 'settled', 'hide', 'show']), 'opening');
  assert.equal(run(['show', 'hide']), 'closing');
  assert.equal(run(['show', 'hide', 'settled']), 'closed');
});

test('a busy sheet cannot be closed', () => {
  assert.equal(mayClose(true), false);
  assert.equal(mayClose(false), true);
});
