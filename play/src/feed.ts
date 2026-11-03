// Live prices straight from the venue: every market's mid as it moves, and
// candles for the market on screen. One socket for the whole app.

import { useEffect, useSyncExternalStore } from 'react';

const WS = 'wss://api.hyperliquid.xyz/ws';
const INFO = 'https://api.hyperliquid.xyz/info';

export type Candle = { time: number; open: number; high: number; low: number; close: number };

const mids = new Map<string, number>();
const listeners = new Set<() => void>();
const candleListeners = new Map<string, Set<(c: Candle) => void>>();
let socket: WebSocket | null = null;
let version = 0;
const wanted = new Set<string>();

function send(message: unknown) {
  if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify(message));
}

function connect() {
  socket = new WebSocket(WS);
  socket.onopen = () => {
    send({ method: 'subscribe', subscription: { type: 'allMids' } });
    send({ method: 'subscribe', subscription: { type: 'allMids', dex: 'xyz' } });
    for (const key of wanted) {
      const [coin, interval] = key.split('|');
      send({ method: 'subscribe', subscription: { type: 'candle', coin, interval } });
    }
  };
  socket.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.channel === 'allMids') {
      for (const [coin, px] of Object.entries(message.data.mids as Record<string, string>)) {
        if (!coin.startsWith('#') && !coin.startsWith('@')) mids.set(coin, Number(px));
      }
      version += 1;
      listeners.forEach((listener) => listener());
    } else if (message.channel === 'candle') {
      const c = message.data;
      const candle = { time: Math.floor(c.t / 1000), open: +c.o, high: +c.h, low: +c.l, close: +c.c };
      candleListeners.get(`${c.s}|${c.i}`)?.forEach((listener) => listener(candle));
    }
  };
  // A dropped socket comes back by itself.
  socket.onclose = () => setTimeout(connect, 1000);
}

function subscribe(listener: () => void) {
  if (!socket) connect();
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

/** One market's live mid. */
export function useMid(coin: string): number | undefined {
  useSyncExternalStore(subscribe, () => version);
  return mids.get(coin);
}

/** Every live mid; the component draws again when any moves. */
export function useMids(): Map<string, number> {
  useSyncExternalStore(subscribe, () => version);
  return mids;
}

export const INTERVALS = { '1m': 60, '5m': 300, '15m': 900, '1h': 3600, '4h': 14400, '1d': 86400 } as const;
export type Interval = keyof typeof INTERVALS;

/** Past candles for a market, then each new one as it forms. */
export function useCandles(coin: string, interval: Interval, count: number, onCandle: (c: Candle) => void, onHistory: (c: Candle[]) => void) {
  useEffect(() => {
    let live = true;
    const key = `${coin}|${interval}`;
    const end = Date.now();
    const start = end - INTERVALS[interval] * 1000 * count;
    fetch(INFO, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ type: 'candleSnapshot', req: { coin, interval, startTime: start, endTime: end } }),
    })
      .then((r) => r.json())
      .then((rows: { t: number; o: string; h: string; l: string; c: string }[]) => {
        if (live) onHistory(rows.map((c) => ({ time: Math.floor(c.t / 1000), open: +c.o, high: +c.h, low: +c.l, close: +c.c })));
      })
      .catch(() => {});
    if (!socket) connect();
    let set = candleListeners.get(key);
    if (!set) candleListeners.set(key, (set = new Set()));
    set.add(onCandle);
    wanted.add(key);
    send({ method: 'subscribe', subscription: { type: 'candle', coin, interval } });
    return () => {
      live = false;
      set!.delete(onCandle);
      if (set!.size === 0) {
        wanted.delete(key);
        send({ method: 'unsubscribe', subscription: { type: 'candle', coin, interval } });
      }
    };
    // The callbacks are the caller's to keep steady.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [coin, interval, count]);
}
