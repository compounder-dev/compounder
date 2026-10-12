// Every part in every variant and state, on one page. It is served in development
// only (`npm run dev`, then /parts.html) and is not part of the built app.

import '@fontsource-variable/inter-tight';
import './tokens.css';
import './base.css';
import './parts/parts.css';
import './screens/screens.css';
import './gallery.css';
import { useEffect, useState, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { FUND_COLOURS } from './api.ts';
import {
  Banner,
  BottomBar,
  Button,
  ButtonLink,
  Card,
  CardSkeleton,
  CardTitle,
  Chip,
  EmptyState,
  IconButton,
  Link,
  Mark,
  Number,
  Row,
  RowSkeleton,
  Sheet,
  Skeleton,
  TopBar,
  WeightBar,
  type ButtonVariant,
  type ChipVariant,
  type NavItem,
} from './parts/index.ts';
import { useAddressFlag } from './route.ts';

const PLACES: NavItem[] = [
  { href: '#home', label: 'Home', icon: 'home', holds: () => true },
  { href: '#explore', label: 'Explore', icon: 'info', holds: () => false },
  { href: '#build', label: 'Build', icon: 'pencil', holds: () => false },
  { href: '#account', label: 'Account', icon: 'coin', holds: () => false, badge: true },
];

const BUTTONS: ButtonVariant[] = ['primary', 'secondary', 'ghost', 'danger'];
const CHIPS: ChipVariant[] = ['live', 'waiting', 'paused', 'needs-you', 'draft', 'pretend-money'];

function Section({ name, children }: { name: string; children: ReactNode }) {
  return (
    <section className="gallery-section" aria-label={name}>
      <h2 className="section-title">{name}</h2>
      {children}
    </section>
  );
}

function Line({ children }: { children: ReactNode }) {
  return <div className="gallery-line">{children}</div>;
}

/** A value that moves every two seconds, to show the wash behind a number that changed. */
function useTicking(): number {
  const [value, setValue] = useState(1240.18);
  useEffect(() => {
    const timer = setInterval(() => setValue((now) => now + (Math.random() - 0.45) * 20), 2000);
    return () => clearInterval(timer);
  }, []);
  return value;
}

function Gallery() {
  const [sheet, setSheet] = useAddressFlag('sheet');
  const [open, setOpen] = useState(false);
  const ticking = useTicking();
  return (
    <>
      <TopBar items={PLACES} path="/parts.html" end={<IconButton icon="info" label="About these parts" />} />
      <main id="main" className="page">
        <h1 className="page-title">Parts</h1>
        <p className="page-lead">Every part in every variant and state. Hover, press and Tab through them, in both themes.</p>
        <div className="gallery">
          <Section name="Button">
            {(['md', 'lg'] as const).map((size) => (
              <Line key={size}>
                {BUTTONS.map((variant) => (
                  <Button key={variant} variant={variant} size={size}>
                    {variant}
                  </Button>
                ))}
              </Line>
            ))}
            <Line>
              {BUTTONS.map((variant) => (
                <Button key={variant} variant={variant} disabled>
                  disabled
                </Button>
              ))}
            </Line>
            <Line>
              {BUTTONS.map((variant) => (
                <Button key={variant} variant={variant} loading>
                  Loading
                </Button>
              ))}
            </Line>
            <Line>
              <Button variant="primary" icon="check">
                With an icon
              </Button>
              <ButtonLink variant="secondary" href="#button" icon="arrow-left">
                A link
              </ButtonLink>
            </Line>
            <Button variant="primary" size="lg" fullWidth>
              Full width
            </Button>
          </Section>

          <Section name="IconButton">
            <Line>
              <IconButton icon="close" label="Close" />
              <IconButton icon="arrow-left" label="Back" variant="filled" />
              <IconButton icon="close" label="Close, off" disabled />
            </Line>
          </Section>

          <Section name="Link">
            <p>
              A sentence with <Link href="#link">an inline link</Link> in it.
            </p>
            <Link href="#link" variant="standalone" icon="arrow-left">
              A standalone link
            </Link>
          </Section>

          <Section name="Card">
            <Card>A plain card.</Card>
            {FUND_COLOURS.map((fund) => (
              <Card key={fund} variant="strategy" fund={fund}>
                <CardTitle href="#card" dot>
                  A strategy card in {fund}
                </CardTitle>
                <p className="card-summary">The whole card opens it. Tab reaches its title.</p>
              </Card>
            ))}
            <Card variant="stat">
              <span>Per year</span>
              <Number kind="percent" value={0.127} sign size="title2" />
            </Card>
            <ul role="list">
              <CardSkeleton />
            </ul>
          </Section>

          <Section name="Row">
            <div className="fund-teal">
              <ul className="rows" role="list">
                <Row leading={<Mark letter="G" />} title="An asset" detail="Second line" trailing={<><WeightBar percent={60} /><Number kind="percent" value={0.4} /></>} />
                <Row variant="fund" href="#row" leading={<Mark letter="F" />} title="A row that goes somewhere" detail="With a name long enough that it has to wrap onto a second line" trailing={<Number kind="money" value={1240.18} />} />
                <Row variant="setting" title="A row that opens in place" detail={open ? 'Open' : 'Closed'} open={open} onToggle={() => setOpen(!open)} />
                <RowSkeleton />
              </ul>
            </div>
            <Card>
              <ul className="rows" role="list">
                <Row variant="method" href="#row" title="A row inside a card" detail="One layer up" />
              </ul>
            </Card>
          </Section>

          <Section name="Number">
            <Line>
              <Number kind="money" value={1240.18} />
              <Number kind="money" value={1240000} />
              <Number kind="money" value={0.004} />
              <Number kind="percent" value={0.0034} sign />
              <Number kind="percent" value={0} />
              <Number kind="ratio" value={1.5} />
              <Number kind="quantity" value={0.0859} />
              <Number kind="countdown" value={728} />
            </Line>
            <Line>
              <Number kind="percent" value={0.127} tone="gain" sign arrow />
              <Number kind="percent" value={-0.182} tone="loss" sign arrow />
              <Number kind="money" value={12} tone="muted" />
              <Number kind="money" value={null} />
              <Number kind="money" value={null} loading />
            </Line>
            <Line>
              <Number kind="money" value={ticking} size="title1" />
              <Number kind="money" value={1240.18} size="title2" proportional />
              <Number kind="money" value={1240.18} size="small" />
              <Number kind="money" value={1240.18} size="caption" />
            </Line>
          </Section>

          <Section name="Chip">
            <Line>
              {CHIPS.map((variant) => (
                <Chip key={variant} variant={variant} />
              ))}
            </Line>
          </Section>

          <Section name="Skeleton">
            <p>
              <Skeleton />
              <Skeleton width="long" />
              <Skeleton width="half" />
              <Skeleton width="short" />
            </p>
          </Section>

          <Section name="EmptyState">
            <EmptyState action={<Button>One action</Button>}>One sentence that says what is missing.</EmptyState>
          </Section>

          <Section name="Banner">
            <Banner variant="needs-you" action={<Button variant="primary">Do it</Button>}>
              Something needs you.
            </Banner>
            <Banner variant="info">Something worth knowing.</Banner>
            <Banner variant="error" action={<Button>Try again</Button>}>
              Something failed.
            </Banner>
          </Section>

          <Section name="Sheet">
            <Line>
              <Button onClick={() => setSheet('action')}>Open an action sheet</Button>
              <Button onClick={() => setSheet('full')}>Open a full-height sheet</Button>
              <Button onClick={() => setSheet('busy')}>Open a busy sheet</Button>
            </Line>
          </Section>
        </div>
      </main>
      <BottomBar items={PLACES} path="/parts.html" />

      <Sheet
        open={sheet !== null}
        onClose={() => setSheet(null)}
        title={sheet === 'busy' ? 'A busy sheet' : 'A sheet with nothing in it yet'}
        variant={sheet === 'full' ? 'full' : 'action'}
        busy={sheet === 'busy'}
        actions={
          <>
            <Button onClick={() => setSheet(null)}>Not now</Button>
            <Button variant="primary" loading={sheet === 'busy'}>
              Continue
            </Button>
          </>
        }
      >
        <p>Escape, the scrim, the cross and the back button all close it. Focus stays inside while it is open.</p>
        {sheet === 'busy' && (
          <p>
            <Link href="#sheet" variant="standalone">
              This one is busy. Use the back button to leave.
            </Link>
          </p>
        )}
      </Sheet>
    </>
  );
}

createRoot(document.getElementById('root')!).render(<Gallery />);
