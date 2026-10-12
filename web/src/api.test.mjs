import assert from 'node:assert/strict';
import { beforeEach, test } from 'node:test';
import { STRATEGIES, load, peek, reset, seed, strategyPath } from './api.ts';

const strategy = (id) => ({ id, name: id, summary: '', accent: 'blue', kind: 'basket', starter: true, trades: 'Every week', holdings: [] });

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
  assert.deepEqual(peek(STRATEGIES), { state: 'loading' });
});

test('what the server put in the page is here without asking', async () => {
  seed({ [STRATEGIES]: { strategies: [strategy('one')] } });
  const { asked, fetcher } = server(200, {});
  await load(STRATEGIES, false, fetcher);
  assert.equal(peek(STRATEGIES).state, 'ready');
  assert.deepEqual(asked, []);
});

test('the list brings every strategy with it', () => {
  seed({ [STRATEGIES]: { strategies: [strategy('one'), strategy('two')] } });
  assert.deepEqual(peek(strategyPath('two')), { state: 'ready', data: { strategy: strategy('two') } });
});

test('a null from the server means there is no such thing', () => {
  seed({ [strategyPath('gone')]: null });
  assert.deepEqual(peek(strategyPath('gone')), { state: 'missing' });
});

test('data in the page that makes no sense is asked for again', async () => {
  seed({ [STRATEGIES]: { strategies: 'three' }, [strategyPath('x')]: 7, [strategyPath('y')]: { strategy: null } });
  assert.equal(peek(strategyPath('y')).state, 'loading');
  assert.equal(peek(STRATEGIES).state, 'loading');
  const { asked, fetcher } = server(200, { strategies: [] });
  await load(STRATEGIES, false, fetcher);
  assert.deepEqual(asked, [STRATEGIES]);
  assert.deepEqual(peek(STRATEGIES), { state: 'ready', data: { strategies: [] } });
});

test('a 404 is missing, any other bad answer is a failure', async () => {
  await load(strategyPath('gone'), false, server(404, { error: {} }).fetcher);
  assert.equal(peek(strategyPath('gone')).state, 'missing');
  await load(STRATEGIES, false, server(500, {}).fetcher);
  assert.equal(peek(STRATEGIES).state, 'failed');
});

test('an answer of the wrong shape is a failure, not a crash later', async () => {
  await load(STRATEGIES, false, server(200, { strategies: null }).fetcher);
  assert.equal(peek(STRATEGIES).state, 'failed');
  await load(strategyPath('x'), false, server(200, 'hello').fetcher);
  assert.equal(peek(strategyPath('x')).state, 'failed');
});

test('a network that fails is a failure, and a second try can succeed', async () => {
  const down = async () => {
    throw new TypeError('offline');
  };
  await load(STRATEGIES, false, down);
  assert.equal(peek(STRATEGIES).state, 'failed');
  // Without `again` a failure is not asked for a second time.
  const { asked, fetcher } = server(200, { strategies: [strategy('one')] });
  await load(STRATEGIES, false, fetcher);
  assert.deepEqual(asked, []);
  await load(STRATEGIES, true, fetcher);
  assert.equal(peek(STRATEGIES).state, 'ready');
});

test('two requests for the same thing at once ask the server once', async () => {
  const { asked, fetcher } = server(200, { strategies: [] });
  await Promise.all([load(STRATEGIES, false, fetcher), load(STRATEGIES, true, fetcher)]);
  assert.deepEqual(asked, [STRATEGIES]);
});

test('what is on screen stays while it is asked for again, and stays if that fails', async () => {
  seed({ [STRATEGIES]: { strategies: [strategy('one')] } });
  let during;
  const slow = async () => {
    during = peek(STRATEGIES).state;
    return new Response('{}', { status: 500 });
  };
  await load(STRATEGIES, true, slow);
  assert.equal(during, 'ready');
  assert.equal(peek(STRATEGIES).state, 'ready');
});
