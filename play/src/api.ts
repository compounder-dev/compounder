// The account and the markets, from our server. Every trade is priced there.

import { useCallback, useEffect, useState } from 'react';

export type Market = { id: string; symbol: string; name: string; kind: 'stocks' | 'commodities' | 'indices' | 'crypto'; price: number | null; change_day: number | null; max_multiplier: number };

export type Fund = {
  id: string;
  name: string;
  speed: 'velocity' | 'compounding';
  thesis: string;
  rules: string;
  model?: string;
  markets: string[];
  capital: number;
  equity: number;
  change: number;
  change_day: number | null;
  worst_drop: number;
  fills: number;
  curve: [number, number][];
  note: string;
  decided: number | null;
  holdings: { market: string; weight: number }[];
  test?: { start: number; end: number; annual_return: number; sharpe: number; max_drawdown: number };
  verified: boolean;
  entries: number;
  head: string;
};

export type Trade = {
  id: string;
  market: string;
  side: 1 | -1;
  stake: number;
  multiplier: number;
  entry: number;
  fee: number;
  opened: number;
  take_profit?: number;
  stop_loss?: number;
  close?: { price: number; at: number; pnl: number; reason: 'closed' | 'bust' | 'take_profit' | 'stop_loss' };
};

export type Me = {
  name: string;
  fund: string;
  temperament: string;
  curve: [number, number][];
  balance: number;
  equity: number;
  start: number;
  open: { trade: Trade; symbol: string; price: number | null; pnl: number | null; bust: number }[];
  history: { trade: Trade; symbol: string }[];
  backed: { fund: string; name: string; units: number; cost: number; value: number }[];
};

async function call<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(path, {
    method: body === undefined ? 'GET' : 'POST',
    headers: body === undefined ? {} : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (response.status === 401) {
    window.location.href = `/login?next=${encodeURIComponent(window.location.pathname)}`;
    throw new Error('signed out');
  }
  const data = await response.json().catch(() => null);
  if (!response.ok) throw new Error(data?.error?.message ?? 'Something went wrong');
  return data as T;
}

/** A server answer that refreshes on an interval, and can be set from a mutation's answer. */
function usePolled<T>(path: string, every: number): [T | null, (t: T) => void, () => void] {
  const [data, setData] = useState<T | null>(null);
  const reload = useCallback(() => void call<T>(path).then(setData).catch(() => {}), [path]);
  useEffect(() => {
    reload();
    const timer = window.setInterval(reload, every);
    return () => window.clearInterval(timer);
  }, [reload, every]);
  return [data, setData, reload];
}

export const useMarkets = () => usePolled<{ markets: Market[]; funds: Fund[]; fee: number }>('/v1/markets', 30_000);
export const useMe = () => usePolled<Me>('/v1/me', 5_000);
export const useLeaderboard = () => usePolled<{ players: { name: string; equity: number; trades: number }[]; start: number }>('/v1/leaderboard', 30_000);

export const openTrade = (order: { market: string; side: 'up' | 'down'; stake: number; multiplier: number; take_profit?: number; stop_loss?: number }) =>
  call<Me>('/v1/trades', order);
export const closeTrade = (id: string) => call<Me>(`/v1/trades/${id}/close`, {});
export const startFund = (order: { name: string; amount: number; temperament: 'steady' | 'balanced' | 'bold' }) => call<Me>('/v1/fund/start', order);
export const backFund = (fund: string, amount: number) => call<Me>('/v1/back', { fund, amount });
export const addMoney = (amount: number) => call<Me>('/v1/fund/add', { amount });
