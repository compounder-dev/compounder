import { useEffect, useState, type ComponentType } from 'react';
import { Banner, Button } from '../parts/index.ts';
import { useTitle } from '../route.ts';

/**
 * The builder's place. Its own code is the largest in the app after the engine, so it is
 * fetched only when this screen is opened, and the page says what it is for meanwhile.
 */
export function Build() {
  useTitle('Build a fund · Compounder');
  const [Builder, setBuilder] = useState<ComponentType | null>(null);
  const [failed, setFailed] = useState(false);
  const [tries, setTries] = useState(0);
  useEffect(() => {
    let current = true;
    setFailed(false);
    import('../builder/Builder.tsx').then(
      (module) => current && setBuilder(() => module.Builder),
      () => current && setFailed(true),
    );
    return () => {
      current = false;
    };
  }, [tries]);

  if (Builder) return <Builder />;
  return (
    <>
      <h1 className="page-title">Build a fund</h1>
      <p className="page-lead">Add what it holds and the rules it follows. It is tested on real prices as you go.</p>
      {failed ? (
        <div className="region-notice">
          <Banner variant="error" action={<Button onClick={() => setTries((count) => count + 1)}>Try again</Button>}>
            The builder could not be loaded.
          </Banner>
        </div>
      ) : (
        <div className="builder-waiting" aria-busy="true">
          <span className="skeleton skeleton-chart" aria-hidden="true" />
        </div>
      )}
    </>
  );
}
