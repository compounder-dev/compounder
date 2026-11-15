# Compounder

Funds as rules. A strategy is a document of rule trees; the engine evaluates it, backtests it over real prices, and rebalances to exact money. One settlement contract trades many funds in a single transaction, netting what one fund sells against what another buys.

- `desk`: funds that trade perpetual markets on stocks, indices, commodities and crypto around the clock. Each is tested on past bars and runs live on paper into a hash-chained ledger. `cargo run -p compounder-desk --release -- test | run | verify`.
- `engine`: strategy documents, rule evaluation, backtests, rebalancing and batching. Pure functions, no I/O.
- `prices`: the price store.
- `server`: the engine behind an HTTP API, serving the web app from one port.
- `wasm`: the engine built for the browser.
- `web`: the strategy builder and fund catalogue.
- `play`: the phone-first app people use: the onchain fund, markets, strategies, and the real tokenised share bought with a wallet. `cd play && npm install && npm run dev`.
- `contracts`: the settlement contract (Foundry).
- `operator`: the daily run. `operator/dry-run.sh` runs a whole day against a private copy of the chain.

## Real shares

The markets screen buys and sells the actual Ondo token — `NVDAon`, `SPCXon` and the rest — on BNB Chain. The user's own wallet pays USDT and receives the token: the server only names the tokens, with the address and decimals from `server/data/assets.json`, and the browser gets the route from KyberSwap's aggregator and asks the wallet to sign. Compounder never holds the money or the token, and no key is entered anywhere. The same sheet sells back to cash. The practice fund beside it is unchanged and labelled practice.

## Run

```sh
cargo test
cd web && npm install && npm run dev
```
