import { useEvent } from '../events.ts';
import { Card, Icon } from '../parts/index.ts';
import { useTitle } from '../route.ts';

/** The two plans. What each includes is a short list of things that are true today. */
const PLANS = [
  { name: 'Free', price: '$0', per: null, includes: ['Build and test any fund', 'Run one fund'] },
  { name: 'Plus', price: '$20', per: 'a month', includes: ['Everything in Free', 'Run more than one fund'] },
];

/** What it costs, said before anyone has to ask. */
export function Pricing() {
  useTitle('Pricing · Compounder');
  useEvent('pricing_seen', 'pricing');
  return (
    <>
      <h1 className="page-title">Pricing</h1>
      <p className="page-lead">Free to build and test. Your first fund is free to run.</p>
      <ul className="plans" role="list">
        {PLANS.map((plan) => (
          <Card key={plan.name} as="li" className="plan">
            <h2 className="card-title">{plan.name}</h2>
            <p className="plan-price">
              <span className="number number-title1">{plan.price}</span>
              {plan.per !== null && <span className="plan-per">{plan.per}</span>}
            </p>
            <ul className="plan-includes" role="list">
              {plan.includes.map((line) => (
                <li key={line}>
                  <Icon name="check" />
                  {line}
                </li>
              ))}
            </ul>
          </Card>
        ))}
      </ul>
      <p className="plan-note">Adding money is not open yet, so nothing is charged today.</p>
    </>
  );
}
