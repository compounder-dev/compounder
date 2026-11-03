import { useLiveFund, type LiveDetail } from '../api.ts';
import { useEvent } from '../events.ts';
import { Banner, Button, Link, Number, Row, Sparkline } from '../parts/index.ts';
import { LIVE, useTitle } from '../route.ts';
import { dayOf, kindWord, when } from '../live.ts';
import { asFall, changeTone } from '../strategy.ts';
import { NotFound } from './System.tsx';

function Ready({ fund }: { fund: LiveDetail }) {
  const tone = changeTone(fund.change);
  return (
    <div className="live-fund">
      <h1 className="page-title">{fund.name}</h1>
      <p className="live-kind">{kindWord(fund)}</p>
      <p className="live-equity">
        <Number kind="money" value={fund.equity} size="title1" proportional />
        <Number kind="percent" value={fund.change} tone={tone} sign arrow />
      </p>
      <Sparkline points={fund.curve} tone={tone} large label={`Value since ${dayOf(fund.opened)}`} />
      <p className="live-since">Since {dayOf(fund.opened)}</p>

      <section aria-labelledby="now-title">
        <h2 className="section-title" id="now-title">
          Now
        </h2>
        <p className="live-note">{fund.note || 'Waiting for its first decision.'}</p>
        {fund.decided !== null && <p className="live-when">{when(fund.decided)}</p>}
        {fund.holdings.length > 0 && (
          <ul className="rows" role="list">
            {fund.holdings.map((holding) => (
              <Row
                key={holding.market}
                title={holding.market}
                detail={holding.weight > 0 ? 'Long' : 'Short'}
                trailing={<Number kind="percent" value={Math.abs(holding.weight)} />}
              />
            ))}
          </ul>
        )}
      </section>

      <section aria-labelledby="record-title">
        <h2 className="section-title" id="record-title">
          Record
        </h2>
        <ul className="rows" role="list">
          <Row title="Worst drop" trailing={<Number kind="percent" value={asFall(fund.worst_drop)} />} />
          <Row title="Trades" trailing={<Number kind="quantity" value={fund.fills} />} />
          <Row title="Fees paid" trailing={<Number kind="money" value={fund.fees} />} />
          <Row title="Funding paid" trailing={<Number kind="money" value={fund.funding} />} />
          {fund.test && (
            <Row
              title="Tested, per year"
              detail={`${dayOf(fund.test.start)} to ${dayOf(fund.test.end)}`}
              trailing={<Number kind="percent" value={fund.test.annual_return} tone={changeTone(fund.test.annual_return)} sign />}
            />
          )}
          <Row
            title="Ledger"
            detail={`${fund.entries} entries · ${fund.head.slice(0, 12)}`}
            trailing={<span className={fund.verified ? 'live-verified' : 'live-broken'}>{fund.verified ? 'Verified' : 'Broken'}</span>}
          />
        </ul>
      </section>

      {fund.trades.length > 0 && (
        <section aria-labelledby="trades-title">
          <h2 className="section-title" id="trades-title">
            Trades
          </h2>
          <ul className="rows" role="list">
            {fund.trades.slice(0, 10).map((trade) => (
              <Row
                key={`${trade.t}-${trade.market}`}
                title={`${trade.qty > 0 ? 'Buy' : 'Sell'} ${trade.market}`}
                detail={when(trade.t)}
                trailing={<Number kind="money" value={Math.abs(trade.qty * trade.px)} />}
              />
            ))}
          </ul>
        </section>
      )}

      <section aria-labelledby="rules-title">
        <h2 className="section-title" id="rules-title">
          How it trades
        </h2>
        <p className="live-rules">{fund.rules}</p>
      </section>
    </div>
  );
}

export function LiveFundScreen({ id }: { id: string }) {
  const [fund, again] = useLiveFund(id);
  useTitle(fund.state === 'ready' ? `${fund.data.name} · Compounder` : 'Live fund · Compounder');
  useEvent('live_fund_opened', 'live-fund', id, fund.state === 'ready');
  if (fund.state === 'missing') return <NotFound />;
  return (
    <>
      <p className="back-link">
        <Link href={LIVE} variant="standalone" icon="arrow-left">
          All live funds
        </Link>
      </p>
      {fund.state === 'loading' && (
        <div className="live-waiting" aria-busy="true">
          <span className="skeleton skeleton-chart" aria-hidden="true" />
        </div>
      )}
      {fund.state === 'failed' && (
        <div className="region-notice">
          <Banner variant="error" action={<Button onClick={again}>Try again</Button>}>
            We could not load this fund.
          </Banner>
        </div>
      )}
      {fund.state === 'ready' && <Ready fund={fund.data} />}
    </>
  );
}
