#!/usr/bin/env bash
# One whole daily run on a private copy of the real chain: real tokens, the
# real public price feed and real market routes. Nothing is sent to the real
# network and no funds are used.
#
# Three funds trade in one transaction:
#   Ava deposits 1,000 dollars into seven stocks.
#   Ben holds about 1,000 dollars of NVIDIA and moves it all into Tesla.
#   Cy deposits 1,500 dollars for NVIDIA, the stock Ben is leaving.
# So Ben's NVIDIA goes straight to Ava and Cy, only the shortfall is bought on
# the market, and Ben's Tesla is bought with what he was just paid.
#
# Run it while the US market is open. Outside those hours several of these
# stocks trade well away from the public price, and the contract refuses them.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.foundry/bin:$PATH"

# The node must keep the state of recent blocks for the length of the run.
FORK_URL="${FORK_URL:-https://bsc-mainnet.public.blastapi.io}"
ROUTING_API="${ROUTING_API:-https://aggregator-api.kyberswap.com/bsc/api/v1}"
RPC=http://127.0.0.1:8546
CASH=0x55d398326f99059ff775485246999027b3197955
FEED=0x14032815B65f6B65f23d2532ad5F5DFf7BE48C03
NVDA=0xa9ee28c80f960b889dfbd1902055218cba016f75
TSLA=0x2494b603319d4d9f9715c9f4496d9e0364b59d93
MAX=115792089237316195423570985008687907853269984665640564039457584007913129639935
DAILY_LIMIT=10000000000000000000000
WORK=$(mktemp -d)
CONFIG=$WORK/config.json

cargo build -q -p compounder-operator
operator() { target/debug/compounder-operator "$@"; }
call() { cast call "$@" --rpc-url $RPC | awk '{print $1; exit}'; }
# Queue a transaction from an account. `mine` then puts everything queued into one block.
declare -A NONCE
send() { # from, then the call
  local from=$1 nonce
  shift
  nonce=${NONCE[$from]:-$(cast nonce "$from" --rpc-url $RPC)}
  cast send --async --unlocked --from "$from" --nonce "$nonce" "$@" --rpc-url $RPC >/dev/null
  NONCE[$from]=$((nonce + 1))
}
mine() { cast rpc evm_mine --rpc-url $RPC >/dev/null; }

anvil --fork-url "$FORK_URL" --port 8546 --timeout 60000 --retries 10 --silent &
ANVIL=$!
# KEEP_NODE=1 leaves the copy of the chain running afterwards, to look into a failed run.
trap 'rm -rf "$WORK"; if [ -n "${KEEP_NODE:-}" ]; then echo "node left running at $RPC, process $ANVIL"; else kill $ANVIL 2>/dev/null; fi' EXIT
for _ in $(seq 120); do cast block-number --rpc-url $RPC >/dev/null 2>&1 && break; sleep 0.5; done
# Nobody refreshes the market makers' quotes on this copy of the chain, and
# their venues refuse to trade on a quote more than a few seconds old. So the
# clock is stopped at the moment the copy was taken. Some of them count
# blocks instead, so the whole setup is packed into three blocks.
cast rpc anvil_setBlockTimestampInterval 0 --rpc-url $RPC >/dev/null

read -r ADMIN OPERATOR AVA BEN CY < <(cast rpc eth_accounts --rpc-url $RPC | python3 -I -c 'import json,sys; print(" ".join(json.load(sys.stdin)[:5]))')

# The contract: prices within 1% of a public feed price no older than two days.
SETTLEMENT=$(forge create src/Settlement.sol:Settlement --root contracts --rpc-url $RPC --unlocked --from "$ADMIN" --broadcast \
  --constructor-args $CASH $FEED "$ADMIN" "$OPERATOR" 100 172800 | awk '/Deployed to:/ {print $3}')
COLLECTOR=$(call "$SETTLEMENT" "collector()(address)")
cast rpc evm_setAutomine false --rpc-url $RPC >/dev/null

# A route normally waits two days. The delay is set aside here: moving this
# copy of the chain two days ahead would make the real prices and routes stale.
ROUTER=$(python3 -I - "$ROUTING_API" $CASH $NVDA <<'PY'
import json, sys, urllib.request
api, cash, token = sys.argv[1:]
request = urllib.request.Request(
    f"{api}/routes?tokenIn={cash}&tokenOut={token}&amountIn={10**19}",
    headers={"User-Agent": "compounder-operator", "x-client-id": "compounder"},
)
print(json.load(urllib.request.urlopen(request, timeout=20))["data"]["routerAddress"])
PY
)
cast rpc anvil_setStorageAt "$SETTLEMENT" "$(cast index address "$ROUTER" 3)" "$(cast to-uint256 1)" --rpc-url $RPC >/dev/null
[ "$(call "$SETTLEMENT" "isRouter(address)(bool)" "$ROUTER")" = true ] || { echo "could not approve the route"; exit 1; }

