// The wallet the browser already has. It keeps the key and it keeps the
// tokens: this app asks for a signature and never holds a fund, so no
// approval or withdrawal runs through a server of ours.

export const BNB_CHAIN = 56;
/** The cash a swap spends. USDT is what the stock tokens are quoted against on BNB Chain. */
export const CASH = { symbol: 'USDT', address: '0x55d398326f99059ff775485246999027b3197955', decimals: 18 } as const;
/** The same cash on Base, where a South African exchange can send it cheaply. Bridged in one signature. */
export const BASE_USDC = { symbol: 'USDC', address: '0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913', decimals: 6 } as const;
export const BASE = 8453;

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
  await onChain(BNB_CHAIN);
  const chainId = Number(await wallet.request({ method: 'eth_chainId' }));
  return { address, chainId };
}

const CHAINS: Record<number, { hex: string; name: string; symbol: string; rpc: string[]; explorer: string }> = {
  [BNB_CHAIN]: {
    hex: '0x38',
    name: 'BNB Smart Chain',
    symbol: 'BNB',
    rpc: ['https://bsc-dataseed.binance.org'],
    explorer: 'https://bscscan.com',
  },
  [BASE]: {
    hex: '0x2105',
    name: 'Base',
    symbol: 'ETH',
    rpc: ['https://mainnet.base.org'],
    explorer: 'https://basescan.org',
  },
};

/** Put the wallet on a chain, and teach it that chain if it has never heard of it. */
export async function onChain(chain: number): Promise<void> {
  const wallet = provider();
  if (!wallet) throw new NoWallet('No wallet in this browser.');
  if (Number(await wallet.request({ method: 'eth_chainId' })) === chain) return;
  const known = CHAINS[chain];
  try {
    await wallet.request({ method: 'wallet_switchEthereumChain', params: [{ chainId: known.hex }] });
  } catch {
    await wallet.request({
      method: 'wallet_addEthereumChain',
      params: [
        {
          chainId: known.hex,
          chainName: known.name,
          nativeCurrency: { name: known.symbol, symbol: known.symbol, decimals: 18 },
          rpcUrls: known.rpc,
          blockExplorerUrls: [known.explorer],
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
