// An onchain account read straight from the venue: its value over time, its
// open positions on every market list, and its fills. Anyone can check it.

import { useEffect, useState } from 'react';

const host = (testnet: boolean) => (testnet ? 'hyperliquid-testnet.xyz' : 'hyperliquid.xyz');
export const explorer = (address: string, testnet = false) => `https://app.${host(testnet)}/explorer/address/${address}`;

export type Period = 'day' | 'week' | 'month' | 'allTime';
export type Position = { coin: string; size: number; entry: number; leverage: number; liquidation: number | null; margin: number; pnl: number; value: number };
export type Fill = { coin: string; price: number; size: number; buy: boolean; time: number; dir: string; pnl: number; hash: string };
export type VenueAccount = {
  history: Record<Period, { value: [number, number][]; pnl: [number, number][] }>;
  positions: Position[];
  fills: Fill[];
};

const ask = <T,>(testnet: boolean, body: unknown) => fetch(`https://api.${host(testnet)}/info`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) }).then((r) => r.json() as Promise<T>);
const pairs = (rows: [number, string][]) => rows.map(([t, v]) => [t, Number(v)] as [number, number]);

type State = { assetPositions: { position: { coin: string; szi: string; entryPx: string; leverage: { value: number }; liquidationPx: string | null; marginUsed: string; unrealizedPnl: string; positionValue: string } }[] };

async function read(user: string, testnet: boolean): Promise<VenueAccount> {
  const [portfolio, main, stocks, fills] = await Promise.all([
    ask<[Period | string, { accountValueHistory: [number, string][]; pnlHistory: [number, string][] }][]>(testnet, { type: 'portfolio', user }),
    ask<State>(testnet, { type: 'clearinghouseState', user }),
    ask<State>(testnet, { type: 'clearinghouseState', user, dex: 'xyz' }),
    ask<{ coin: string; px: string; sz: string; side: 'B' | 'A'; time: number; dir: string; closedPnl: string; hash: string }[]>(testnet, { type: 'userFills', user }),
  ]);
  const byPeriod = Object.fromEntries(portfolio.map(([k, v]) => [k, { value: pairs(v.accountValueHistory), pnl: pairs(v.pnlHistory) }]));
  return {
    history: byPeriod as VenueAccount['history'],
    positions: [...main.assetPositions, ...stocks.assetPositions].map(({ position: p }) => ({
      coin: p.coin,
      size: Number(p.szi),
      entry: Number(p.entryPx),
      leverage: p.leverage.value,
      liquidation: p.liquidationPx ? Number(p.liquidationPx) : null,
      margin: Number(p.marginUsed),
      pnl: Number(p.unrealizedPnl),
      value: Number(p.positionValue),
    })),
    // Spot swaps are not the fund's trades.
    fills: fills
      .filter((f) => !f.coin.startsWith('@') && !f.coin.includes('/'))
      .slice(0, 40)
      .map((f) => ({ coin: f.coin, price: Number(f.px), size: Number(f.sz), buy: f.side === 'B', time: f.time, dir: f.dir, pnl: Number(f.closedPnl), hash: f.hash })),
  };
}

/** An onchain account, read again every few seconds. */
export function useVenueAccount(address: string | null, testnet = false): VenueAccount | null {
  const [account, setAccount] = useState<VenueAccount | null>(null);
  useEffect(() => {
    if (!address) return;
    let live = true;
    const load = () => read(address, testnet).then((a) => live && setAccount(a)).catch(() => {});
    load();
    const timer = window.setInterval(load, 8_000);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, [address, testnet]);
  return account;
}

export const symbolOf = (coin: string) => coin.split(':').pop() ?? coin;
export const shortAddress = (a: string) => `${a.slice(0, 6)}…${a.slice(-4)}`;
