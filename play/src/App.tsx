import { Asset } from './Asset.tsx';
import { Home } from './Fund.tsx';
import { Markets } from './Markets.tsx';
import { Link } from './parts.tsx';
import { match, usePath } from './route.ts';
import { Strategies, StrategyPage } from './Strategy.tsx';
import { You } from './You.tsx';

const TABS = [
  { to: '/', label: 'Fund', on: ['fund'], icon: 'M4 19V9l8-5 8 5v10M9 19v-6h6v6' },
  { to: '/strategies', label: 'Strategies', on: ['strategies', 'strategy'], icon: 'M4 17l5-5 4 4 7-7M15 9h5v5' },
  { to: '/markets', label: 'Markets', on: ['markets', 'asset'], icon: 'M4 6h16M4 12h16M4 18h10' },
  { to: '/you', label: 'You', on: ['you'], icon: 'M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4 20c1.5-3.5 4.5-5 8-5s6.5 1.5 8 5' },
];

export function App() {
  const route = match(usePath());
  const top = route.screen === 'fund' || route.screen === 'you' || route.screen === 'strategies' || route.screen === 'markets';
  return (
    <>
      <main className={`screen screen-${route.screen}`}>
        {route.screen === 'fund' && <Home />}
        {route.screen === 'you' && <You />}
        {route.screen === 'markets' && <Markets />}
        {route.screen === 'asset' && <Asset key={route.market} id={route.market} />}
        {route.screen === 'strategies' && <Strategies />}
        {route.screen === 'strategy' && <StrategyPage key={route.id} id={route.id} />}
      </main>
      {top && (
        <nav className="tabs" aria-label="Main">
          {TABS.map((tab) => (
            <Link key={tab.to} to={tab.to} className={`tab${tab.on.includes(route.screen) ? ' tab-on' : ''}`}>
              <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                <path d={tab.icon} />
              </svg>
              <span>{tab.label}</span>
            </Link>
          ))}
        </nav>
      )}
    </>
  );
}
