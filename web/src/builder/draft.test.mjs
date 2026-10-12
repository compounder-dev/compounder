import assert from 'node:assert/strict';
import { test } from 'node:test';
import { added, at, blank, canMove, edited, evenShares, isDraft, listed, moved, pathOf, redone, removed, replaced, started, symbolsIn, undone, weightedBy } from './draft.ts';

const split = (weighting, children) => ({ block: 'weight', weighting, children });
const symbols = (block) => block.children.map((child) => child.asset?.symbol ?? child.block);

test('a new draft is an empty equal split', () => {
  const draft = blank();
  assert.deepEqual(draft.root, { block: 'weight', weighting: { type: 'equal' }, children: [] });
  assert.ok(isDraft(draft));
});

test('blocks are added at the end of a list, and the old draft is left as it was', () => {
  const first = blank();
  const second = added(first, [], 'children', [listed('SPY'), listed('TLT')]);
  assert.deepEqual(symbols(second.root), ['SPY', 'TLT']);
  assert.deepEqual(first.root.children, []);
  assert.deepEqual(at(second.root, ['children', 1]), listed('TLT'));
  assert.equal(at(second.root, ['children', 5]), null);
  assert.equal(at(second.root, ['then', 0]), null);
});

test('set shares follow their children when one is added, removed or moved', () => {
  let draft = { ...blank(), root: split({ type: 'specified', shares: [0.6, 0.4] }, [listed('VTI'), listed('AGG')]) };
  draft = added(draft, [], 'children', [listed('GLD')]);
  assert.deepEqual(draft.root.weighting.shares, [0.6, 0.4, 0]);
  draft = moved(draft, ['children', 0], 1);
  assert.deepEqual([symbols(draft.root), draft.root.weighting.shares], [['AGG', 'VTI', 'GLD'], [0.4, 0.6, 0]]);
  draft = removed(draft, ['children', 0]);
  assert.deepEqual([symbols(draft.root), draft.root.weighting.shares], [['VTI', 'GLD'], [0.6, 0]]);
});

test('a block moves inside its own list and stops at the ends', () => {
  const draft = added(blank(), [], 'children', [listed('A'), listed('B'), listed('C')]);
  assert.deepEqual(symbols(moved(draft, ['children', 2], -1).root), ['A', 'C', 'B']);
  // A move that cannot be made gives back the very same draft, so it is no edit to undo.
  assert.equal(moved(draft, ['children', 0], -1), draft);
  assert.equal(moved(draft, ['children', 2], 1), draft);
  assert.deepEqual([canMove(draft, ['children', 0], -1), canMove(draft, ['children', 0], 1), canMove(draft, ['children', 2], 1), canMove(draft, [], 1)], [false, true, false, false]);
});

test('edits reach blocks inside the branches of a condition', () => {
  const rule = { block: 'if', condition: { compare: 'above', left: { indicator: 'price', asset: { market: 'equities', symbol: 'SPY' } }, right: { indicator: 'average_price', window: 200, asset: { market: 'equities', symbol: 'QQQ' } } }, then: [listed('SPY')], else: [] };
  let draft = replaced(blank(), [], rule);
  draft = added(draft, [], 'else', [split({ type: 'equal' }, [])]);
  draft = added(draft, ['else', 0], 'children', [listed('BIL'), listed('TLT')]);
  assert.deepEqual(symbols(at(draft.root, ['else', 0])), ['BIL', 'TLT']);
  draft = removed(draft, ['else', 0, 'children', 0]);
  assert.deepEqual(symbols(at(draft.root, ['else', 0])), ['TLT']);
  // The assets a draft needs prices for: those it holds and those its conditions read.
  assert.deepEqual(symbolsIn(draft).sort(), ['QQQ', 'SPY', 'TLT']);
  // The root cannot be removed, and a path that leads nowhere changes nothing.
  assert.equal(removed(draft, []), draft);
  assert.equal(removed(draft, ['children', 9]), draft);
  assert.equal(removed(draft, ['else', 0, 'children', 4]), draft);
});

test('even shares add up to exactly one', () => {
  for (const count of [1, 2, 3, 6, 7, 9, 11]) {
    const shares = evenShares(count);
    assert.equal(shares.length, count);
    assert.equal(Number(shares.reduce((sum, share) => sum + share, 0).toFixed(9)), 1, String(count));
  }
  assert.deepEqual(evenShares(0), []);
});

test('a split changes how it weights and keeps its children', () => {
  const equal = split({ type: 'equal' }, [listed('A'), listed('B')]);
  assert.deepEqual(weightedBy(equal, 'specified').weighting, { type: 'specified', shares: [0.5, 0.5] });
  assert.deepEqual(weightedBy(equal, 'inverse_volatility').weighting, { type: 'inverse_volatility', window: 63 });
  assert.equal(weightedBy(equal, 'equal'), equal);
  assert.deepEqual(symbols(weightedBy(equal, 'specified')), ['A', 'B']);
});

test("the server's place for a problem is read as a path", () => {
  assert.deepEqual(pathOf('root'), []);
  assert.deepEqual(pathOf('root.children[1].then[0]'), ['children', 1, 'then', 0]);
  assert.deepEqual(pathOf('root.else[2].condition.all[1].left'), ['else', 2]);
  assert.equal(pathOf('rebalance'), null);
});

test('undo and redo walk the edits of a sitting', () => {
  const [a, b, c] = [blank(), added(blank(), [], 'children', [listed('A')]), added(blank(), [], 'children', [listed('B')])];
  let history = edited(edited(started(a), b), c);
  assert.equal(history.now, c);
  history = undone(history);
  assert.equal(history.now, b);
  history = undone(undone(undone(history)));
  assert.equal(history.now, a);
  history = redone(history);
  assert.equal(history.now, b);
  // A new edit after an undo forgets what was undone.
  history = edited(history, a);
  assert.deepEqual([history.now, redone(history).now, history.past.length], [a, a, 2]);
  // An edit that changes nothing is not an edit.
  assert.equal(edited(history, history.now), history);
});

test('a stored draft that is not one is told apart', () => {
  for (const odd of [null, 7, {}, { name: 'x', rebalance: {}, root: { block: 'nope' } }, { name: 'x', rebalance: {}, root: { block: 'weight', weighting: {}, children: [{}] } }]) assert.equal(isDraft(odd), false);
});

test('a wrong total of shares is said in percents, as the builder shows them', async () => {
  const { inBuilderWords } = await import('./testing.ts');
  const draft = { ...blank(), root: split({ type: 'specified', shares: [0.6, 0.5] }, [listed('VTI'), listed('AGG')]) };
  const issue = { code: 'INVALID_WEIGHT_TOTAL', path: [], message: 'The shares add up to 1.1, not 1.', fix: 'Adjust the shares so they add up to 1.' };
  assert.deepEqual(inBuilderWords(issue, draft), { ...issue, message: 'These shares add up to 110%.', fix: 'Change them to add up to 100%.' });
  // Any other problem is said as the server said it.
  const other = { code: 'INCOMPLETE_BLOCK', path: [], message: 'This weight block is empty.', fix: 'Add at least one block to it.' };
  assert.equal(inBuilderWords(other, draft), other);
});
