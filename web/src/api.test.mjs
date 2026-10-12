import assert from 'node:assert/strict';
import { beforeEach, test } from 'node:test';
import { STARTERS, load, peek, reset, seed, starterPath } from './api.ts';

const starter = (id) => ({ id, name: id, summary: '', accent: 'blue', strategy: { tradingSettings: { type: 'calendar', frequency: 'weekly' } }, holdings: [] });

/** A server that answers once with a status and a body, and counts how often it was asked. */
function server(status, body) {
  const asked = [];
  const fetcher = async (path) => {
    asked.push(path);
    return new Response(JSON.stringify(body), { status });
  };
  return { asked, fetcher };
}

beforeEach(reset);

test('nothing asked for yet reads as on its way', () => {
  assert.deepEqual(peek(STARTERS), { state: 'loading' });
});

test('what the server put in the page is here without asking', async () => {
  seed({ [STARTERS]: { starters: [starter('one')] } });
  const { asked, fetcher } = server(200, {});
  await load(STARTERS, false, fetcher);
  assert.equal(peek(STARTERS).state, 'ready');
  assert.deepEqual(asked, []);
});

test('the list brings every starter with it', () => {
  seed({ [STARTERS]: { starters: [starter('one'), starter('two')] } });
  assert.deepEqual(peek(starterPath('two')), { state: 'ready', data: { starter: starter('two') } });
});

test('a null from the server means there is no such thing', () => {
  seed({ [starterPath('gone')]: null });
  assert.deepEqual(peek(starterPath('gone')), { state: 'missing' });
});

test('data in the page that makes no sense is asked for again', async () => {
  seed({ [STARTERS]: { starters: 'three' }, [starterPath('x')]: 7, [starterPath('y')]: { starter: null } });
  assert.equal(peek(starterPath('y')).state, 'loading');
  assert.equal(peek(STARTERS).state, 'loading');
  const { asked, fetcher } = server(200, { starters: [] });
  await load(STARTERS, false, fetcher);
  assert.deepEqual(asked, [STARTERS]);
  assert.deepEqual(peek(STARTERS), { state: 'ready', data: { starters: [] } });
});

test('a 404 is missing, any other bad answer is a failure', async () => {
  await load(starterPath('gone'), false, server(404, { error: {} }).fetcher);
  assert.equal(peek(starterPath('gone')).state, 'missing');
  await load(STARTERS, false, server(500, {}).fetcher);
  assert.equal(peek(STARTERS).state, 'failed');
});

test('an answer of the wrong shape is a failure, not a crash later', async () => {
  await load(STARTERS, false, server(200, { starters: null }).fetcher);
  assert.equal(peek(STARTERS).state, 'failed');
  await load(starterPath('x'), false, server(200, 'hello').fetcher);
  assert.equal(peek(starterPath('x')).state, 'failed');
});

test('a network that fails is a failure, and a second try can succeed', async () => {
  const down = async () => {
    throw new TypeError('offline');
  };
  await load(STARTERS, false, down);
  assert.equal(peek(STARTERS).state, 'failed');
  // Without `again` a failure is not asked for a second time.
  const { asked, fetcher } = server(200, { starters: [starter('one')] });
  await load(STARTERS, false, fetcher);
  assert.deepEqual(asked, []);
  await load(STARTERS, true, fetcher);
  assert.equal(peek(STARTERS).state, 'ready');
});

test('two requests for the same thing at once ask the server once', async () => {
  const { asked, fetcher } = server(200, { starters: [] });
  await Promise.all([load(STARTERS, false, fetcher), load(STARTERS, true, fetcher)]);
  assert.deepEqual(asked, [STARTERS]);
});

test('what is on screen stays while it is asked for again, and stays if that fails', async () => {
  seed({ [STARTERS]: { starters: [starter('one')] } });
  let during;
  const slow = async () => {
    during = peek(STARTERS).state;
    return new Response('{}', { status: 500 });
  };
  await load(STARTERS, true, slow);
  assert.equal(during, 'ready');
  assert.equal(peek(STARTERS).state, 'ready');
});
