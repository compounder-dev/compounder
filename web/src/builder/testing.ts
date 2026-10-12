// A draft's test: asked of the server a moment after each edit, with the last
// good answer kept on screen while the next one is on its way.

import { useEffect, useRef, useState } from 'react';
import { formatPercent } from '../format.ts';
import { at, pathOf, type Draft, type Path } from './draft.ts';

export type Answer = {
  first_day: string;
  last_day: string;
  slippage: number;
  days: string[];
  values: number[];
  assets: { symbol: string | null; name: string | null }[];
  holdings: { weights: number[] };
  stats: { annual_return: number | null; max_drawdown: number | null; sharpe: number | null };
  benchmark: { symbol: string; first_day: string; last_day: string; values: (number | null)[]; stats: { annual_return: number | null } } | null;
};

export type Issue = { code: string; path: Path | null; message: string; fix: string };

export type Testing = {
  /** `empty` has nothing to test yet. `unavailable` is a server with no prices. */
  state: 'empty' | 'testing' | 'ready' | 'problems' | 'failed' | 'unavailable';
  /** The last good answer. It stays while a new one is asked for and while there are problems. */
  answer: Answer | null;
  issues: Issue[];
  /** How many different versions have been tested in this sitting. */
  versions: number;
};

const WAIT = 350;

/**
 * A problem in the words the builder uses. The server counts shares as fractions of one.
 * The builder shows them as percents, so a wrong total is said in percents too.
 */
export function inBuilderWords(issue: Issue, draft: Draft): Issue {
  const block = issue.path && at(draft.root, issue.path);
  if (issue.code === 'INVALID_WEIGHT_TOTAL' && block && block.block === 'weight' && block.weighting.type === 'specified') {
    const total = block.weighting.shares.reduce((sum, share) => sum + share, 0);
    return { ...issue, message: `These shares add up to ${formatPercent(total) ?? ''}.`, fix: 'Change them to add up to 100%.' };
  }
  return issue;
}

/** What of a draft a test depends on. Its name is not part of it. */
const tested = (draft: Draft) => JSON.stringify({ rebalance: draft.rebalance, root: draft.root });

const isEmpty = (draft: Draft) => draft.root.block === 'weight' && draft.root.children.length === 0;

export function useTesting(draft: Draft): Testing {
  const [testing, setTesting] = useState<Testing>({ state: isEmpty(draft) ? 'empty' : 'testing', answer: null, issues: [], versions: 0 });
  const seen = useRef(new Set<string>());
  const key = tested(draft);

  useEffect(() => {
    if (isEmpty(draft)) {
      setTesting((before) => ({ ...before, state: 'empty', issues: [] }));
      return;
    }
    setTesting((before) => ({ ...before, state: 'testing' }));
    const stop = new AbortController();
    const timer = setTimeout(async () => {
      try {
        const response = await fetch('/v1/backtest', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json', Accept: 'application/json' },
          body: JSON.stringify({ strategy: { name: draft.name, rebalance: draft.rebalance, root: draft.root }, benchmark: 'SPY' }),
          signal: stop.signal,
        });
        const body = await response.json().catch(() => null);
        if (response.ok && body && Array.isArray(body.days)) {
          seen.current.add(key);
          setTesting({ state: 'ready', answer: body as Answer, issues: [], versions: seen.current.size });
        } else if (response.status === 422 && Array.isArray(body?.error?.issues)) {
          const issues: Issue[] = (body.error.issues as { code?: string; path?: string; message?: string; fix?: string }[]).map((issue) => ({
            code: issue.code ?? '',
            path: typeof issue.path === 'string' ? pathOf(issue.path) : null,
            message: issue.message ?? body.error.message ?? 'Something in the rules cannot be tested.',
            fix: issue.fix ?? '',
          }));
          // A refusal with no list still says why in its own message.
          setTesting((before) => ({ ...before, state: 'problems', issues: issues.length ? issues.map((issue) => inBuilderWords(issue, draft)) : [{ code: '', path: null, message: String(body.error.message ?? 'The rules cannot be tested.'), fix: '' }] }));
        } else {
          setTesting((before) => ({ ...before, state: response.status === 503 ? 'unavailable' : 'failed', issues: [] }));
        }
      } catch (error) {
        if (!stop.signal.aborted) setTesting((before) => ({ ...before, state: 'failed', issues: [] }));
      }
    }, WAIT);
    return () => {
      clearTimeout(timer);
      stop.abort();
    };
    // The key stands for everything of the draft that a test reads.
  }, [key]);

  return testing;
}