# Write the operator's config: which funds exist and the strategy each follows.
write_config() { # ben's strategy: nvidia or tesla
  python3 -I - "$1" "$CONFIG" "$RPC" "$ROUTING_API" "$SETTLEMENT" $FEED $CASH "$OPERATOR" "$AVA" "$BEN" "$CY" $NVDA $TSLA <<'PY'
import json, sys
bens, path, rpc, api, settlement, feed, cash, operator, ava, ben, cy, nvda, tsla = sys.argv[1:]

def strategy(name, token):
    slug = name.lower().replace(" ", "-")
    return {
        "entry": {
            "blockType": "topLevel", "name": name, "blockIdForInstance": slug, "parentNodeId": "root",
            "children": {
                "blockType": "weight", "weightType": "specified-percentage", "weightings": ["100"],
                "blockIdForInstance": f"{slug}-weights", "parentNodeId": slug,
                "children": [{"blockType": "asset", "assetId": f"{token}:56", "blockIdForInstance": f"{slug}-0", "parentNodeId": f"{slug}-weights"}],
            },
        },
        "version": "1.0.0",
        "tradingSettings": {"type": "threshold", "triggerPercentage": 10},
    }

starters = json.load(open("server/data/starters.json"))["starters"]
seven = next(s["strategy"] for s in starters if s["id"] == "magnificent-seven")
json.dump({
    "rpc_url": rpc, "routing_api": api, "settlement": settlement, "price_feed": feed, "cash": cash, "operator": operator,
    "funds": [
        {"name": "Ava", "account": ava, "strategy": seven},
        {"name": "Ben", "account": ben, "strategy": strategy("All NVIDIA", nvda) if bens == "nvidia" else strategy("All Tesla", tsla)},
        {"name": "Cy", "account": cy, "strategy": strategy("All NVIDIA", nvda)},
    ],
    "trigger_bps": 1000, "min_trade_dollars": 5, "cushion_bps": 100, "slippage_bps": 50,
}, open(path, "w"))
PY
}

deposit() { # account dollars
  cast rpc anvil_setStorageAt $CASH "$(cast index address "$1" 1)" "$(cast to-uint256 "$(python3 -I -c "print($2 * 10**18)")")" --rpc-url $RPC >/dev/null
  [ "$(call $CASH "balanceOf(address)(uint256)" "$1")" = "$(python3 -I -c "print($2 * 10**18)")" ] || { echo "could not credit the deposit"; exit 1; }
}

# Put stock tokens in an account by writing its balance. Where a token keeps
# balances is found by asking which storage it reads to answer balanceOf.
give_tokens() { # token account amount
  local slot old
  for slot in $(cast access-list "$1" "balanceOf(address)" "$2" --rpc-url $RPC | grep -oE '0x[0-9a-f]{64}'); do
    old=$(cast storage "$1" "$slot" --rpc-url $RPC)
    cast rpc anvil_setStorageAt "$1" "$slot" "$(cast to-uint256 "$3")" --rpc-url $RPC >/dev/null
    [ "$(call "$1" "balanceOf(address)(uint256)" "$2" 2>/dev/null)" = "$3" ] && return 0
    cast rpc anvil_setStorageAt "$1" "$slot" "$old" --rpc-url $RPC >/dev/null
  done
  echo "could not find where $1 keeps balances"; exit 1
}

# The fund approves the collector and opts in with the shares its strategy gives.
join() { # name
  local account tokens shares token
  read -r account tokens shares < <(operator targets "$CONFIG" | python3 -I -c '
import json, sys
fund = next(f for f in json.load(sys.stdin) if f["name"] == sys.argv[1])
print(fund["account"], ",".join(fund["tokens"]), ",".join(fund["shares"]))' "$1")
  for token in $CASH ${tokens//,/ }; do
    send "$account" "$token" "approve(address,uint256)" "$COLLECTOR" $MAX
  done
  send "$account" "$SETTLEMENT" "open(address[],uint256[],uint256)" "[$tokens]" "[$shares]" $DAILY_LIMIT
}

# Before today: Ben already runs an all-NVIDIA fund holding 4.3 tokens.
write_config nvidia
give_tokens $NVDA "$BEN" 4300000000000000000
join Ben
mine

# Today: Ava and Cy deposit, and Ben switches his fund to Tesla.
write_config tesla
deposit "$AVA" 1000
deposit "$CY" 1500
join Ava
join Ben
join Cy
mine
cast rpc evm_setAutomine true --rpc-url $RPC >/dev/null

echo "=== The daily run ==="
operator run "$CONFIG" --send-unlocked

echo
echo "=== Straight after: is anything left to do? ==="
operator run "$CONFIG"
