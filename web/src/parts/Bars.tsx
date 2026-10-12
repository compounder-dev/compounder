import type { ReactNode } from 'react';
import { followLink } from '../route.ts';
import { Icon, type IconName } from './Icon.tsx';

/** One place the bars lead to. */
export type NavItem = {
  href: string;
  label: string;
  icon: IconName;
  /** Whether an address belongs to this place, so its tab shows as chosen on the screens under it. */
  holds: (path: string) => boolean;
  /** Something new waits there. */
  badge?: boolean;
};

/** `page` on the place itself, `true` on a screen under it, nothing elsewhere. */
export function current(item: NavItem, path: string): 'page' | 'true' | undefined {
  if (item.href === path) return 'page';
  return item.holds(path) ? 'true' : undefined;
}

function Wordmark() {
  return (
    <a className="wordmark" href="/" aria-label="Compounder, home" onClick={followLink}>
      <span>Compounder</span>
      <svg className="wordmark-mark" viewBox="0 0 32 32" width="24" height="24" aria-hidden="true">
        <path d="M5 25c8 0 12.5-2.4 15.7-7.2S25 9 26 5" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" />
        <circle cx="26" cy="5" r="3" fill="currentColor" />
      </svg>
    </a>
  );
}

/** The bar at the top of every screen. From tablet width up it also holds the places to go. */
export function TopBar({ items, path, end }: { items: NavItem[]; path: string; end?: ReactNode }) {
  return (
    <header className="top-bar">
      <div className="top-bar-inner">
        <Wordmark />
        <nav className="top-bar-nav" aria-label="Main">
          {items.map((item) => (
            <a key={item.href} className="top-bar-link" href={item.href} aria-current={current(item, path)} onClick={followLink}>
              {item.label}
            </a>
          ))}
        </nav>
        {end !== undefined && <div className="top-bar-end">{end}</div>}
      </div>
    </header>
  );
}

/** The bar at the bottom of a phone screen, in reach of a thumb. */
export function BottomBar({ items, path }: { items: NavItem[]; path: string }) {
  return (
    <nav className="bottom-bar" aria-label="Main">
      {items.map((item) => (
        <a key={item.href} className="tab" href={item.href} aria-current={current(item, path)} onClick={followLink}>
          <span className="tab-mark">
            <Icon name={item.icon} />
            {item.badge && <span className="tab-badge" aria-hidden="true" />}
          </span>
          <span>{item.label}</span>
          {item.badge && <span className="sr-only">, something new</span>}
        </a>
      ))}
    </nav>
  );
}
