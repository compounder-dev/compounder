// The wallet the browser already has. It keeps the key and it keeps the
// tokens: this app asks for a signature and never holds a fund, so no
// approval or withdrawal runs through a server of ours.

export const BNB_CHAIN = 56;
/** The cash a swap spends. USDT is what the stock tokens are quoted against on BNB Chain. */
export const CASH = { symbol: 'USDT', address: '0x55d398326f99059ff775485246999027b3197955', decimals: 18 } as const;

export type Wallet = { address: string; chainId: number };

type Provider = { request(args: { method: string; params?: unknown[] }): Promise<unknown> };

const provider = (): Provider | null => (window as unknown as { ethereum?: Provider }).ethereum ?? null;

/** The browser has no wallet extension to sign with. */
export class NoWallet extends Error {}

/** Ask the wallet for an account and make sure it is on the chain the tokens live on. */
export async function connect(): Promise<Wallet> {
  const wallet = provider();
  if (!wallet) throw new NoWallet('No wallet in this browser. Install MetaMask, Rabby or Trust.');
  const [address] = (await wallet.request({ method: 'eth_requestAccounts' })) as string[];
  await onBnb(wallet);
  const chainId = Number(await wallet.request({ method: 'eth_chainId' }));
  return { address, chainId };
}

/** Wallets rarely start on BNB Chain, and some have not heard of it at all. */
async function onBnb(wallet: Provider): Promise<void> {
  if (Number(await wallet.request({ method: 'eth_chainId' })) === BNB_CHAIN) return;
  try {
    await wallet.request({ method: 'wallet_switchEthereumChain', params: [{ chainId: '0x38' }] });
  } catch {
    await wallet.request({
      method: 'wallet_addEthereumChain',
      params: [
        {
          chainId: '0x38',
          chainName: 'BNB Smart Chain',
          nativeCurrency: { name: 'BNB', symbol: 'BNB', decimals: 18 },
          rpcUrls: ['https://bsc-dataseed.binance.org'],
          blockExplorerUrls: ['https://bscscan.com'],
        },
      ],
    });
  }
}

/** A whole-token amount as the chain counts it. */
export const units = (value: number, decimals: number): bigint => {
  const [whole, fraction = ''] = value.toFixed(decimals).split('.');
  return BigInt(whole) * 10n ** BigInt(decimals) + BigInt((fraction + '0'.repeat(decimals)).slice(0, decimals) || '0');
};

/** What the chain counts, as a number for showing. */
export const tokens = (raw: bigint, decimals: number): number => Number(raw) / 10 ** decimals;

const padded = (address: string) => address.toLowerCase().replace(/^0x/, '').padStart(64, '0');

async function call(wallet: Provider, to: string, data: string): Promise<bigint> {
  const answer = (await wallet.request({ method: 'eth_call', params: [{ to, data }, 'latest'] })) as string;
  return BigInt(answer);
}

/** How much of a token an address holds. */
export function balanceOf(token: string, owner: string): Promise<bigint> {
  const wallet = provider();
  if (!wallet) return Promise.resolve(0n);
  return call(wallet, token, `0x70a08231${padded(owner)}`);
}

/** How much of a token a spender may take from an owner. */
export function allowance(token: string, owner: string, spender: string): Promise<bigint> {
  const wallet = provider();
  if (!wallet) return Promise.resolve(0n);
  return call(wallet, token, `0xdd62ed3e${padded(owner)}${padded(spender)}`);
}

/** Let a router take exactly this much, and no more. */
export async function approve(token: string, spender: string, amount: bigint): Promise<string> {
  return send(token, `0x095ea7b3${padded(spender)}${amount.toString(16).padStart(64, '0')}`);
}

/** One transaction, signed by the wallet and sent as it answers. */
export async function send(to: string, data: string, value = 0n): Promise<string> {
  const wallet = provider();
  if (!wallet) throw new NoWallet('No wallet in this browser.');
  const [from] = (await wallet.request({ method: 'eth_accounts' })) as string[];
  const hash = (await wallet.request({
    method: 'eth_sendTransaction',
    params: [{ from, to, data, value: `0x${value.toString(16)}` }],
  })) as string;
  return hash;
}

export const transaction = (hash: string) => `https://bscscan.com/tx/${hash}`;

/** Wait until the chain includes a transaction, and fail if it did not work. */
export async function mined(hash: string, tries = 80): Promise<void> {
  const wallet = provider();
  if (!wallet) throw new NoWallet('No wallet in this browser.');
  for (let i = 0; i < tries; i++) {
    const receipt = (await wallet.request({ method: 'eth_getTransactionReceipt', params: [hash] })) as { status?: string } | null;
    if (receipt) {
      if (receipt.status !== '0x1') throw new Error('The transaction failed on chain.');
      return;
    }
    await new Promise((resolve) => window.setTimeout(resolve, 1500));
  }
  throw new Error('The chain has not included it yet. Check your wallet.');
}
