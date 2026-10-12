import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { forwardedAddress, leftToBrowser, matchRoute, starterAddress, withFlag } from './route.ts';

// The server reads the same file, so both sides send an address to the same screen.
const cases = JSON.parse(readFileSync(new URL('../contract/routes.json', import.meta.url), 'utf8'));

test('every address goes to the screen the contract says', () => {
  for (const { path, screen, id } of cases) {
    assert.deepEqual(matchRoute(path), id === undefined ? { screen } : { screen, id }, path);
  }
});

test('an old address after the hash forwards to its real path', () => {
  assert.equal(forwardedAddress('#/portfolio/hard-assets'), '/s/hard-assets');
  assert.equal(forwardedAddress('#/'), '/');
  assert.deepEqual(matchRoute(forwardedAddress('#/portfolio/ai-build-out')), { screen: 'starter', id: 'ai-build-out' });
});

test('a hash that is not an old address is left alone', () => {
  for (const hash of ['', '#', '#main', '#/portfolio/', '#/portfolio/a/b', '#/portfolio/a b', '#/elsewhere']) {
    assert.equal(forwardedAddress(hash), null, hash);
  }
});

test('a starter address is one the app matches', () => {
  assert.deepEqual(matchRoute(starterAddress('magnificent-seven')), { screen: 'starter', id: 'magnificent-seven' });
});

test('a flag in the query is set and cleared without touching the others', () => {
  assert.equal(withFlag('', 'sheet', 'start'), '?sheet=start');
  assert.equal(withFlag('?start=50', 'sheet', 'start'), '?start=50&sheet=start');
  assert.equal(withFlag('?start=50&sheet=start', 'sheet', null), '?start=50');
  assert.equal(withFlag('?sheet=start', 'sheet', null), '');
  assert.equal(withFlag('?sheet=a', 'sheet', 'b'), '?sheet=b');
});

test('only a plain click on one of our own screens is handled in place', () => {
  const plain = { button: 0, modified: false, target: '', sameOrigin: true, known: true };
  assert.equal(leftToBrowser(plain), false);
  assert.equal(leftToBrowser({ ...plain, target: '_self' }), false);
  assert.equal(leftToBrowser({ ...plain, button: 1 }), true);
  assert.equal(leftToBrowser({ ...plain, modified: true }), true);
  assert.equal(leftToBrowser({ ...plain, target: '_blank' }), true);
  assert.equal(leftToBrowser({ ...plain, sameOrigin: false }), true);
  assert.equal(leftToBrowser({ ...plain, known: false }), true);
});
