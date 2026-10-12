import { useEffect, useRef, useState } from 'react';
import { formatNumber, type NumberKind } from '../format.ts';
import { Icon } from './Icon.tsx';

type NumberProps = {
  kind: NumberKind;
  /** Percent takes a fraction, money takes dollars, countdown takes seconds. Null is "not known". */
  value: number | null;
  tone?: 'plain' | 'gain' | 'loss' | 'muted';
  /** The step of the type scale. Left out, the number takes the size of the text around it. */
  size?: 'caption' | 'small' | 'body' | 'title2' | 'title1';
  /** A plus on a rise, for a change. */
  sign?: boolean;
  /** An arrow before a change, so colour never carries it alone. A change that prints as zero has none. */
  arrow?: boolean;
  /** Proportional figures, for a large number that stands alone and does not tick. */
  proportional?: boolean;
  /** The value is on its way. */
  loading?: boolean;
};

/** Which way a value moved, for the wash behind it. */
export function movement(before: number | null, after: number | null): 'up' | 'down' | null {
  if (before === null || after === null || before === after) return null;
  return after > before ? 'up' : 'down';
}

/** Every number a person reads: one piece of text, in tabular figures, said as a number. */
export function Number({ kind, value, tone = 'plain', size, sign, arrow, proportional, loading }: NumberProps) {
  const previous = useRef(value);
  const [moved, setMoved] = useState<'up' | 'down' | null>(null);
  useEffect(() => {
    setMoved(movement(previous.current, value));
    previous.current = value;
  }, [value]);

  const classes = ['number', size && `number-${size}`, tone !== 'plain' && `number-${tone}`, proportional && 'number-proportional'].filter(Boolean).join(' ');
  if (loading) return <span className={`${classes} skeleton skeleton-number`} aria-hidden="true" />;
  const text = formatNumber(kind, value, { sign });
  if (text === null) {
    return (
      <span className={classes}>
        <span aria-hidden="true">{'\u2013'}</span>
        <span className="sr-only">not available</span>
      </span>
    );
  }
  return (
    <span className={moved ? `${classes} number-moved-${moved}` : classes} onAnimationEnd={() => setMoved(null)}>
      {arrow && value !== null && text !== formatNumber(kind, 0, { sign }) && <Icon name={value > 0 ? 'arrow-up' : 'arrow-down'} size="sm" />}
      {text}
    </span>
  );
}
