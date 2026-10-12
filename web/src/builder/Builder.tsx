import './builder.css';
import { useCallback, useEffect, useState } from 'react';
import { useEvent } from '../events.ts';
import { Button, Chip, IconButton } from '../parts/index.ts';
import { navigate, useAddress, useTitle } from '../route.ts';
import { added, at, blank, canMove, edited, isDraft, listed, moved, redone, removed, replaced, started, symbolsIn, undone, type AssetRef, type Block, type Draft, type History, type Path, type Slot } from './draft.ts';
import { AddSheet, AssetPicker, BlockSheet, startingComparison, TradesSheet, type BlockActions } from './Editors.tsx';
import { TestPanel, testLine } from './TestPanel.tsx';
import { useTesting } from './testing.ts';
import { Tree, type Problem } from './Tree.tsx';
import { useUniverse } from './universe.ts';
import { tradesWords } from './words.ts';

// The draft is kept in this browser, so it is still here after a reload or a day away.
const KEPT_AS = 'compounder.draft.v1';

function kept(): Draft {
  try {
    const read: unknown = JSON.parse(localStorage.getItem(KEPT_AS) ?? 'null');
    return isDraft(read) ? read : blank();
  } catch {
    return blank();
  }
}

function keep(draft: Draft) {
  try {
    localStorage.setItem(KEPT_AS, JSON.stringify(draft));
  } catch {
    // A browser that will not store keeps the draft for this visit only.
  }
}

/**
 * A fund's rules as a draft of one's own. A starter holds tokens, and a draft is tested on the
 * listed shares they track, so each token is swapped for its share.
 */
async function copyOf(id: string): Promise<Draft | null> {
  // The name comes from the address bar. Only one that could be a fund's is asked for.
  if (!/^[A-Za-z0-9_-]+$/.test(id)) return null;
  const [found, registry] = await Promise.all([fetch(`/v1/strategies/${id}`), fetch('/v1/assets')]);
  if (!found.ok || !registry.ok) return null;
  const { strategy } = (await found.json()) as { strategy?: { rules?: unknown } };
  const { assets } = (await registry.json()) as { assets?: { id: string; tracks?: string }[] };
  const tracked = new Map((assets ?? []).flatMap((asset) => (asset.tracks ? [[asset.id, asset.tracks] as const] : [])));
  const listedFor = (asset: AssetRef): AssetRef => ('token' in asset && tracked.has(asset.token) ? listed(tracked.get(asset.token) as string).asset : asset);
  // Every asset in the document, held or read, sits under a key called "asset".
  const swapped: unknown = JSON.parse(JSON.stringify(strategy?.rules ?? null), (key, value) => (key === 'asset' && typeof value === 'object' && value !== null ? listedFor(value as AssetRef) : value));
  return isDraft(swapped) ? { ...swapped, name: `My ${swapped.name}` } : null;
}

type Open = { kind: 'edit'; path: Path } | { kind: 'add'; path: Path; slot: Slot } | { kind: 'assets'; path: Path; slot: Slot } | { kind: 'trades' } | null;

/** The block a new rule starts as, before a person has said anything about it. */
function startingBlock(kind: 'weight' | 'if' | 'filter', draft: Draft): Block {
  if (kind === 'weight') return { block: 'weight', weighting: { type: 'equal' }, children: [] };
  if (kind === 'filter') return { block: 'filter', sort_by: { indicator: 'cumulative_return', window: 126 }, take: 'top', count: 1, children: [] };
  // A condition starts on an asset the draft already names, so it reads as part of the same fund.
  return { block: 'if', condition: startingComparison(symbolsIn(draft)[0] ?? 'SPY'), then: [], else: [] };
}

