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

export type Starter = {
  id: string;
  name: string;
  summary: string;
  accent: string;
  /** The full strategy document. Only the trading settings are read here; the engine reads the rest. */
  strategy: { tradingSettings: { type: 'threshold'; triggerPercentage: number } | { type: 'calendar'; frequency: string } };
  holdings: Holding[];
};

/** One thing asked of the server: on its way, here, not there at all, or failed. */
export type Loaded<T> = { state: 'loading' } | { state: 'ready'; data: T } | { state: 'missing' } | { state: 'failed' };

export const STARTERS = '/v1/starters';
export const starterPath = (id: string) => `${STARTERS}/${id}`;

const LOADING: Loaded<unknown> = { state: 'loading' };
const entries = new Map<string, Loaded<unknown>>();
const inFlight = new Set<string>();
const listeners = new Set<() => void>();

function put(path: string, entry: Loaded<unknown>) {
  entries.set(path, entry);
  // The list carries every starter in full, so opening one from the list needs no second request.
  if (path === STARTERS && entry.state === 'ready') {
    for (const starter of (entry.data as { starters: Starter[] }).starters) entries.set(starterPath(starter.id), { state: 'ready', data: { starter } });
  }
  listeners.forEach((listener) => listener());
}

/** Whether a body has the shape its path promises. A wrong shape is a failure, never a crash later. */
function understood(path: string, body: unknown): boolean {
  if (typeof body !== 'object' || body === null) return false;
  if (path === STARTERS) return Array.isArray((body as { starters?: unknown }).starters);
  const { starter } = body as { starter?: unknown };
  return typeof starter === 'object' && starter !== null;
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

export function useStarters(): [Loaded<Starter[]>, () => void] {
  const [entry, again] = useResource(STARTERS);
  return [entry.state === 'ready' ? { state: 'ready', data: (entry.data as { starters: Starter[] }).starters } : entry, again];
}

export function useStarter(id: string): [Loaded<Starter>, () => void] {
  const [entry, again] = useResource(starterPath(id));
  return [entry.state === 'ready' ? { state: 'ready', data: (entry.data as { starter: Starter }).starter } : entry, again];
}

/** Forget everything. For tests. */
export function reset() {
  entries.clear();
  inFlight.clear();
}
