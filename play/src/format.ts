const money = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', minimumFractionDigits: 2, maximumFractionDigits: 2 });

export const usd = (n: number) => money.format(n);

/** A price with as many decimals as it needs to move. */
export function price(n: number | null | undefined): string {
  if (n == null || !Number.isFinite(n)) return '–';
  const digits = n >= 1000 ? 2 : n >= 1 ? 2 : n >= 0.01 ? 4 : 6;
  return n.toLocaleString('en-US', { minimumFractionDigits: digits, maximumFractionDigits: digits });
}

export function pct(fraction: number | null | undefined, sign = true): string {
  if (fraction == null || !Number.isFinite(fraction)) return '–';
  const text = `${Math.abs(fraction * 100).toFixed(2)}%`;
  if (!sign) return text;
  return fraction > 0 ? `+${text}` : fraction < 0 ? `−${text}` : text;
}

export function signedUsd(n: number): string {
  const text = money.format(Math.abs(n));
  return n > 0 ? `+${text}` : n < 0 ? `−${text}` : text;
}
