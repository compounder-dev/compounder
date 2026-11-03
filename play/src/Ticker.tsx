import { useEffect, useRef, useState, type ReactNode } from 'react';

/** A number that flashes green or red for a moment when it moves. */
export function Ticker({ value, text, className }: { value: number; text: ReactNode; className?: string }) {
  const last = useRef(value);
  const [flash, setFlash] = useState<'up' | 'down' | null>(null);
  useEffect(() => {
    if (value === last.current) return;
    setFlash(value > last.current ? 'up' : 'down');
    last.current = value;
    const timer = window.setTimeout(() => setFlash(null), 450);
    return () => window.clearTimeout(timer);
  }, [value]);
  return <span className={`${className ?? ''}${flash ? ` flash-${flash}` : ''}`}>{text}</span>;
}
