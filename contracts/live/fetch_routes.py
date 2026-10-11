"""Fetch two real swap routes for the live check and save them as a fixture.

One route buys the stock token with cash, the other sells some back. Both are
built for the settlement contract's address, so the contract can execute them
unchanged on a private copy of the chain.
"""
import json, pathlib, sys, time, urllib.request

API = "https://aggregator-api.kyberswap.com/bsc/api/v1"
HEADERS = {"User-Agent": "Mozilla/5.0", "Accept": "application/json", "Content-Type": "application/json", "x-client-id": "live-check"}
SETTLEMENT = "0x5e771e5e771e5e771e5e771e5e771e5e771e5e77"
CASH = "0x55d398326f99059ff775485246999027b3197955"
TOKEN = sys.argv[1] if len(sys.argv) > 1 else "0xa9ee28c80f960b889dfbd1902055218cba016f75"
BUY_CASH = 200 * 10**18
SELL_TOKENS = 2 * 10**17
SLIPPAGE_BPS = 50


def call(url, body=None):
    request = urllib.request.Request(url, json.dumps(body).encode() if body is not None else None, HEADERS)
    return json.load(urllib.request.urlopen(request, timeout=30))


def route(token_in, token_out, amount_in):
    found = call(f"{API}/routes?tokenIn={token_in}&tokenOut={token_out}&amountIn={amount_in}")["data"]
    time.sleep(0.6)
    built = call(f"{API}/route/build", {
        "routeSummary": found["routeSummary"],
        "sender": SETTLEMENT,
        "recipient": SETTLEMENT,
        "slippageTolerance": SLIPPAGE_BPS,
    })["data"]
    venues = sorted({hop["exchange"] for path in found["routeSummary"]["route"] for hop in path})
    amount_out = int(built["amountOut"])
    return {
        "router": built["routerAddress"],
        "data": built["data"],
        "amountIn": str(amount_in),
        "amountOut": str(amount_out),
        "minAmountOut": str(amount_out * (10_000 - SLIPPAGE_BPS) // 10_000),
        "venues": ", ".join(venues),
    }


fixture = {
    "settlement": SETTLEMENT,
    "cash": CASH,
    "token": TOKEN,
    "buy": route(CASH, TOKEN, BUY_CASH),
    "sell": route(TOKEN, CASH, SELL_TOKENS),
}
out = pathlib.Path(__file__).resolve().parent.parent / "test" / "fixtures" / "live.json"
out.write_text(json.dumps(fixture, indent=1))
buy, sell = fixture["buy"], fixture["sell"]
print(f"buy  {BUY_CASH / 1e18:.0f} cash -> {int(buy['amountOut']) / 1e18:.6f} tokens via {buy['venues']}")
print(f"sell {SELL_TOKENS / 1e18:.1f} tokens -> {int(sell['amountOut']) / 1e18:.4f} cash via {sell['venues']}")
