// What the server knows, kept once in the browser. A page arrives with the
// answers its first screen needs already inside it, so nothing is fetched twice.

import { useEffect, useSyncExternalStore } from 'react';

export type Holding = {
  asset_id: string;
  symbol: string | null;
  name: string | null;
  weight: number;
};

/** The five identity colours a fund can carry, in their fixed order. */
export const FUND_COLOURS = ['blue', 'orange', 'teal', 'violet', 'magenta'] as const;
export type FundColour = (typeof FUND_COLOURS)[number];

/** What a test of a strategy over past prices came to. A figure the test could not work out is null. */
export type TestFigures = {
  first_day: string;
  last_day: string;
  /** The yearly gain over the tested days, as a fraction. */
  per_year: number | null;
  /** The largest fall from a high to a later low, as a fraction of the high. */
  worst_drop: number | null;
  sharpe: number | null;
  /** The benchmark's yearly gain over the same days. */
  benchmark_per_year: number | null;
  /** The share of each trade that the test took as its cost. */
  cost: number;
};

/** One line of a strategy's rules, in the server's words. An asset line is its ticker, with its name and share. */
export type OutlineLine = {
  depth: number;
  kind: 'split' | 'pick' | 'if' | 'condition' | 'then' | 'else' | 'asset';
  text: string;
  name?: string;
  share?: number;
};

export type Strategy = {
  id: string;
  name: string;
  summary: string;
  accent: string;
    /** A basket holds named assets in set shares. What a rules fund holds follows from prices. */
  kind: 'basket' | 'rules';
  /** One of the funds the home page leads with. */
  starter: boolean;
  /** The fixed document a browser can check the holdings against. Only a starter has one. */
  fixed?: unknown;
  /** When it trades, in the server's words. */
  trades: string;
  /** Its rules in plain words. A starter has none: what it holds is all its rules say. */
  outline?: OutlineLine[];
  /** Absent where the server has no prices to test the strategy on. */
  test?: TestFigures;
  holdings: Holding[];
};

/** One thing asked of the server: on its way, here, not there at all, or failed. */
export type Loaded<T> = { state: 'loading' } | { state: 'ready'; data: T } | { state: 'missing' } | { state: 'failed' };

/** A whole test, day by day: what the fund would have been worth, and its benchmark beside it. */
export type TestSeries = {
  days: string[];
  values: number[];
  benchmark: { symbol: string; values: (number | null)[] } | null;
};

/** A fund trading live on paper, as the board shows it. Times are milliseconds since 1970. */
export type LiveFund = {
  id: string;
  name: string;
  speed: 'velocity' | 'compounding';
  thesis: string;
  rules: string;
  model?: string;
  markets: string[];
  capital: number;
  opened: number;
  updated: number;
  equity: number;
  /** Since it opened, as a fraction. */
  change: number;
  change_day: number | null;
  worst_drop: number;
  fills: number;
  fees: number;
  funding: number;
  curve: [number, number][];
  note: string;
  decided: number | null;
  holdings: { market: string; weight: number }[];
  test?: { start: number; end: number; total_return: number; annual_return: number; sharpe: number; max_drawdown: number };
  verified: boolean;
  entries: number;
  head: string;
};

export type LiveDetail = LiveFund & {
  decisions: { t: number; note: string; targets: Record<string, number> }[];
  trades: { t: number; market: string; qty: number; px: number; fee: number }[];
  test_curve: [number, number][];
};

export const LIVE = '/v1/live';
export const liveFundPath = (id: string) => `${LIVE}/${id}`;

export const STRATEGIES = '/v1/strategies';
export const strategyPath = (id: string) => `${STRATEGIES}/${id}`;
export const strategyTestPath = (id: string) => `${STRATEGIES}/${id}/test`;

const LOADING: Loaded<unknown> = { state: 'loading' };
const entries = new Map<string, Loaded<unknown>>();
const inFlight = new Set<string>();
const listeners = new Set<() => void>();

function put(path: string, entry: Loaded<unknown>) {
  entries.set(path, entry);
  // The list carries every strategy in full, so opening one from the list needs no second request.
  if (path === STRATEGIES && entry.state === 'ready') {
    for (const strategy of (entry.data as { strategies: Strategy[] }).strategies) entries.set(strategyPath(strategy.id), { state: 'ready', data: { strategy } });
  }
  listeners.forEach((listener) => listener());
}

