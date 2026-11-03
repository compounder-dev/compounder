import { useMe } from './api.ts';
import { Asset } from './Asset.tsx';
import { FundHome } from './Fund.tsx';
import { Markets } from './Home.tsx';
import { Link } from './parts.tsx';
import { match, usePath } from './route.ts';
import { Start } from './Start.tsx';
import { ManagerPage, Managers } from './Strategy.tsx';

const TABS = [
  { to: '/', label: 'Fund', on: ['fund'], icon: 'M4 19V9l8-5 8 5v10M9 19v-6h6v6' },
  { to: '/managers', label: 'Managers', on: ['managers', 'manager'], icon: 'M12 3l2.4 4.9 5.4.8-3.9 3.8.9 5.4L12 15.4l-4.8 2.5.9-5.4-3.9-3.8 5.4-.8z' },
  { to: '/markets', label: 'Markets', on: ['markets', 'asset'], icon: 'M4 17l5-5 4 4 7-7M15 9h5v5' },
];

export function App() {
  const route = match(usePath());
  const [me, setMe] = useMe();
  if (!me) return <main className="screen"><div className="loading" aria-busy="true" /></main>;
  if (!me.fund) return <main className="screen"><Start onStarted={setMe} /></main>;
  const top = route.screen === 'fund' || route.screen === 'managers' || route.screen === 'markets';
  return (
    <>
      <main className={`screen screen-${route.screen}`}>
        {route.screen === 'fund' && <FundHome me={me} setMe={setMe} />}
        {route.screen === 'markets' && <Markets />}
        {route.screen === 'asset' && <Asset key={route.market} id={route.market} />}
        {route.screen === 'managers' && <Managers />}
        {route.screen === 'manager' && <ManagerPage key={route.id} id={route.id} />}
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
