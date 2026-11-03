import { useCallback, useEffect, useRef } from 'react';
import { AreaSeries, ColorType, createChart, LineStyle, type IChartApi, type ISeriesApi, type UTCTimestamp } from 'lightweight-charts';
import { INTERVALS, useCandles, useMid, type Candle, type Interval } from './feed.ts';

/** The ranges a chart can show: the candle each uses and how many. */
export const RANGES = {
  Live: { interval: '1m', count: 90 },
  '1D': { interval: '5m', count: 288 },
  '1W': { interval: '1h', count: 168 },
  '1M': { interval: '4h', count: 180 },
  '1Y': { interval: '1d', count: 365 },
} as const satisfies Record<string, { interval: Interval; count: number }>;
export type Range = keyof typeof RANGES;

/** A live price chart: past candles, then every tick of the mid. Coloured by the move over the range. */
export function Chart({ coin, range, entry, bust, onFirst }: { coin: string; range: Range; entry?: number; bust?: number; onFirst?: (price: number) => void }) {
  const box = useRef<HTMLDivElement>(null);
  const chart = useRef<IChartApi | null>(null);
  const series = useRef<ISeriesApi<'Area'> | null>(null);
  const first = useRef<number | null>(null);
  const lines = useRef<ReturnType<ISeriesApi<'Area'>['createPriceLine']>[]>([]);
  const { interval, count } = RANGES[range];
  const mid = useMid(coin);

  const paint = (up: boolean) => {
    series.current?.applyOptions({
      lineColor: up ? '#00ec4b' : '#ff4444',
      topColor: up ? 'rgba(0,236,75,0.22)' : 'rgba(255,68,68,0.2)',
      bottomColor: 'rgba(0,0,0,0)',
    });
  };

  useEffect(() => {
    if (!box.current) return;
    chart.current = createChart(box.current, {
      autoSize: true,
      layout: { background: { type: ColorType.Solid, color: 'transparent' }, textColor: '#757575', attributionLogo: false, fontFamily: 'Inter Variable, system-ui', fontSize: 11 },
      grid: { vertLines: { visible: false }, horzLines: { visible: false } },
      rightPriceScale: { visible: false },
      leftPriceScale: { visible: false },
      timeScale: { visible: false, borderVisible: false },
      crosshair: { horzLine: { visible: false, labelVisible: false }, vertLine: { color: 'rgba(255,255,255,0.2)', labelVisible: false, style: LineStyle.Solid } },
      handleScale: false,
      handleScroll: false,
    });
    series.current = chart.current.addSeries(AreaSeries, { lineWidth: 2, priceLineVisible: false, lastValueVisible: false, crosshairMarkerRadius: 4 });
    paint(true);
    return () => {
      chart.current?.remove();
      chart.current = null;
      series.current = null;
    };
  }, []);

  const onHistory = useCallback(
    (candles: Candle[]) => {
      if (!candles.length) return;
      first.current = candles[0].open;
      onFirst?.(candles[0].open);
      series.current?.setData(candles.map((c) => ({ time: c.time as UTCTimestamp, value: c.close })));
      paint(candles[candles.length - 1].close >= candles[0].open);
      chart.current?.timeScale().fitContent();
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );
  const onCandle = useCallback((c: Candle) => series.current?.update({ time: c.time as UTCTimestamp, value: c.close }), []);
  useCandles(coin, interval, count, onCandle, onHistory);

  // Every tick of the mid moves the last point, so the line is never a minute behind.
  useEffect(() => {
    if (!mid || !series.current || first.current === null) return;
    const step = INTERVALS[interval];
    const time = (Math.floor(Date.now() / 1000 / step) * step) as UTCTimestamp;
    try {
      series.current.update({ time, value: mid });
    } catch {
      // A tick older than the last candle is dropped.
    }
    paint(mid >= first.current);
  }, [mid, interval]);

  useEffect(() => {
    const s = series.current;
    if (!s) return;
    lines.current.forEach((line) => s.removePriceLine(line));
    lines.current = [];
    if (entry) lines.current.push(s.createPriceLine({ price: entry, color: 'rgba(255,255,255,0.5)', lineStyle: LineStyle.Dashed, lineWidth: 1, axisLabelVisible: false, title: 'Entry' }));
    if (bust) lines.current.push(s.createPriceLine({ price: bust, color: '#ff4444', lineStyle: LineStyle.Dashed, lineWidth: 1, axisLabelVisible: false, title: 'Bust' }));
  }, [entry, bust]);

  return <div className="chart" ref={box} />;
}
