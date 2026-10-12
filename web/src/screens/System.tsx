// The screens for when there is no screen: not found, something broke, offline.

import { Component, useEffect, useSyncExternalStore, type ReactNode } from 'react';
import { retryFailed } from '../api.ts';
import { ButtonLink, Icon } from '../parts/index.ts';
import { useTitle } from '../route.ts';

export function NotFound() {
  useTitle('Page not found · Compounder');
  return (
    <div className="system">
      <h1 className="page-title">That page does not exist.</h1>
      <ButtonLink variant="primary" href="/">
        Go home
      </ButtonLink>
    </div>
  );
}

/** Both ways out load the page afresh, since whatever broke may still be in memory. */
export function Broke({ path }: { path: string }) {
  useTitle('Something went wrong · Compounder');
  return (
    <div className="system" role="alert">
      <h1 className="page-title">Something went wrong on our side.</h1>
      <div className="system-actions">
        <ButtonLink variant="primary" href={path} reload>
          Try again
        </ButtonLink>
        {path !== '/' && (
          <ButtonLink variant="ghost" href="/" reload>
            Go home
          </ButtonLink>
        )}
      </div>
    </div>
  );
}

/** Catches an error no screen expected and shows the page for it in place of the screen. */
export class Boundary extends Component<{ path: string; children: ReactNode }, { broke: boolean }> {
  state = { broke: false };

  static getDerivedStateFromError() {
    return { broke: true };
  }

  render() {
    return this.state.broke ? <Broke path={this.props.path} /> : this.props.children;
  }
}

function subscribe(listener: () => void) {
  window.addEventListener('online', listener);
  window.addEventListener('offline', listener);
  return () => {
    window.removeEventListener('online', listener);
    window.removeEventListener('offline', listener);
  };
}

/**
 * A thin bar while the device has no network. The bar is always in the page and
 * only its words come and go, which is what lets a screen reader announce them.
 */
export function OfflineBar() {
  const online = useSyncExternalStore(
    subscribe,
    () => navigator.onLine,
    () => true,
  );
  // Whatever failed while the network was away is asked for again when it returns.
  useEffect(() => {
    if (online) retryFailed();
  }, [online]);
  return (
    <div className="offline-bar" role="status">
      {!online && (
        <>
          <Icon name="offline" size="sm" />
          You are offline. Showing what we last had.
        </>
      )}
    </div>
  );
}
