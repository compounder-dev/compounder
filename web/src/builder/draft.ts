// A strategy while it is being built: the rule document the server tests, and the
// edits a person can make to it. Every edit gives back a new document and leaves
// the old one as it was, so undo is a list of documents.

export type AssetRef = { market: 'equities' | 'crypto'; symbol: string } | { token: string };

export const INDICATORS = [
  'price',
  'average_price',
  'exponential_average_price',
  'price_deviation',
  'cumulative_return',
  'average_return',
  'return_deviation',
  'max_drawdown',
  'relative_strength',
] as const;
export type Indicator = (typeof INDICATORS)[number];

export type Measure = { indicator: Indicator; window?: number };
export type Reading = Measure & { asset: AssetRef };

export const COMPARATORS = ['above', 'at_least', 'equal', 'below', 'at_most'] as const;
export type Comparator = (typeof COMPARATORS)[number];

export type Comparison = { compare: Comparator; left: Reading; right: Reading | { fixed: number } };
export type Condition = Comparison | { any: Condition[] } | { all: Condition[] };

export type Weighting = { type: 'equal' } | { type: 'specified'; shares: number[] } | { type: 'inverse_volatility'; window: number };

export type AssetBlock = { block: 'asset'; asset: AssetRef };
export type WeightBlock = { block: 'weight'; name?: string; weighting: Weighting; children: Block[] };
export type IfBlock = { block: 'if'; condition: Condition; then: Block[]; else: Block[] };
export type FilterBlock = { block: 'filter'; sort_by: Measure; take: 'top' | 'bottom'; count: number; children: Block[] };
export type Block = AssetBlock | WeightBlock | IfBlock | FilterBlock;

export const FREQUENCIES = ['daily', 'weekly', 'monthly', 'quarterly', 'yearly'] as const;
export type Frequency = (typeof FREQUENCIES)[number];
export type Rebalance = { type: 'calendar'; frequency: Frequency } | { type: 'threshold'; corridor: number };

export type Draft = { name: string; rebalance: Rebalance; root: Block };

/** Which list of a block a child sits in. */
export type Slot = 'children' | 'then' | 'else';
/** The way from the root to a block: the list it is in and its place there, step by step. */
export type Path = (Slot | number)[];

/** A new draft: an empty split, waiting for what it holds. */
export function blank(): Draft {
  return { name: 'My fund', rebalance: { type: 'calendar', frequency: 'monthly' }, root: { block: 'weight', weighting: { type: 'equal' }, children: [] } };
}

export const listed = (symbol: string): AssetBlock => ({ block: 'asset', asset: { market: 'equities', symbol } });

/** The ticker of an asset, where it has one. */
export function symbolOf(asset: AssetRef): string {
  return 'symbol' in asset ? asset.symbol : asset.token.slice(0, 6);
}

/** The lists a block holds other blocks in, in the order they are drawn. */
export function slotsOf(block: Block): Slot[] {
  if (block.block === 'if') return ['then', 'else'];
  return block.block === 'asset' ? [] : ['children'];
}

function listOf(block: Block, slot: Slot): Block[] | null {
  if (block.block === 'if' && (slot === 'then' || slot === 'else')) return block[slot];
  if ((block.block === 'weight' || block.block === 'filter') && slot === 'children') return block.children;
  return null;
}

/** The block at the end of a path, or null when the path leads nowhere. */
export function at(root: Block, path: Path): Block | null {
  let here: Block | null = root;
  for (let step = 0; here && step < path.length; step += 2) {
    here = listOf(here, path[step] as Slot)?.[path[step + 1] as number] ?? null;
  }
  return here;
}

/** The document with the block at `path` changed by `change`. A path that leads nowhere changes nothing. */
function changed(root: Block, path: Path, change: (block: Block) => Block): Block {
  if (path.length === 0) return change(root);
  const [slot, place, ...rest] = path as [Slot, number, ...Path];
  const list = listOf(root, slot);
  if (!list || !list[place]) return root;
  const child = changed(list[place], rest, change);
  // A change that changed nothing gives back the very same block, so the draft above it is the same too.
  if (child === list[place]) return root;
  return { ...root, [slot]: list.map((each, index) => (index === place ? child : each)) } as Block;
}

/** The draft with a new root, or the very same draft when the root is the same one. */
const rooted = (draft: Draft, root: Block): Draft => (root === draft.root ? draft : { ...draft, root });

/** Shares for a split whose children have changed: a new child starts with none, and a removed one takes its share with it. */
function sharesAfter(weighting: Weighting, change: (shares: number[]) => number[]): Weighting {
  return weighting.type === 'specified' ? { type: 'specified', shares: change(weighting.shares) } : weighting;
}

export function replaced(draft: Draft, path: Path, block: Block): Draft {
  return rooted(draft, changed(draft.root, path, () => block));
}

/** Put `blocks` at the end of one list of the block at `path`. */
export function added(draft: Draft, path: Path, slot: Slot, blocks: Block[]): Draft {
  const root = changed(draft.root, path, (parent) => {
    const list = listOf(parent, slot);
    if (!list) return parent;
    const next = { ...parent, [slot]: [...list, ...blocks] } as Block;
    return next.block === 'weight' ? { ...next, weighting: sharesAfter(next.weighting, (shares) => [...shares, ...blocks.map(() => 0)]) } : next;
  });
  return rooted(draft, root);
}

