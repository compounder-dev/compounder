// The one formatter for every number a person reads. The locale is fixed here:
// a comma between thousands and a point before decimals, whatever the browser prefers.
// The server prints the same text for the first response, so the rounding is
// spelled out in plain arithmetic that both sides can do alike.

// A true minus sign, as wide as a plus and read aloud as "minus".
const MINUS = '\u2212';

/** A value that is not a finite number has no text. The Number part shows a dash for it. */
export type Formatted = string | null;

/** A whole number of `10^-decimals` units, halves rounded up. `amount` is never negative. */
function scaled(amount: number, decimals: number): number {
  return Math.floor(amount * 10 ** decimals + 0.5);
}

/** `units` of `10^-decimals` written out: 124018 at 2 becomes "1,240.18". */
function written(units: number, decimals: number): string {
  const digits = String(units).padStart(decimals + 1, '0');
  const whole = digits.slice(0, digits.length - decimals).replace(/\B(?=(\d{3})+$)/g, ',');
  return decimals ? `${whole}.${digits.slice(-decimals)}` : whole;
}

function signed(text: string, value: number, isZero: boolean, always: boolean): string {
  if (isZero) return text;
  if (value < 0) return MINUS + text;
  return always ? `+${text}` : text;
}

/**
 * A share or a change, given as a fraction: 0.0034 becomes "0.34%".
 * Two decimals under 10, one from 10, none from 1,000. Zero is "0%", never negative.
 * `sign` puts a plus on a rise, for a change.
 */
export function formatPercent(fraction: number, options: { sign?: boolean } = {}): Formatted {
  if (!Number.isFinite(fraction)) return null;
  const amount = Math.abs(fraction) * 100;
  let decimals = 2;
  let units = scaled(amount, 2);
  if (units >= 1000) {
    decimals = 1;
    units = scaled(amount, 1);
    if (units >= 10000) {
      decimals = 0;
      units = scaled(amount, 0);
    }
  }
  if (units === 0) return '0%';
  return signed(`${written(units, decimals)}%`, fraction, false, options.sign ?? false);
}

const LARGE: [number, string][] = [
  [1e6, 'M'],
  [1e9, 'B'],
  [1e12, 'T'],
];

/** Dollars: "$1,240.18". Under a cent is "<$0.01". From a million, "$1.24M". */
export function formatMoney(dollars: number, options: { sign?: boolean } = {}): Formatted {
  if (!Number.isFinite(dollars)) return null;
  const amount = Math.abs(dollars);
  const always = options.sign ?? false;
  if (amount === 0) return '$0.00';
  const cents = scaled(amount, 2);
  if (cents === 0) return signed('<$0.01', dollars, false, always);
  if (cents < 1e8) return signed(`$${written(cents, 2)}`, dollars, false, always);
  // The first letter under which the figure stays below 1,000 after rounding.
  const [size, letter] = LARGE.find(([each]) => scaled(amount / each, 2) < 100000) ?? LARGE[LARGE.length - 1];
  return signed(`$${written(scaled(amount / size, 2), 2)}${letter}`, dollars, false, always);
}

/** A ratio with both decimals always shown: "1.50". */
export function formatRatio(ratio: number): Formatted {
  if (!Number.isFinite(ratio)) return null;
  const units = scaled(Math.abs(ratio), 2);
  return signed(written(units, 2), ratio, units === 0, false);
}

/** A count or an amount of an asset: up to four decimals, and no zeros after the last digit that counts. */
export function formatQuantity(quantity: number): Formatted {
  if (!Number.isFinite(quantity)) return null;
  const units = scaled(Math.abs(quantity), 4);
  const text = written(units, 4).replace(/\.?0+$/, '');
  return signed(text, quantity, units === 0, false);
}

/** Time left, in seconds: "5h 12m" above an hour, "12m 08s" under it. */
export function formatCountdown(seconds: number): Formatted {
  if (!Number.isFinite(seconds)) return null;
  const left = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(left / 3600);
  const minutes = Math.floor((left % 3600) / 60);
  if (hours > 0) return `${hours}h ${String(minutes).padStart(2, '0')}m`;
  return `${minutes}m ${String(left % 60).padStart(2, '0')}s`;
}

export type NumberKind = 'money' | 'percent' | 'ratio' | 'quantity' | 'countdown';

export function formatNumber(kind: NumberKind, value: number | null | undefined, options: { sign?: boolean } = {}): Formatted {
  if (value === null || value === undefined) return null;
  switch (kind) {
    case 'money':
      return formatMoney(value, options);
    case 'percent':
      return formatPercent(value, options);
    case 'ratio':
      return formatRatio(value);
    case 'quantity':
      return formatQuantity(value);
    case 'countdown':
      return formatCountdown(value);
  }
}
