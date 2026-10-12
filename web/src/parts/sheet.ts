// The life of a sheet, apart from the screen: closed, opening, open, closing.

export type SheetPhase = 'closed' | 'opening' | 'open' | 'closing';

/** What happened to a sheet: its owner wants it shown or hidden, or its motion came to rest. */
export type SheetEvent = 'show' | 'hide' | 'settled';

export function nextPhase(phase: SheetPhase, event: SheetEvent): SheetPhase {
  switch (event) {
    case 'show':
      // A sheet asked for again while it leaves comes straight back.
      return phase === 'open' ? 'open' : 'opening';
    case 'hide':
      return phase === 'closed' ? 'closed' : 'closing';
    case 'settled':
      if (phase === 'opening') return 'open';
      if (phase === 'closing') return 'closed';
      return phase;
  }
}

/** A sheet that is busy holds work in flight, so nothing may close it: not Escape, not the scrim, not the cross. */
export function mayClose(busy: boolean): boolean {
  return !busy;
}
