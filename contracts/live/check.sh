#!/usr/bin/env bash
# Run the settlement contract on a private copy of the real chain, with real
# tokens, the real public price feed, and swap routes and issuer quotes
# fetched seconds earlier. Nothing is sent to the real network and no funds
# are used.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.foundry/bin:$PATH"
export LIVE=1
# FORK_URL is a node of the real chain. It must keep the state of recent
# blocks for a few minutes: many public nodes forget it within a minute,
# which fails a check half way through. ROUTING_API is the base address of
# the swap-routing service for that chain.
: "${FORK_URL:?set FORK_URL to a node of the real chain}"
: "${ROUTING_API:?set ROUTING_API to the routing service for that chain}"
export FORK_URL
SETTLEMENT=0x5e771e5e771e5e771e5e771e5e771e5e771e5e77
CASH=0x55d398326f99059ff775485246999027b3197955
ISSUER=0x91f8Aff3738825e8eB16FC6f6b1A7A4647bDB299
TOKEN="${1:-0xa9ee28c80f960b889dfbd1902055218cba016f75}"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cargo build -q -p compounder-operator --manifest-path ../Cargo.toml
forge build -q
mkdir -p test/fixtures

# One route built for the settlement contract, with the issuer quote its price is held to.
route() { # side token_in token_out amount_in
  cat > "$WORK/request.json" <<JSON
{"rpc_url": "$FORK_URL", "routing_api": "$ROUTING_API", "issuer": "$ISSUER", "cash": "$CASH", "taker": "$SETTLEMENT",
 "token_in": "$2", "token_out": "$3", "amount_in": "$4", "slippage_bps": 50}
JSON
  ../target/debug/compounder-operator route "$WORK/request.json" > "$WORK/$1.json"
  jq -r --arg side "$1" '"\($side) route via \(.venues), issuer quote \(.quote.price | tonumber / 1e18)"' "$WORK/$1.json"
}
buy() { route buy $CASH "$TOKEN" 200000000000000000000; }
sell() { route sell "$TOKEN" $CASH 200000000000000000; }

# Each check runs on its own copy of the chain. An issuer quote lives for
# seconds, so the side a check trades on is fetched just before it.
check() { # which side to refresh, then the test
  "$1"
  jq -n --arg settlement $SETTLEMENT --arg cash $CASH --arg token "$TOKEN" --arg issuer $ISSUER \
    --slurpfile buy "$WORK/buy.json" --slurpfile sell "$WORK/sell.json" \
    '{settlement: $settlement, cash: $cash, token: $token, issuer: $issuer, buy: $buy[0], sell: $sell[0]}' > test/fixtures/live.json
  forge test --match-test "$2" -vv | grep -E "^\[(PASS|FAIL)|^  [A-Za-z]|Suite result"
}
sell
check buy test_live_twoAccountsBuyTogether
check sell test_live_oneSellsOneBuysTheRestGoesToMarket
check buy test_live_oneBuysOneSellsTheRestComesFromMarket