/** Whether a body has the shape its path promises. A wrong shape is a failure, never a crash later. */
function understood(path: string, body: unknown): boolean {
  if (typeof body !== 'object' || body === null) return false;
  if (path === STRATEGIES) return Array.isArray((body as { strategies?: unknown }).strategies);
  if (path === LIVE) return Array.isArray((body as { funds?: unknown }).funds);
  if (path.startsWith(`${LIVE}/`)) {
    const { fund } = body as { fund?: unknown };
    return typeof fund === 'object' && fund !== null;
  }
  if (/^\/v1\/strategies\/[^/]+\/test$/.test(path)) {
    const { days, values } = body as { days?: unknown; values?: unknown };
    return Array.isArray(days) && Array.isArray(values) && days.length === values.length && days.length > 0;
  }
  const { strategy } = body as { strategy?: unknown };
  return typeof strategy === 'object' && strategy !== null;
}

/** Take the answers the server put in the page. `null` means the server said there is no such thing. */
export function seed(resources: Record<string, unknown>) {
  for (const [path, body] of Object.entries(resources)) {
    if (body === null) put(path, { state: 'missing' });
    else if (understood(path, body)) put(path, { state: 'ready', data: body });
  }
}

export function peek(path: string): Loaded<unknown> {
  return entries.get(path) ?? LOADING;
}

/** Ask the server, unless the answer is here or on its way. `again` asks even after a failure or an answer. */
export async function load(path: string, again = false, fetcher: typeof fetch = fetch): Promise<void> {
  if (inFlight.has(path) || (entries.has(path) && !again)) return;
  inFlight.add(path);
  // What is on screen stays there while a good answer is asked for again, and stays if the asking fails.
  const before = peek(path);
  if (before.state !== 'ready') put(path, LOADING);
  let entry: Loaded<unknown> = before.state === 'ready' ? before : { state: 'failed' };
  try {
    const response = await fetcher(path, { headers: { Accept: 'application/json' } });
    const body: unknown = await response.json().catch(() => null);
    if (response.status === 404) entry = { state: 'missing' };
    else if (response.ok && understood(path, body)) entry = { state: 'ready', data: body };
  } catch {
    // The network failed. The entry stays as it was set above, and the screen offers another try.
  }
  inFlight.delete(path);
  put(path, entry);
}

/** Ask again for everything that failed, for when the network comes back. */
export function retryFailed() {
  for (const [path, entry] of entries) if (entry.state === 'failed') void load(path, true);
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

/** One answer from the server and a way to ask again. */
function useResource(path: string): [Loaded<unknown>, () => void] {
  const entry = useSyncExternalStore(
    subscribe,
    () => peek(path),
    () => peek(path),
  );
  useEffect(() => void load(path), [path]);
  return [entry, () => void load(path, true)];
}

export function useStrategies(): [Loaded<Strategy[]>, () => void] {
  const [entry, again] = useResource(STRATEGIES);
  return [entry.state === 'ready' ? { state: 'ready', data: (entry.data as { strategies: Strategy[] }).strategies } : entry, again];
}

export function useStrategy(id: string): [Loaded<Strategy>, () => void] {
  const [entry, again] = useResource(strategyPath(id));
  return [entry.state === 'ready' ? { state: 'ready', data: (entry.data as { strategy: Strategy }).strategy } : entry, again];
}

export function useStrategyTest(id: string): [Loaded<TestSeries>, () => void] {
  const [entry, again] = useResource(strategyTestPath(id));
  return [entry as Loaded<TestSeries>, again];
}

/** The ledgers move each minute, so a live answer is asked for again while it is on screen. */
function useFresh(path: string) {
  useEffect(() => {
    const timer = window.setInterval(() => void load(path, true), 60_000);
    return () => window.clearInterval(timer);
  }, [path]);
}

export function useLive(): [Loaded<LiveFund[]>, () => void] {
  const [entry, again] = useResource(LIVE);
  useFresh(LIVE);
  return [entry.state === 'ready' ? { state: 'ready', data: (entry.data as { funds: LiveFund[] }).funds } : entry, again];
}

export function useLiveFund(id: string): [Loaded<LiveDetail>, () => void] {
  const [entry, again] = useResource(liveFundPath(id));
  useFresh(liveFundPath(id));
  return [entry.state === 'ready' ? { state: 'ready', data: (entry.data as { fund: LiveDetail }).fund } : entry, again];
}

/** Forget everything. For tests. */
export function reset() {
  entries.clear();
  inFlight.clear();
}
