// What a strategy can be built from: every listed share or fund the server has prices for.
// Asked for once and kept, since every picker and every pill reads it.

import { useEffect, useState } from 'react';

export type Listed = { symbol: string; name: string | null; first_day: string };

type Universe = { state: 'loading' } | { state: 'ready'; assets: Listed[]; names: Map<string, string> } | { state: 'failed' };

let kept: Universe = { state: 'loading' };
let asked: Promise<void> | null = null;
const listeners = new Set<() => void>();

function ask(): Promise<void> {
  asked ??= fetch('/v1/universe', { headers: { Accept: 'application/json' } })
    .then(async (response) => {
      const body = (await response.json()) as { assets?: Listed[] };
      if (!response.ok || !Array.isArray(body.assets)) throw new Error('no universe');
      kept = { state: 'ready', assets: body.assets, names: new Map(body.assets.flatMap((asset) => (asset.name ? [[asset.symbol, asset.name]] : []))) };
    })
    .catch(() => {
      kept = { state: 'failed' };
      // A failure is not kept for good: the next screen that needs the list asks again.
      asked = null;
    })
    .finally(() => listeners.forEach((listener) => listener()));
  return asked;
}

export function useUniverse(): Universe {
  const [, redraw] = useState(0);
  useEffect(() => {
    const listener = () => redraw((count) => count + 1);
    listeners.add(listener);
    if (kept.state !== 'ready') void ask();
    return () => void listeners.delete(listener);
  }, []);
  return kept;
}

/** The assets whose ticker or name has every word of what was typed, tickers that start with it first. */
export function searched(assets: Listed[], typed: string, most = 60): Listed[] {
  const words = typed.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return assets.slice(0, most);
  const first = words[0];
  const found = assets.filter((asset) => {
    const text = `${asset.symbol} ${asset.name ?? ''}`.toLowerCase();
    return words.every((word) => text.includes(word));
  });
  const rank = (asset: Listed) => (asset.symbol.toLowerCase() === first ? 0 : asset.symbol.toLowerCase().startsWith(first) ? 1 : 2);
  return found.sort((a, b) => rank(a) - rank(b)).slice(0, most);
}
