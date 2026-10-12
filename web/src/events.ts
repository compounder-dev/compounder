// The app's own log of named events, kept by our own server and by no one else.
// An event says which screen and which fund. It says nothing about who.

import { useEffect } from 'react';

/** The events of this step. The server keeps the same list and refuses any other name. */
export type EventName = 'home_seen' | 'explore_seen' | 'strategy_opened' | 'pricing_seen';

export function logEvent(name: EventName, screen: string, id?: string) {
  const body = new Blob([JSON.stringify({ name, screen, id })], { type: 'application/json' });
  // A beacon is sent even as the page is left, and a lost one is not worth a retry.
  navigator.sendBeacon?.('/v1/events', body);
}

/** Log an event once each time its screen is shown. `ready` holds it back until the screen has what it shows. */
export function useEvent(name: EventName, screen: string, id?: string, ready = true) {
  useEffect(() => {
    if (ready) logEvent(name, screen, id);
  }, [name, screen, id, ready]);
}
