import { useEffect, useId, useReducer, useRef, type MouseEvent, type ReactNode, type SyntheticEvent } from 'react';
import { IconButton } from './Button.tsx';
import { mayClose, nextPhase } from './sheet.ts';

type SheetProps = {
  open: boolean;
  /** Asked for by Escape, the scrim, the cross or the back button. The owner decides. */
  onClose: () => void;
  title: string;
  /** An action sheet is as tall as what it holds. A full-height sheet takes the screen. */
  variant?: 'action' | 'full';
  /** Work is in flight: the body dims and nothing can close the sheet. */
  busy?: boolean;
  /** Buttons pinned to the foot. */
  actions?: ReactNode;
  children?: ReactNode;
};

/**
 * One part for sheet and dialog: it rises from the bottom edge on a phone and sits in
 * the middle from tablet width up. The browser's own dialog keeps focus inside it and
 * hands focus back to what opened it.
 */
export function Sheet({ open, onClose, title, variant = 'action', busy = false, actions, children }: SheetProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const [phase, send] = useReducer(nextPhase, 'closed');

  useEffect(() => send(open ? 'show' : 'hide'), [open]);

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    if (phase === 'closed' && element.open) element.close();
    if (phase !== 'closed' && !element.open) element.showModal();
    // Where nothing moves, nothing will say the motion ended. Come to rest at once.
    if ((phase === 'opening' || phase === 'closing') && element.getAnimations().length === 0) send('settled');
  }, [phase]);

  const ask = () => {
    if (mayClose(busy)) onClose();
  };
  // Escape asks to close. The owner's answer closes it, so a busy sheet can stay.
  const cancel = (event: SyntheticEvent) => {
    event.preventDefault();
    ask();
  };
  // The dialog element itself is only hit outside the panel, on the scrim.
  const scrim = (event: MouseEvent<HTMLDialogElement>) => {
    if (event.target === event.currentTarget) ask();
  };
  const settle = (event: SyntheticEvent) => {
    if (event.target === event.currentTarget) send('settled');
  };
  // A browser may close a dialog over our head, on a second Escape. Follow it, or reopen a busy sheet.
  const closed = () => {
    if (phase === 'closed') return;
    if (busy) dialog.current?.showModal();
    else {
      send('hide');
      send('settled');
      onClose();
    }
  };

  return (
    <dialog
      ref={dialog}
      className={`sheet sheet-${variant}`}
      data-phase={phase}
      aria-labelledby={titleId}
      aria-busy={busy || undefined}
      onCancel={cancel}
      onClose={closed}
      onClick={scrim}
      onAnimationEnd={settle}
    >
      <div className="sheet-panel">
        <span className="sheet-handle" aria-hidden="true" />
        <header className="sheet-head">
          <h2 className="sheet-title" id={titleId}>
            {title}
          </h2>
          <IconButton icon="close" label="Close" onClick={ask} disabled={busy} />
        </header>
        <div className="sheet-body">{children}</div>
        {actions !== undefined && <footer className="sheet-actions">{actions}</footer>}
      </div>
    </dialog>
  );
}
