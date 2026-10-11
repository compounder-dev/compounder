// The Rust engine, compiled to WebAssembly and run here in the browser, so the
// page can check the server's numbers instead of trusting them.

// Imported as a URL so the build gives the file a content hash and browsers never run a stale engine.
import engineUrl from './engine.wasm?url';

type Exports = {
  memory: WebAssembly.Memory;
  alloc(len: number): number;
  dealloc(ptr: number, len: number): void;
  strategy_weights(ptr: number, len: number): bigint;
  rebalance_plan(ptr: number, len: number): bigint;
};

type Reply<T> = { ok: T } | { error: { message: string } };

let loading: Promise<Exports> | undefined;

function load(): Promise<Exports> {
  loading ??= WebAssembly.instantiateStreaming(fetch(engineUrl)).then(
    (module) => module.instance.exports as unknown as Exports,
  );
  return loading;
}

/** Hand `input` to one engine function and return its parsed reply. */
function call<T>(engine: Exports, entry: 'strategy_weights' | 'rebalance_plan', input: unknown): T {
  const bytes = new TextEncoder().encode(JSON.stringify(input));
  const ptr = engine.alloc(bytes.length);
  new Uint8Array(engine.memory.buffer, ptr, bytes.length).set(bytes);
  // The reply's pointer is in the high 32 bits and its length in the low 32.
  const packed = BigInt.asUintN(64, engine[entry](ptr, bytes.length));
  engine.dealloc(ptr, bytes.length);
  const replyPtr = Number(packed >> 32n);
  const replyLen = Number(packed & 0xffffffffn);
  const text = new TextDecoder().decode(new Uint8Array(engine.memory.buffer, replyPtr, replyLen));
  engine.dealloc(replyPtr, replyLen);
  const reply = JSON.parse(text) as Reply<T>;
  if ('error' in reply) throw new Error(reply.error.message);
  return reply.ok;
}

/** Each asset's target in parts per billion, worked out from the strategy document in this browser. */
export async function strategyWeights(strategy: unknown): Promise<Record<string, number>> {
  return call<{ weights: Record<string, number> }>(await load(), 'strategy_weights', strategy).weights;
}
