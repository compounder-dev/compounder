import type { ReactNode } from 'react';
import { Icon, type IconName } from './Icon.tsx';

/** A region with nothing in it: one sentence and one way forward. No picture. */
export function EmptyState({ children, action }: { children: ReactNode; action?: ReactNode }) {
  return (
    <div className="empty">
      <p>{children}</p>
      {action}
    </div>
  );
}

const BANNERS = {
  // "Needs you" is the only notice that carries the action colour: gold means it is your move.
  'needs-you': { icon: 'alert', role: 'status' },
  info: { icon: 'info', role: 'status' },
  error: { icon: 'alert', role: 'alert' },
} as const satisfies Record<string, { icon: IconName; role: 'status' | 'alert' }>;

export type BannerVariant = keyof typeof BANNERS;

/** A notice at the top of the page it is about: one sentence, one action. */
export function Banner({ variant = 'info', children, action }: { variant?: BannerVariant; children: ReactNode; action?: ReactNode }) {
  const { icon, role } = BANNERS[variant];
  return (
    <div className={`banner banner-${variant}`} role={role}>
      <Icon name={icon} />
      <p>{children}</p>
      {action}
    </div>
  );
}
