#!/usr/bin/env bash
# Run the settlement contract on a private copy of the real chain, with real
# tokens, the real public price feed and swap routes fetched seconds earlier.
# Nothing is sent to the real network and no funds are used.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.foundry/bin:$PATH"
export LIVE=1
export FORK_URL="${FORK_URL:-https://bsc-dataseed.bnbchain.org}"

# Each check gets fresh routes and its own copy of the chain, so it finishes
# before a public node forgets the block it started from.
for check in test_live_twoAccountsBuyTogether test_live_oneSellsOneBuysTheRestGoesToMarket; do
  python3 -I live/fetch_routes.py "$@"
  forge test --match-test "$check" -vv | grep -E "^\[(PASS|FAIL)|^  [A-Za-z]|Suite result"
done
