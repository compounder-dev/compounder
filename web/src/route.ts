// Addresses. Every screen has a real path that can be shared and reloaded.
// The server matches paths by the same rules, and contract/routes.json holds both to them.

import { useEffect, useSyncExternalStore, type MouseEvent } from 'react';

export const PRICING = '/pricing';

export type Route = { screen: 'home' } | { screen: 'pricing' } | { screen: 'starter'; id: string } | { screen: 'not-found' };

export function matchRoute(path: string): Route {
  if (path === '/') return { screen: 'home' };
  if (path === PRICING) return { screen: 'pricing' };
  const starter = /^\/s\/([A-Za-z0-9_-]+)$/.exec(path);
  if (starter) return { screen: 'starter', id: starter[1] };
  return { screen: 'not-found' };
}

export const starterAddress = (id: string) => `/s/${id}`;

/** Where an address from before real paths lives now: `#/portfolio/<id>` is `/s/<id>`. */
export function forwardedAddress(hash: string): string | null {
  if (hash === '#/') return '/';
  const old = /^#\/portfolio\/([A-Za-z0-9_-]+)$/.exec(hash);
  return old ? starterAddress(old[1]) : null;
}

/** A query with one key set, or removed when `value` is null. Other keys keep their place. */
export function withFlag(search: string, key: string, value: string | null): string {
  const query = new URLSearchParams(search);
  if (value === null) query.delete(key);
  else query.set(key, value);
  const text = query.toString();
  return text ? `?${text}` : '';
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

const here = () => window.location.pathname + window.location.search;

/** Go to an address inside the app without a page load. */
export function navigate(to: string, options: { replace?: boolean; keepScroll?: boolean } = {}) {
  if (to === here()) {
    // A tap on the place you are already in takes you back to its top.
    if (!options.keepScroll) window.scrollTo(0, 0);
    return;
  }
  if (options.replace) window.history.replaceState(window.history.state, '', to);
  else window.history.pushState(null, '', to);
  listeners.forEach((listener) => listener());
  if (!options.keepScroll) window.scrollTo(0, 0);
}

/**
 * Move an address from before real paths to its new home, without leaving a trace of the old one.
 * Run once before the first screen is drawn, and again whenever the part after the hash changes.
 */
export function forwardOldAddress(): boolean {
  const to = forwardedAddress(window.location.hash);
  if (to === null) return false;
  window.history.replaceState(window.history.state, '', to);
  listeners.forEach((listener) => listener());
  return true;
}

/** The path and query in the address bar. `rendered` is the address a page was made for outside a browser. */
export function useAddress(rendered?: string): { path: string; search: string } {
  const address = useSyncExternalStore(subscribe, here, () => rendered ?? here());
  const cut = address.indexOf('?');
  return cut < 0 ? { path: address, search: '' } : { path: address.slice(0, cut), search: address.slice(cut) };
}

/**
 * One key of the query as state, so what it opens survives a reload, travels in a link
 * and closes on the back button. Setting null clears it.
 */
export function useAddressFlag(key: string): [string | null, (value: string | null) => void] {
  const { path, search } = useAddress();
  const value = new URLSearchParams(search).get(key);
  const set = (next: string | null) => {
    if (next === value) return;
    // Closing what this visit opened goes back, so the back button is not left pointing at it.
    if (next === null && window.history.state?.flag === key) window.history.back();
    else if (next === null) navigate(path + withFlag(search, key, null), { replace: true, keepScroll: true });
    else {
      window.history.pushState({ flag: key }, '', path + withFlag(search, key, next));
      listeners.forEach((listener) => listener());
    }
  };
  return [value, set];
}

/** Whether a click on a link should be left to the browser: a new tab, a download, another site. */
export function leftToBrowser(click: { button: number; modified: boolean; target: string; sameOrigin: boolean; known: boolean }): boolean {
  return click.button !== 0 || click.modified || (click.target !== '' && click.target !== '_self') || !click.sameOrigin || !click.known;
}

/** Click handler for links to our own screens: go there in place, and let the browser do anything else. */
export function followLink(event: MouseEvent<HTMLAnchorElement>) {
  const anchor = event.currentTarget;
  const url = new URL(anchor.href);
  const click = {
    button: event.button,
    modified: event.metaKey || event.ctrlKey || event.shiftKey || event.altKey,
    target: anchor.target,
    sameOrigin: url.origin === window.location.origin,
    // An address this build does not know may exist in a newer one. The server answers for it.
    known: matchRoute(url.pathname).screen !== 'not-found',
  };
  if (event.defaultPrevented || anchor.hasAttribute('download') || leftToBrowser(click)) return;
  event.preventDefault();
  navigate(url.pathname + url.search);
}

/** The name of the screen in the tab and in the history list. */
export function useTitle(title: string) {
  useEffect(() => {
    document.title = title;
  }, [title]);
}
