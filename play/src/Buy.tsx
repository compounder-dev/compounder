// The real share: the wallet pays, the router swaps, and the token lands in the
// wallet's own address — or goes back for cash the same way. This app never
// holds the money, so there is nothing of yours here to lose.

import { useEffect, useState } from 'react';
import { useMe, type Token } from './api.ts';
import { usd } from './format.ts';
import { Sheet } from './sheets.tsx';
import { quote, type Quote } from './swap.ts';
import { CASH, approve, allowance, balanceOf, connect, mined, send, tokens, transaction, units, type Wallet } from './wallet.ts';

const PRESETS = [25, 50, 100];
const NAME = (token: Token) => token.name.replace(/\s*\(Ondo Tokenized\)$/, '');

export function BuySheet({ token, onClose }: { token: Token; onClose: () => void }) {
  const [mode, setMode] = useState<'buy' | 'sell'>('buy');
  const [wallet, setWallet] = useState<Wallet | null>(null);
  const [cash, setCash] = useState<number | null>(null);
  const [held, setHeld] = useState<number | null>(null);
  const [amount, setAmount] = useState(50);
  const [found, setFound] = useState<Quote | null>(null);
  const [busy, setBusy] = useState<'idle' | 'connecting' | 'quoting' | 'trading'>('idle');
  const [step, setStep] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<{ hash: string; got: number; sold: boolean } | null>(null);
  const [me] = useMe();

  const buying = mode === 'buy';
  const payToken = buying ? CASH.address : token.address;
  const payDecimals = buying ? CASH.decimals : token.decimals;
  const getDecimals = buying ? token.decimals : CASH.decimals;

  const read = async (w: Wallet) => {
    const [cashRaw, heldRaw] = await Promise.all([balanceOf(CASH.address, w.address), balanceOf(token.address, w.address)]);
    setCash(tokens(cashRaw, CASH.decimals));
    setHeld(tokens(heldRaw, token.decimals));
  };

  const join = async () => {
    setBusy('connecting');
    setError(null);
    try {
      const w = await connect();
      setWallet(w);
      await read(w);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy('idle');
    }
  };

  // A quote is asked for as the amount changes, and only the last answer is kept.
  useEffect(() => {
    if (!wallet || !(amount > 0)) return;
    let live = true;
    setBusy('quoting');
    setFound(null);
    setError(null);
    const timer = window.setTimeout(() => {
      quote(payToken, buying ? token.address : CASH.address, units(amount, payDecimals), wallet.address, 100)
        .then((answer) => live && setFound(answer))
        .catch((e) => live && setError((e as Error).message))
        .finally(() => live && setBusy('idle'));
    }, 450);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [wallet, amount, buying, payToken, payDecimals, token.address]);

  const swap = async () => {
    if (!wallet || !found || !(amount > 0)) return;
    setBusy('trading');
    setError(null);
    try {
      const need = units(amount, payDecimals);
      if ((await allowance(payToken, wallet.address, found.router)) < need) {
        setStep(buying ? 'Approve the cash — confirm in your wallet' : `Approve ${token.symbol} — confirm in your wallet`);
        await mined(await approve(payToken, found.router, need));
      }
      // The approval took a block, so the route is older than the price it quoted.
      setStep('Getting a fresh price');
      const fresh = await quote(payToken, buying ? token.address : CASH.address, need, wallet.address, 100);
      setStep(buying ? `Confirm the swap for ${token.symbol} in your wallet` : 'Confirm the sale in your wallet');
      const hash = await send(fresh.router, fresh.data, fresh.value);
      setStep('Waiting for the chain');
      await mined(hash);
      setDone({ hash, got: tokens(fresh.out, getDecimals), sold: !buying });
      await read(wallet);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy('idle');
      setStep('');
    }
  };

  const short = buying ? cash !== null && cash < amount : held !== null && held < amount;
  const shortBy = buying ? amount - (cash ?? 0) : amount - (held ?? 0);
  const below = buying && found && amount > 0 ? Math.max(0, 1 - found.outUsd / amount) : 0;

  return (
    <Sheet onClose={onClose}>
      <div className="sheet-head">
        <h3>
          {buying ? 'Buy' : 'Sell'} {token.symbol}
        </h3>
        <span className="num">{NAME(token)}</span>
      </div>

      {done ? (
        <>
          <p className="big change-up">
            {done.sold ? usd(done.got) : `${done.got.toFixed(6)} ${token.symbol}`}
          </p>
          <p className="muted">
            {done.sold ? 'Sold for USDT, in your wallet now.' : `Bought with USDT. It is in your wallet now, not in Compounder.`}
          </p>
          <a className="cta" href={transaction(done.hash)} target="_blank" rel="noreferrer">
            See it on BscScan ↗
          </a>
        </>
      ) : !wallet ? (
        <>
          <p className="muted">
            {token.symbol} is a real share token on BNB Chain. Your wallet pays and your wallet holds it — no account with us, no custody.
          </p>
          <button className="cta cta-up" onClick={join} disabled={busy !== 'idle'}>
            {busy === 'connecting' ? 'Waiting for your wallet…' : 'Connect a wallet'}
          </button>
        </>
      ) : (
        <>
          <div className="summary">
            <span>
              In your wallet<b>{cash === null ? '–' : usd(cash)}</b>
            </span>
            <span style={{ textAlign: 'right' }}>
              {token.symbol} held<b>{held === null ? '–' : held.toFixed(4)}</b>
            </span>
          </div>

          <div className="sides">
            <button className={`go${buying ? ' go-up go-on' : ''}`} onClick={() => setMode('buy')}>
              Buy
            </button>
            <button className={`go${!buying ? ' go-down go-on' : ''}`} onClick={() => setMode('sell')}>
              Sell
            </button>
          </div>

          <div className="start-presets">
            {(buying
              ? PRESETS.map((dollars) => ({ key: `$${dollars}`, label: `$${dollars}`, target: dollars }))
              : PRESETS.map((percent) => ({ key: `${percent}%`, label: `${percent}%`, target: ((held ?? 0) * percent) / 100 }))
            ).map(({ key, label, target }) => (
              <button key={key} className={`chip${Math.abs(target - amount) < 1e-9 ? ' chip-on' : ''}`} onClick={() => setAmount(target)}>
                {label}
              </button>
            ))}
          </div>

          <div className="field">
            <div className="field-head">
              <label htmlFor="buy-amount">{buying ? 'Spend' : `Sell ${token.symbol}`}</label>
              <span className="field-value">
                {buying ? '$' : ''}
                <input
                  id="buy-amount"
                  inputMode="decimal"
                  value={amount}
                  onChange={(e) => setAmount(Math.max(0, Number(e.target.value.replace(/[^0-9.]/g, '')) || 0))}
                />
                {buying ? '' : ` ${token.symbol}`}
              </span>
            </div>
          </div>

          {found && (
            <div className="summary">
              <span>
                You get<b>{buying ? `${tokens(found.out, token.decimals).toFixed(6)} ${token.symbol}` : usd(tokens(found.out, CASH.decimals))}</b>
              </span>
              <span style={{ textAlign: 'right' }}>
                Gas<b>{usd(found.gasUsd)}</b>
              </span>
            </div>
          )}
          {below > 0.0005 && (
            <p className="muted">
              The route pays {below < 0.01 ? `${(below * 100).toFixed(2)}%` : `${(below * 100).toFixed(1)}%`} below the cash you put in: the
              market's own price for the token today, plus the trade.
            </p>
          )}

          {step && <p className="muted">{step}</p>}
          {error && <p className="error">{error}</p>}

          {short ? (
            <p className="error">
              {buying
                ? `Send at least ${usd(shortBy)} more USDT on BNB Chain to ${wallet.address.slice(0, 6)}…${wallet.address.slice(-4)}.`
                : `You hold ${held?.toFixed(6)} ${token.symbol}; sell that or less.`}
            </p>
          ) : (
            <button className="cta cta-up" disabled={!found || busy !== 'idle' || amount <= 0} onClick={swap}>
              {busy === 'quoting' ? 'Getting a price…' : busy === 'trading' ? (buying ? 'Buying…' : 'Selling…') : buying ? `Buy ${token.symbol} with USDT` : `Sell ${token.symbol} for USDT`}
            </button>
          )}
        </>
      )}

      <p className="muted">
        A tokenised share tracks the stock; it is not a share certificate, and it pays no dividend in cash. Ondo redeems it after their identity
        check.
      </p>
      {me && me.balance > 0 && <p className="muted">Your {usd(me.balance)} of practice credits stay in the paper fund and are not spent here.</p>}
    </Sheet>
  );
}
