# Compounder

Funds as rules. A strategy is a document of rule trees; the engine evaluates it, backtests it over real prices, and rebalances to exact money. One settlement contract trades many funds in a single transaction, netting what one fund sells against what another buys.

- `engine`: strategy documents, rule evaluation, backtests, rebalancing and batching. Pure functions, no I/O.
- `prices`: the price store.
- `server`: the engine behind an HTTP API, serving the web app from one port.
- `wasm`: the engine built for the browser.
- `web`: the strategy builder and fund catalogue.
- `contracts`: the settlement contract (Foundry).
- `operator`: the daily run. `operator/dry-run.sh` runs a whole day against a private copy of the chain.

## Run

```sh
cargo test
cd web && npm install && npm run dev
```
