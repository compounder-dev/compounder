import { useEffect, useRef } from 'react';
import { AreaSeries, ColorType, createChart, LineStyle, type IChartApi, type ISeriesApi, type UTCTimestamp } from 'lightweight-charts';

/** A fund's value over time, green when above where it started, red below. */
export function ValueChart({ points, start, height = 200 }: { points: [number, number][]; start: number; height?: number }) {
  const box = useRef<HTMLDivElement>(null);
  const chart = useRef<IChartApi | null>(null);
  const series = useRef<ISeriesApi<'Area'> | null>(null);
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
    series.current = chart.current.addSeries(AreaSeries, { lineWidth: 2, priceLineVisible: false, lastValueVisible: false });
    return () => chart.current?.remove();
  }, []);
  useEffect(() => {
    const s = series.current;
    if (!s) return;
    const byTime = new Map<number, number>();
    for (const [t, v] of points) byTime.set(Math.floor(t / 1000), v);
    const data = [...byTime].sort((a, b) => a[0] - b[0]).map(([time, value]) => ({ time: time as UTCTimestamp, value }));
    const up = (data[data.length - 1]?.value ?? start) >= start;
    s.applyOptions({ lineColor: up ? '#00ec4b' : '#ff4444', topColor: up ? 'rgba(0,236,75,0.2)' : 'rgba(255,68,68,0.18)', bottomColor: 'rgba(0,0,0,0)' });
    s.setData(data);
    s.priceScale().applyOptions({ scaleMargins: { top: 0.15, bottom: 0.1 } });
    chart.current?.timeScale().fitContent();
  }, [points, start]);
  return <div className="chart" style={{ height }} ref={box} />;
}
