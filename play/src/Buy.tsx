// The real share: the wallet pays, a router swaps, and the token lands in the
// wallet's own address — or goes back for cash the same way. Cash can sit on
// BNB Chain or on Base, which is where a South African exchange can send it.
// This app never holds the money, so there is nothing of yours here to lose.

import { useEffect, useState } from 'react';
import { useMe, type Token } from './api.ts';
import { usd } from './format.ts';
import { Sheet } from './sheets.tsx';
import { crossQuote, quote, type CrossQuote, type Quote } from './swap.ts';
import { BASE, BASE_USDC, BNB_CHAIN, CASH, approve, allowance, balanceOf, connect, mined, onChain, send, tokens, transaction, units, type Wallet } from './wallet.ts';

const PRESETS = [25, 50, 100];
const NAME = (token: Token) => token.name.replace(/\s*\(Ondo Tokenized\)$/, '');

type Source = 'bnb' | 'base';

export function BuySheet({ token, onClose }: { token: Token; onClose: () => void }) {
  const [mode, setMode] = useState<'buy' | 'sell'>('buy');
  // Where the cash sits. A token can only be sold where it lives, so a sale is always on BNB Chain.
  const [source, setSource] = useState<Source>('bnb');
  const [wallet, setWallet] = useState<Wallet | null>(null);
  const [cash, setCash] = useState<number | null>(null);
  const [held, setHeld] = useState<number | null>(null);
  const [amount, setAmount] = useState(50);
  /** What the user typed, in whole dollars. */
  const spend = amount;
  const [found, setFound] = useState<Quote | null>(null);
  const [cross, setCross] = useState<CrossQuote | null>(null);
  const [busy, setBusy] = useState<'idle' | 'connecting' | 'quoting' | 'trading'>('idle');
  const [step, setStep] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<{ hash: string; got: number; sold: boolean; bridged: boolean } | null>(null);
  const [me] = useMe();

  const buying = mode === 'buy';
  const bridging = buying && source === 'base';
  const pay = bridging ? BASE_USDC : CASH;
  const payDecimals = bridging ? BASE_USDC.decimals : CASH.decimals;

  const read = async (w: Wallet, where: Source) => {
    await onChain(where === 'base' ? BASE : BNB_CHAIN);
    if (where === 'base') {
      setCash(tokens(await balanceOf(BASE_USDC.address, w.address), BASE_USDC.decimals));
      setHeld(null);
      return;
    }
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
      await read(w, source);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy('idle');
    }
  };

  const pick = async (next: Source) => {
    setSource(next);
    setFound(null);
    setCross(null);
    setError(null);
    setCash(null);
    if (wallet) {
      try {
        await read(wallet, next);
      } catch (e) {
        setError((e as Error).message);
      }
    }
  };

  // A price is asked for as the amount changes, and only the last answer is kept.
  useEffect(() => {
    if (!wallet || !(spend > 0)) return;
    let live = true;
    setBusy('quoting');
    setFound(null);
    setCross(null);
    setError(null);
    const timer = window.setTimeout(() => {
      const work = bridging
        ? crossQuote(BASE_USDC, BASE, token.address, units(spend, BASE_USDC.decimals), wallet.address).then((answer) => live && setCross(answer))
        : quote(pay.address, buying ? token.address : CASH.address, units(spend, payDecimals), wallet.address, 100).then((answer) => live && setFound(answer));
      work.catch((e) => live && setError((e as Error).message)).finally(() => live && setBusy('idle'));
    }, 450);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [wallet, spend, buying, bridging, token.address]);

  const trade = async () => {
    if (!wallet || !(spend > 0)) return;
    if (!bridging && !found) return;
    if (bridging && !cross) return;
    setBusy('trading');
    setError(null);
    try {
      const need = units(spend, payDecimals);
      const spender = bridging ? cross!.approval : found!.router;
      if ((await allowance(pay.address, wallet.address, spender)) < need) {
        setStep(buying ? 'Approve the cash — confirm in your wallet' : `Approve ${token.symbol} — confirm in your wallet`);
        await mined(await approve(pay.address, spender, need));
      }
      // The approval took a block, so the route is older than the price it quoted.
      setStep('Getting a fresh price');
      if (bridging) {
        const fresh = await crossQuote(BASE_USDC, BASE, token.address, need, wallet.address);
        setStep('Confirm the bridge in your wallet');
        const hash = await send(fresh.to, fresh.data, fresh.value);
        setStep('Crossing to BNB Chain…');
        await mined(hash);
        setDone({ hash, got: tokens(fresh.out, token.decimals), sold: false, bridged: true });
        // The token arrives on the other chain; look again from there.
        await new Promise((resolve) => window.setTimeout(resolve, 4000));
        await read(wallet, 'bnb').catch(() => {});
        return;
      }
      const fresh = await quote(pay.address, buying ? token.address : CASH.address, need, wallet.address, 100);
      setStep(buying ? `Confirm the swap for ${token.symbol} in your wallet` : 'Confirm the sale in your wallet');
      const hash = await send(fresh.router, fresh.data, fresh.value);
      setStep('Waiting for the chain');
      await mined(hash);
      setDone({ hash, got: tokens(fresh.out, buying ? token.decimals : CASH.decimals), sold: !buying, bridged: false });
      await read(wallet, 'bnb');
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy('idle');
      setStep('');
    }
  };

  const short = buying ? cash !== null && cash < spend : held !== null && held < spend;
  const shortBy = buying ? spend - (cash ?? 0) : spend - (held ?? 0);
  const below = buying ? (bridging ? (cross && spend > 0 ? Math.max(0, 1 - cross.outUsd / spend) : 0) : (found && spend > 0 ? Math.max(0, 1 - found.outUsd / spend) : 0)) : 0;
  const gas = bridging ? (cross?.gasUsd ?? 0) : (found?.gasUsd ?? 0);

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
          <p className="big change-up">{done.sold ? usd(done.got) : `${done.got.toFixed(6)} ${token.symbol}`}</p>
          <p className="muted">
            {done.sold
              ? 'Sold for USDT, in your wallet now.'
              : done.bridged
                ? 'Bought. It is on BNB Chain in your wallet now — open your wallet in a minute to see it.'
                : 'Bought with USDT. It is in your wallet now, not in Compounder.'}
          </p>
          <a className="cta" href={transaction(done.hash)} target="_blank" rel="noreferrer">
            See it on the explorer ↗
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
              {buying && source === 'base' ? 'USDC on Base' : 'USDT in your wallet'}
              <b>{cash === null ? '–' : usd(cash)}</b>
            </span>
            <span style={{ textAlign: 'right' }}>
              {token.symbol} held<b>{held === null ? '–' : held.toFixed(4)}</b>
            </span>
          </div>

          <div className="sides">
            <button className={`go${buying ? ' go-up go-on' : ''}`} onClick={() => setMode('buy')}>
              Buy
            </button>
            <button className={`go${!buying ? ' go-down go-on' : ''}`} onClick={() => { setMode('sell'); void pick('bnb'); }}>
              Sell
            </button>
          </div>

          {buying && (
            <div className="start-presets">
              {(
                [
                  { key: 'bnb', label: 'USDT · BNB' },
                  { key: 'base', label: 'USDC · Base' },
                ] as { key: Source; label: string }[]
              ).map(({ key, label }) => (
                <button key={key} className={`chip${source === key ? ' chip-on' : ''}`} onClick={() => void pick(key)}>
                  {label}
                </button>
              ))}
            </div>
          )}

          <div className="start-presets">
            {(buying
              ? PRESETS.map((dollars) => ({ key: `$${dollars}`, label: `$${dollars}`, target: dollars }))
              : PRESETS.map((percent) => ({ key: `${percent}%`, label: `${percent}%`, target: ((held ?? 0) * percent) / 100 }))
            ).map(({ key, label, target }) => (
              <button key={key} className={`chip${Math.abs(target - spend) < 1e-9 ? ' chip-on' : ''}`} onClick={() => setAmount(target)}>
                {label}
              </button>
            ))}
          </div>

          <div className="field">
            <div className="field-head">
              <label htmlFor="buy-amount">{buying ? 'Spend' : `Sell ${token.symbol}`}</label>
              <span className="field-value">
                {buying ? '$' : ''}
                <input id="buy-amount" inputMode="decimal" value={amount} onChange={(e) => setAmount(Math.max(0, Number(e.target.value.replace(/[^0-9.]/g, '')) || 0))} />
                {buying ? '' : ` ${token.symbol}`}
              </span>
            </div>
          </div>

          {(found || cross) && (
            <div className="summary">
              <span>
                You get
                <b>
                  {buying
                    ? `${tokens((cross ?? found)!.out, token.decimals).toFixed(6)} ${token.symbol}`
                    : usd(tokens(found!.out, CASH.decimals))}
                </b>
              </span>
              <span style={{ textAlign: 'right' }}>
                Gas<b>{usd(gas)}</b>
              </span>
            </div>
          )}
          {below > 0.0005 && (
            <p className="muted">
              The route pays {below < 0.01 ? `${(below * 100).toFixed(2)}%` : `${(below * 100).toFixed(1)}%`} below the cash you put in: the
              market's own price for the token today, plus the trade{bridging ? ` and the bridge, about ${cross?.seconds ?? 40} seconds` : ''}.
            </p>
          )}

          {step && <p className="muted">{step}</p>}
          {error && <p className="error">{error}</p>}

          {short ? (
            <p className="error">
              {buying
                ? `Send at least ${usd(shortBy)} more ${source === 'base' ? 'USDC on Base' : 'USDT on BNB Chain'} to ${wallet.address.slice(0, 6)}…${wallet.address.slice(-4)}.`
                : `You hold ${held?.toFixed(6)} ${token.symbol}; sell that or less.`}
            </p>
          ) : (
            <button className="cta cta-up" disabled={(bridging ? !cross : !found) || busy !== 'idle' || spend <= 0} onClick={trade}>
              {busy === 'quoting'
                ? 'Getting a price…'
                : busy === 'trading'
                  ? buying
                    ? 'Buying…'
                    : 'Selling…'
                  : buying
                    ? `Buy ${token.symbol} with ${bridging ? 'USDC on Base' : 'USDT'}`
                    : `Sell ${token.symbol} for USDT`}
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