/** The builder: the rules on one side, and their test, kept up to date, on the other. */
export function Builder() {
  useTitle('Build a fund · Compounder');
  useEvent('builder_opened', 'build');
  const [history, setHistory] = useState<History>(() => started(kept()));
  const [open, setOpen] = useState<Open>(null);
  const draft = history.now;
  const testing = useTesting(draft);
  const universe = useUniverse();
  const names = universe.state === 'ready' ? universe.names : new Map<string, string>();
  const apply = useCallback((edit: (draft: Draft) => Draft) => setHistory((before) => edited(before, edit(before.now))), []);

  useEffect(() => keep(draft), [draft]);

  // "Change the rules" on a fund's page comes here with the fund named in the address.
  const { search } = useAddress();
  const from = new URLSearchParams(search).get('from');
  useEffect(() => {
    if (from === null) return;
    let current = true;
    void copyOf(from)
      .catch(() => null)
      .then((copy) => {
        if (!current) return;
        // The draft that was here can be had back with Undo.
        if (copy) apply(() => copy);
        navigate('/build', { replace: true, keepScroll: true });
      });
    return () => {
      current = false;
    };
  }, [from, apply]);

  // Undo and redo from the keyboard, unless the keys are meant for a field being typed in.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const key = event.key.toLowerCase();
      if (!(event.metaKey || event.ctrlKey) || (key !== 'z' && key !== 'y')) return;
      if (event.target instanceof HTMLElement && event.target.closest('input, select, textarea')) return;
      event.preventDefault();
      setHistory((before) => (key === 'y' || event.shiftKey ? redone(before) : undone(before)));
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const problems: Problem[] = testing.issues.flatMap((issue) => (issue.path ? [{ path: issue.path, message: issue.message, fix: issue.fix }] : []));
  const editing = open?.kind === 'edit' ? at(draft.root, open.path) : null;
  const adding = open?.kind === 'add' || open?.kind === 'assets' ? open : null;

  const actions: BlockActions = {};
  if (open?.kind === 'edit' && open.path.length >= 2) {
    const path = open.path;
    const place = path[path.length - 1] as number;
    const step = (by: -1 | 1) => () => {
      apply((before) => moved(before, path, by));
      setOpen({ kind: 'edit', path: [...path.slice(0, -1), place + by] });
    };
    if (canMove(draft, path, -1)) actions.up = step(-1);
    if (canMove(draft, path, 1)) actions.down = step(1);
    actions.remove = () => {
      apply((before) => removed(before, path));
      setOpen(null);
    };
  }

  const addKind = (kind: 'asset' | 'weight' | 'if' | 'filter') => {
    if (!adding) return;
    const { path, slot } = adding;
    if (kind === 'asset') return setOpen({ kind: 'assets', path, slot });
    const parent = at(draft.root, path);
    const place = parent && slot in parent ? (parent as unknown as Record<Slot, Block[]>)[slot].length : 0;
    apply((before) => added(before, path, slot, [startingBlock(kind, before)]));
    // A condition and a pick have something to say before they are of use, so their editor opens at once.
    setOpen(kind === 'weight' ? null : { kind: 'edit', path: [...path, slot, place] });
  };

  /** The tickers held directly in the list the picker is adding to. */
  const inList = (): string[] => {
    if (open?.kind !== 'assets') return [];
    const parent = at(draft.root, open.path);
    const list = parent && open.slot in parent ? (parent as unknown as Record<Slot, Block[]>)[open.slot] : [];
    return list.flatMap((block) => (block.block === 'asset' && 'symbol' in block.asset ? [block.asset.symbol] : []));
  };
  // A tap on an asset adds it, and a tap on one that is already there takes it out again.
  const pick = (symbol: string) => {
    if (open?.kind !== 'assets') return;
    const { path, slot } = open;
    apply((before) => {
      const parent = at(before.root, path);
      const list = parent && slot in parent ? (parent as unknown as Record<Slot, Block[]>)[slot] : [];
      const place = list.findIndex((block) => block.block === 'asset' && 'symbol' in block.asset && block.asset.symbol === symbol);
      return place < 0 ? added(before, path, slot, [listed(symbol)]) : removed(before, [...path, slot, place]);
    });
  };

  return (
    <div className="builder fund-blue">
      <header className="builder-head">
        <h1 className="builder-title">
          <label className="sr-only" htmlFor="builder-name">
            Name of the fund
          </label>
          <input
            id="builder-name"
            className="builder-name"
            value={draft.name}
            maxLength={60}
            autoComplete="off"
            // A name is typed a letter at a time, and each letter is not an edit to undo.
            onChange={(event) => setHistory((before) => ({ ...before, now: { ...before.now, name: event.target.value } }))}
          />
        </h1>
        <div className="builder-tools">
          <Chip variant="draft">Draft, kept on this device</Chip>
          <IconButton icon="undo" label="Undo" disabled={history.past.length === 0} onClick={() => setHistory(undone)} />
          <IconButton icon="redo" label="Redo" disabled={history.future.length === 0} onClick={() => setHistory(redone)} />
          <Button variant="ghost" onClick={() => apply(() => blank())}>
            Start over
          </Button>
        </div>
        <button type="button" className="builder-trades" onClick={() => setOpen({ kind: 'trades' })}>
          <span className="builder-trades-label">Trades</span>
          {tradesWords(draft.rebalance)}
          <span className="sr-only">, change</span>
        </button>
      </header>
      <div className="builder-body">
        <section className="builder-rules" aria-label="Rules">
          <Tree root={draft.root} names={names} problems={problems} onEdit={(path) => setOpen({ kind: 'edit', path })} onAdd={(path, slot) => setOpen({ kind: 'add', path, slot })} />
        </section>
        <TestPanel testing={testing} onProblem={(path) => setOpen({ kind: 'edit', path })} />
      </div>
      <button type="button" className="test-bar" onClick={() => document.getElementById('test')?.scrollIntoView({ behavior: 'smooth' })}>
        {testLine(testing)}
      </button>
      <AddSheet open={open?.kind === 'add'} onPick={addKind} onClose={() => setOpen(null)} />
      <AssetPicker open={open?.kind === 'assets'} title="Add assets" several chosen={inList()} onPick={pick} onClose={() => setOpen(null)} />
      <BlockSheet block={editing} actions={actions} onChange={(block) => open?.kind === 'edit' && apply((before) => replaced(before, open.path, block))} onClose={() => setOpen(null)} />
      <TradesSheet open={open?.kind === 'trades'} rebalance={draft.rebalance} onChange={(rebalance) => apply((before) => ({ ...before, rebalance }))} onClose={() => setOpen(null)} />
    </div>
  );
}
