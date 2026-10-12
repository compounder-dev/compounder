import { useEffect, useRef } from 'react';
import { BottomBar, TopBar, type NavItem } from './parts/index.ts';
import { matchRoute, PRICING, useAddress, type Route } from './route.ts';
import { Home } from './screens/Home.tsx';
import { Pricing } from './screens/Pricing.tsx';
import { StarterScreen } from './screens/Starter.tsx';
import { Boundary, Broke, NotFound, OfflineBar } from './screens/System.tsx';

/** The places the bars lead to. A place joins the list when its screen is built, and not before. */
const PLACES: NavItem[] = [
  { href: '/', label: 'Home', icon: 'home', holds: (path) => matchRoute(path).screen === 'starter' },
  { href: PRICING, label: 'Pricing', icon: 'coin', holds: () => false },
];

function Screen({ route }: { route: Route }) {
  switch (route.screen) {
    case 'home':
      return <Home />;
    case 'pricing':
      return <Pricing />;
    case 'starter':
      return <StarterScreen id={route.id} />;
    case 'not-found':
      return <NotFound />;
  }
}

type AppProps = {
  /** The address this page was made for, where there is no address bar to read. */
  address?: string;
  /** The server could not make this page and sent the page for that instead. */
  broke?: boolean;
};

/** The frame every screen stands in: a way past the bars, the bars, and the screen between them. */
export function App({ address, broke = false }: AppProps) {
  const { path } = useAddress(address);
  const first = useRef(path);
  const shown = useRef(path);

  // After a move between screens, focus goes to the new screen, so a keyboard and a screen reader start at its top.
  useEffect(() => {
    if (shown.current === path) return;
    shown.current = path;
    document.getElementById('main')?.focus({ preventScroll: true });
  }, [path]);

  return (
    <>
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <OfflineBar />
      <TopBar items={PLACES} path={path} />
      <main id="main" className="page" tabIndex={-1}>
        <Boundary key={path} path={path}>
          {broke && path === first.current ? <Broke path={path} /> : <Screen route={matchRoute(path)} />}
        </Boundary>
      </main>
      <BottomBar items={PLACES} path={path} />
    </>
  );
}
