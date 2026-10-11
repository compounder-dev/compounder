#!/usr/bin/env bash
# Run the settlement contract on a private copy of the real chain, with real
# tokens, the real public price feed and swap routes fetched seconds earlier.
# Nothing is sent to the real network and no funds are used.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.foundry/bin:$PATH"
export LIVE=1
# The node must keep the state of recent blocks for a few minutes. Many public
# nodes forget it within a minute, which fails a check half way through.
export FORK_URL="${FORK_URL:-https://bsc-mainnet.public.blastapi.io}"

# Each check gets fresh routes and its own copy of the chain.
for check in test_live_twoAccountsBuyTogether test_live_oneSellsOneBuysTheRestGoesToMarket; do
  python3 -I live/fetch_routes.py "$@"
  forge test --match-test "$check" -vv | grep -E "^\[(PASS|FAIL)|^  [A-Za-z]|Suite result"
done
