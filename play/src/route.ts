// Addresses: every screen has a path that survives a reload.

import { useSyncExternalStore } from 'react';

export type Route = { screen: 'fund' } | { screen: 'markets' } | { screen: 'asset'; market: string } | { screen: 'managers' } | { screen: 'manager'; id: string };

export function match(path: string): Route {
  const asset = /^\/markets\/([A-Za-z0-9:%_-]+)$/.exec(path);
  if (asset) return { screen: 'asset', market: decodeURIComponent(asset[1]) };
  const manager = /^\/managers\/([A-Za-z0-9_-]+)$/.exec(path);
  if (manager) return { screen: 'manager', id: manager[1] };
  if (path === '/managers') return { screen: 'managers' };
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
export const fundPath = (id: string) => `/managers/${id}`;
