import { Card } from './Card.tsx';

/**
 * A placeholder for one line of text. It takes the line height of the text it
 * stands in for, so content arrives without the page moving.
 */
export function Skeleton({ width = 'full' }: { width?: 'short' | 'half' | 'long' | 'full' }) {
  return <span className={`skeleton skeleton-${width}`} aria-hidden="true" />;
}

/** A strategy card that has not arrived: a title, two lines and the line of what it holds. */
export function CardSkeleton() {
  return (
    <Card variant="strategy" as="li">
      <h2 className="card-title">
        <Skeleton width="long" />
      </h2>
      <p className="card-summary">
        <Skeleton />
        <Skeleton width="half" />
      </p>
      <dl className="card-facts">
        <div>
          <dt>
            <Skeleton width="short" />
          </dt>
          <dd>
            <Skeleton width="long" />
          </dd>
        </div>
      </dl>
    </Card>
  );
}

/** A row that has not arrived: a mark, two lines and a number. */
export function RowSkeleton() {
  return (
    <li className="row">
      <span className="mark skeleton" aria-hidden="true" />
      <span className="row-text">
        <span className="row-title">
          <Skeleton width="half" />
        </span>
        <span className="row-detail">
          <Skeleton width="short" />
        </span>
      </span>
      <span className="row-trailing">
        <span className="number skeleton skeleton-number" aria-hidden="true" />
      </span>
    </li>
  );
}
