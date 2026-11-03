// Addresses: every screen has a path that survives a reload.

import { useSyncExternalStore } from 'react';

export type Route = { screen: 'fund' } | { screen: 'you' } | { screen: 'markets' } | { screen: 'asset'; market: string } | { screen: 'strategies' } | { screen: 'strategy'; id: string };

export function match(path: string): Route {
  const asset = /^\/markets\/([A-Za-z0-9:%_-]+)$/.exec(path);
  if (asset) return { screen: 'asset', market: decodeURIComponent(asset[1]) };
  const strategy = /^\/(?:strategies|managers)\/([A-Za-z0-9_-]+)$/.exec(path);
  if (strategy) return { screen: 'strategy', id: strategy[1] };
  if (path === '/strategies' || path === '/managers') return { screen: 'strategies' };
  if (path === '/you') return { screen: 'you' };
  if (path === '/markets') return { screen: 'markets' };
  return { screen: 'fund' };
}

const listeners = new Set<() => void>();
function subscribe(listener: () => void) {
  listeners.add(listener);
  window.addEventListener('popstate', listener);
  return () => {
    listeners.delete(listener);
    window.removeEventListener('popstate', listener);
  };
}

export function go(to: string) {
  if (to === window.location.pathname) return;
  window.history.pushState(null, '', to);
  listeners.forEach((listener) => listener());
  window.scrollTo(0, 0);
}

export function back(fallback = '/') {
  if (window.history.state !== null || window.history.length > 1) window.history.back();
  else go(fallback);
}

export function usePath(): string {
  return useSyncExternalStore(subscribe, () => window.location.pathname);
}

export const tradePath = (market: string) => `/markets/${encodeURIComponent(market)}`;
export const fundPath = (id: string) => `/strategies/${id}`;
