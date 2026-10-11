export type Holding = {
  asset_id: string;
  symbol: string | null;
  name: string | null;
  weight: number;
};

export type Accent = 'amber' | 'blue' | 'green';

export type Starter = {
  id: string;
  name: string;
  summary: string;
  accent: Accent;
  /** The full strategy document. Only the trading settings are read here; the engine reads the rest. */
  strategy: { tradingSettings: { type: 'threshold'; triggerPercentage: number } | { type: 'calendar'; frequency: string } };
  holdings: Holding[];
};

/** A failed request, with a message fit to show a person. */
export class ApiError extends Error {}

export async function fetchStarters(signal?: AbortSignal): Promise<Starter[]> {
  let response: Response;
  try {
    response = await fetch('/v1/starters', { headers: { Accept: 'application/json' }, signal });
  } catch (cause) {
    if (signal?.aborted) throw cause;
    throw new ApiError('Could not reach the Compounder backend. Check that it is running.');
  }
  const body: unknown = await response.json().catch(() => null);
  if (!response.ok) {
    const message = (body as { error?: { message?: string } } | null)?.error?.message;
    throw new ApiError(message ?? `The backend answered with status ${response.status}.`);
  }
  const starters = (body as { starters?: Starter[] } | null)?.starters;
  if (!Array.isArray(starters)) throw new ApiError('The backend sent an answer this app does not understand.');
  return starters;
}

/** 0.142857 becomes "14.29%", 0.4 becomes "40%". */
export function percent(weight: number): string {
  return `${(weight * 100).toLocaleString('en-US', { maximumFractionDigits: 2 })}%`;
}

/** The ticker a person recognises: "NVDAon" becomes "NVDA". */
export function ticker(holding: Holding): string {
  return (holding.symbol ?? holding.asset_id.slice(0, 6)).replace(/on$/, '');
}

/** The company or fund name, without the issuer's suffix. */
export function displayName(holding: Holding): string {
  return (holding.name ?? holding.symbol ?? holding.asset_id).replace(/\s*\([^)]*\)\s*$/, '');
}

/** A stable hue per ticker, so a token keeps its colour everywhere. */
export function hue(label: string): number {
  let hash = 0;
  for (const char of label) hash = (hash * 31 + char.charCodeAt(0)) % 360;
  return hash;
}

/** The chain every holding sits on, or null when they differ. */
export function chainName(holdings: Holding[]): string | null {
  const chains = new Set(holdings.map((holding) => holding.asset_id.split(':')[1]));
  if (chains.size !== 1) return null;
  const names: Record<string, string> = { '1': 'Ethereum', '56': 'BNB Chain', '8453': 'Base', '42161': 'Arbitrum' };
  const [chain] = chains;
  return names[chain] ?? `chain ${chain}`;
}
