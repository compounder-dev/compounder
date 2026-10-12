import type { ReactNode } from 'react';
import { followLink } from '../route.ts';
import { Icon, type IconName } from './Icon.tsx';

type LinkProps = {
  href: string;
  /** Inline sits in a sentence and is always underlined. Standalone is a line by itself with room to tap. */
  variant?: 'inline' | 'standalone';
  icon?: IconName;
  children: ReactNode;
};

export function Link({ href, variant = 'inline', icon, children }: LinkProps) {
  return (
    <a className={`link link-${variant}`} href={href} onClick={followLink}>
      {icon && <Icon name={icon} />}
      {children}
    </a>
  );
}
