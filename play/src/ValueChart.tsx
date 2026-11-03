import { useEffect, useRef } from 'react';
import { AreaSeries, ColorType, createChart, LineStyle, type IChartApi, type ISeriesApi, type UTCTimestamp } from 'lightweight-charts';

/** A fund's value over time, green when above where it started, red below. */
export function ValueChart({ points, start, height = 200 }: { points: [number, number][]; start: number; height?: number }) {
  const box = useRef<HTMLDivElement>(null);
  const chart = useRef<IChartApi | null>(null);
  const series = useRef<ISeriesApi<'Area'> | null>(null);
  const startRef = useRef(start);
  startRef.current = start;
  useEffect(() => {
    if (!box.current) return;
    chart.current = createChart(box.current, {
      autoSize: true,
      layout: { background: { type: ColorType.Solid, color: 'transparent' }, textColor: '#757575', attributionLogo: false },
      grid: { vertLines: { visible: false }, horzLines: { visible: false } },
      rightPriceScale: { visible: false },
      timeScale: { visible: false },
      crosshair: { horzLine: { visible: false, labelVisible: false }, vertLine: { color: 'rgba(255,255,255,0.2)', labelVisible: false, style: LineStyle.Solid } },
      handleScale: false,
      handleScroll: false,
    });
    series.current = chart.current.addSeries(AreaSeries, {
      lineWidth: 2,
      priceLineVisible: false,
      lastValueVisible: false,
      // The axis spans at least 1% either side of the start, so pennies of noise read as flat.
      autoscaleInfoProvider: (original: () => { priceRange: { minValue: number; maxValue: number } } | null) => {
        const found = original();
        const floor = startRef.current * 0.01;
        if (!found) return found;
        return {
          ...found,
          priceRange: {
            minValue: Math.min(found.priceRange.minValue, startRef.current - floor),
            maxValue: Math.max(found.priceRange.maxValue, startRef.current + floor),
          },
        };
      },
    });
    return () => chart.current?.remove();
  }, []);
  useEffect(() => {
    const s = series.current;
    if (!s) return;
    const byTime = new Map<number, number>();
    for (const [t, v] of points) byTime.set(Math.floor(t / 1000), v);
    const data = [...byTime].sort((a, b) => a[0] - b[0]).map(([time, value]) => ({ time: time as UTCTimestamp, value }));
    const last = data[data.length - 1]?.value ?? start;
    const tone = Math.abs(last - start) < Math.max(0.005, start * 0.0005) ? 'flat' : last > start ? 'up' : 'down';
    const colours = { up: ['#00ec4b', 'rgba(0,236,75,0.2)'], down: ['#ff4444', 'rgba(255,68,68,0.18)'], flat: ['#757575', 'rgba(117,117,117,0.12)'] }[tone];
    s.applyOptions({ lineColor: colours[0], topColor: colours[1], bottomColor: 'rgba(0,0,0,0)' });
    s.setData(data);
    s.priceScale().applyOptions({ scaleMargins: { top: 0.15, bottom: 0.1 } });
    chart.current?.timeScale().fitContent();
  }, [points, start]);
  return <div className="chart" style={{ height }} ref={box} />;
}
