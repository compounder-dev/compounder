import type { ReactNode } from 'react';
import type { FundColour } from '../api.ts';
import { followLink } from '../route.ts';

type CardProps = {
  /** Plain holds anything. Strategy carries a wash of its fund colour. Stat holds one labelled number. */
  variant?: 'plain' | 'strategy' | 'stat';
  /** The identity colour of the fund or strategy the card is about. */
  fund?: FundColour;
  as?: 'div' | 'li' | 'article' | 'aside' | 'section';
  className?: string;
  label?: string;
  children: ReactNode;
};

export function Card({ variant = 'plain', fund, as: Tag = 'div', className, label, children }: CardProps) {
  const classes = ['card', `card-${variant}`, fund && `fund-${fund}`, className].filter(Boolean).join(' ');
  return (
    <Tag className={classes} aria-label={label}>
      {children}
    </Tag>
  );
}

/**
 * The title of a card that opens something. The link is the one control a keyboard
 * and a screen reader meet, and it is stretched over the card so a tap anywhere opens it.
 */
export function CardTitle({ href, dot = false, children }: { href?: string; dot?: boolean; children: ReactNode }) {
  return (
    <h2 className="card-title">
      {dot && <span className="fund-dot" aria-hidden="true" />}
      {href ? (
        <a className="card-link" href={href} onClick={followLink}>
          {children}
        </a>
      ) : (
        children
      )}
    </h2>
  );
}
