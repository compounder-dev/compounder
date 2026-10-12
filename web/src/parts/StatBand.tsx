import type { ReactNode } from 'react';

/** A few figures that lead a page, each under its name, in a fixed order. */
export function StatBand({ items }: { items: { label: string; value: ReactNode }[] }) {
  return (
    <dl className="stat-band">
      {items.map((item) => (
        <div key={item.label}>
          <dt>{item.label}</dt>
          <dd>{item.value}</dd>
        </div>
      ))}
    </dl>
  );
}
