// A swap the wallet signs: cash in, tokenised stock out, and the token lands in
// the wallet's own address. Routes come from KyberSwap's aggregator on BNB Chain;
// nothing here is trusted by the app, the router does the trade and the chain
// checks the result.

const API = 'https://aggregator-api.kyberswap.com/bsc/api/v1';
const HEADERS = { accept: 'application/json', 'x-client-id': 'compounder' } as const;

export type Quote = {
  /** What the route pays, in the token's own units. */
  out: bigint;
  outUsd: number;
  gasUsd: number;
  /** The contract the wallet must let take the cash. */
  router: string;
  /** The call that does the swap. */
  data: string;
  value: bigint;
};

type Answer<T> = T & { code?: number; message?: string };

async function ask<T>(path: string, init?: RequestInit): Promise<Answer<T>> {
  const response = await fetch(`${API}${path}`, { ...init, headers: { ...HEADERS, ...(init?.body ? { 'content-type': 'application/json' } : {}) } });
  const body = (await response.json().catch(() => null)) as Answer<T> | null;
  if (!response.ok || !body) throw new Error(body?.message || 'The swap service did not answer.');
  if (body.code !== 0) throw new Error(body.message || 'The swap service refused this trade.');
  return body;
}

/** The best route for exactly this much cash, built for this wallet to pay from and receive. */
export async function quote(tokenIn: string, tokenOut: string, amountIn: bigint, taker: string, slippageBps = 50): Promise<Quote> {
  const query = new URLSearchParams({ tokenIn, tokenOut, amountIn: amountIn.toString() });
  const found = await ask<{ data?: { routeSummary?: { amountOutUsd?: string }; gasUsd?: string } }>(`/routes?${query}`);
  const summary = found.data?.routeSummary;
  if (!summary) throw new Error('No route between these two right now.');

  const built = await ask<{ data?: { amountOut: string; routerAddress: string; data: string; transactionValue: string; gasUsd?: string } }>('/route/build', {
    method: 'POST',
    body: JSON.stringify({ routeSummary: summary, sender: taker, recipient: taker, slippageTolerance: slippageBps }),
  });
  if (!built.data?.data) throw new Error('The trade could not be built.');

  return {
    out: BigInt(built.data.amountOut),
    outUsd: Number(found.data?.routeSummary?.amountOutUsd ?? 0),
    gasUsd: Number(built.data.gasUsd ?? found.data?.gasUsd ?? 0),
    router: built.data.routerAddress,
    data: built.data.data,
    value: BigInt(built.data.transactionValue || '0'),
  };
}