/** Take the block at `path` away. The root cannot be taken away. */
export function removed(draft: Draft, path: Path): Draft {
  if (path.length < 2) return draft;
  const slot = path[path.length - 2] as Slot;
  const place = path[path.length - 1] as number;
  const root = changed(draft.root, path.slice(0, -2), (parent) => {
    const list = listOf(parent, slot);
    if (!list || !list[place]) return parent;
    const next = { ...parent, [slot]: list.filter((_, index) => index !== place) } as Block;
    return next.block === 'weight' ? { ...next, weighting: sharesAfter(next.weighting, (shares) => shares.filter((_, index) => index !== place)) } : next;
  });
  return rooted(draft, root);
}

/** Move the block at `path` one place up (-1) or down (+1) in its list. At the end of the list it stays. */
export function moved(draft: Draft, path: Path, by: -1 | 1): Draft {
  if (!canMove(draft, path, by)) return draft;
  const slot = path[path.length - 2] as Slot;
  const from = path[path.length - 1] as number;
  const swap = <T>(list: T[]): T[] => {
    const to = from + by;
    const next = [...list];
    [next[from], next[to]] = [next[to], next[from]];
    return next;
  };
  const root = changed(draft.root, path.slice(0, -2), (parent) => {
    const list = listOf(parent, slot);
    if (!list) return parent;
    const next = { ...parent, [slot]: swap(list) } as Block;
    return next.block === 'weight' ? { ...next, weighting: sharesAfter(next.weighting, swap) } : next;
  });
  return rooted(draft, root);
}

/** Whether a block can move one place up or down in its list. */
export function canMove(draft: Draft, path: Path, by: -1 | 1): boolean {
  if (path.length < 2) return false;
  const parent = at(draft.root, path.slice(0, -2));
  const list = parent && listOf(parent, path[path.length - 2] as Slot);
  const to = (path[path.length - 1] as number) + by;
  return !!list && to >= 0 && to < list.length;
}

/** Equal shares that add up to exactly 1: the last one takes what rounding left over. */
export function evenShares(count: number): number[] {
  if (count === 0) return [];
  const each = Math.floor(1e6 / count) / 1e6;
  const shares = Array.from({ length: count }, () => each);
  shares[count - 1] = Number((1 - each * (count - 1)).toFixed(6));
  return shares;
}

/** A split's weighting changed to another kind. Set shares start out even. */
export function weightedBy(block: WeightBlock, type: Weighting['type']): WeightBlock {
  if (type === block.weighting.type) return block;
  if (type === 'specified') return { ...block, weighting: { type, shares: evenShares(block.children.length) } };
  if (type === 'inverse_volatility') return { ...block, weighting: { type, window: 63 } };
  return { ...block, weighting: { type: 'equal' } };
}

/** The server's place for a problem, such as `root.children[1].then[0]`, as a path. Null when it is not one. */
export function pathOf(place: string): Path | null {
  if (!place.startsWith('root')) return null;
  const path: Path = [];
  for (const step of place.slice(4).matchAll(/\.(children|then|else)\[(\d+)\]/g)) path.push(step[1] as Slot, Number(step[2]));
  // A place inside a condition, or one this does not know, belongs to the nearest block above it.
  return path;
}

export const samePath = (a: Path, b: Path) => a.length === b.length && a.every((step, index) => step === b[index]);

/** Every asset a draft names, in the blocks it holds and in the conditions it reads, each once. */
export function symbolsIn(draft: Draft): string[] {
  const found = new Set<string>();
  const readCondition = (condition: Condition) => {
    if ('any' in condition) condition.any.forEach(readCondition);
    else if ('all' in condition) condition.all.forEach(readCondition);
    else {
      found.add(symbolOf(condition.left.asset));
      if ('asset' in condition.right) found.add(symbolOf(condition.right.asset));
    }
  };
  const read = (block: Block) => {
    if (block.block === 'asset') found.add(symbolOf(block.asset));
    if (block.block === 'if') readCondition(block.condition);
    for (const slot of slotsOf(block)) listOf(block, slot)?.forEach(read);
  };
  read(draft.root);
  return [...found];
}

/** Whether something is a draft this code can open. A stored draft from an older build may not be. */
export function isDraft(value: unknown): value is Draft {
  const isBlock = (block: unknown): boolean => {
    if (typeof block !== 'object' || block === null) return false;
    const each = block as Record<string, unknown>;
    const lists = (names: string[]) => names.every((name) => Array.isArray(each[name]) && (each[name] as unknown[]).every(isBlock));
    if (each.block === 'asset') return typeof each.asset === 'object' && each.asset !== null;
    if (each.block === 'weight') return typeof each.weighting === 'object' && each.weighting !== null && lists(['children']);
    if (each.block === 'filter') return typeof each.sort_by === 'object' && each.sort_by !== null && lists(['children']);
    if (each.block === 'if') return typeof each.condition === 'object' && each.condition !== null && lists(['then', 'else']);
    return false;
  };
  if (typeof value !== 'object' || value === null) return false;
  const draft = value as Record<string, unknown>;
  return typeof draft.name === 'string' && typeof draft.rebalance === 'object' && draft.rebalance !== null && isBlock(draft.root);
}

/** The edits of one sitting: what the draft was, is, and was before it was undone. */
export type History = { past: Draft[]; now: Draft; future: Draft[] };

const KEPT = 100;

export const started = (draft: Draft): History => ({ past: [], now: draft, future: [] });

export function edited(history: History, draft: Draft): History {
  if (draft === history.now) return history;
  return { past: [...history.past, history.now].slice(-KEPT), now: draft, future: [] };
}

export function undone(history: History): History {
  const before = history.past[history.past.length - 1];
  return before ? { past: history.past.slice(0, -1), now: before, future: [history.now, ...history.future] } : history;
}

export function redone(history: History): History {
  const [next, ...rest] = history.future;
  return next ? { past: [...history.past, history.now], now: next, future: rest } : history;
}
