import type { CSSProperties, ReactNode } from 'react';
import { followLink } from '../route.ts';
import { Icon } from './Icon.tsx';

type RowProps = {
  /** What the row is about. Each kind fills the same places: a mark, two lines of text, something at the end. */
  variant?: 'asset' | 'fund' | 'activity' | 'method' | 'setting';
  leading?: ReactNode;
  title: ReactNode;
  /** The second line. */
  detail?: ReactNode;
  /** A number or a control at the end. */
  trailing?: ReactNode;
  /** A row that goes somewhere is one link with a chevron. */
  href?: string;
  /** A row that opens in place says whether it is open. */
  open?: boolean;
  onToggle?: () => void;
  as?: 'li' | 'div';
};

/** One thing in a list: a filled block, never a line of a table. */
export function Row({ variant = 'asset', leading, title, detail, trailing, href, open, onToggle, as: Tag = 'li' }: RowProps) {
  const inside = (
    <>
      {leading}
      <span className="row-text">
        <span className="row-title">{title}</span>
        {detail !== undefined && <span className="row-detail">{detail}</span>}
      </span>
      {trailing !== undefined && <span className="row-trailing">{trailing}</span>}
      {(href !== undefined || onToggle) && <Icon name="chevron-right" />}
    </>
  );
  const classes = `row row-${variant}`;
  if (href !== undefined) {
    return (
      <Tag className="row-slot">
        <a className={`${classes} row-action`} href={href} onClick={followLink}>
          {inside}
        </a>
      </Tag>
    );
  }
  if (onToggle) {
    return (
      <Tag className="row-slot">
        <button type="button" className={`${classes} row-action`} aria-expanded={open ?? false} onClick={onToggle}>
          {inside}
        </button>
      </Tag>
    );
  }
  return <Tag className={classes}>{inside}</Tag>;
}

/** The round mark at the start of a row. Until assets have marks of their own it holds one letter. */
export function Mark({ letter }: { letter: string }) {
  return (
    <span className="mark" aria-hidden="true">
      {letter}
    </span>
  );
}

/** A holding's share as a bar in the fund's colour. `percent` is 0 to 100, a whole number. */
export function WeightBar({ percent }: { percent: number }) {
  return (
    <span className="bar" aria-hidden="true">
      <span style={{ '--share': percent } as CSSProperties} />
    </span>
  );
}
