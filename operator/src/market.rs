//! Where market routes and the issuer's quotes come from, and how long each
//! stays good.
//!
//! Two things live here. The routing client finds and builds routes, at a
//! steady pace and with a time limit. The quote source answers one question:
//! the issuer's live quote for this token in this direction. Today a quote is
//! read out of a route's call. A direct source would replace `quote_for` and
//! nothing else.

use std::{
    collections::HashMap,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use alloy::{
    primitives::{Address, Bytes, B256, U256},
    providers::Provider,
};
use anyhow::{anyhow, bail, Context, Result};

use crate::{quote, routes, unit, IIssuer, ISettlement};

/// A quote with less than this many seconds left is treated as gone: there
/// would be no time to simulate, sign and be mined before it dies.
pub const RESERVE_SECONDS: i64 = 5;
/// How long fetching one more token's route and quote can take.
pub const FETCH_SECONDS: i64 = 6;
/// A call larger than this is not a route. Real ones are a few thousand bytes.
const LARGEST_CALL: usize = 100_000;

pub struct Settings<'a> {
    pub routing_api: &'a str,
    pub issuer: Address,
    pub cash: Address,
    /// Who routes are built for: it both pays and receives.
    pub taker: Address,
    pub slippage_bps: u32,
    /// Trade only through these venues, comma separated. Any venue when absent.
    pub venues: Option<&'a str>,
    /// The smallest route asked for only to read the issuer's quote from it.
    pub quote_probe_dollars: u64,
}

/// A route, and the issuer's quote its price is held to.
#[derive(Clone)]
pub struct Priced {
    pub route: routes::Route,
    pub quote: ISettlement::Quote,
    pub signature: Bytes,
}

/// Seconds since 1970 by this machine's clock, which is what quotes are dated in.
pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|since| since.as_secs() as i64).unwrap_or(0)
}

/// How many seconds a quote has left.
pub fn seconds_left(quote: &ISettlement::Quote) -> i64 {
    i64::try_from(quote.expiration).unwrap_or(i64::MAX).saturating_sub(now())
}

impl Priced {
    pub fn seconds_left(&self) -> i64 {
        seconds_left(&self.quote)
    }
}

pub struct Market<'a, P> {
    http: reqwest::Client,
    provider: &'a P,
    settings: Settings<'a>,
    chain: u64,
    domain: B256,
    probe: U256,
    last_request: Option<Instant>,
    /// Whether the issuer lets each address seen so far sign its quotes.
    signers: HashMap<Address, bool>,
    routes: HashMap<(Address, Address, U256), Priced>,
}

impl<'a, P: Provider> Market<'a, P> {
    pub async fn open(provider: &'a P, settings: Settings<'a>) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-client-id", reqwest::header::HeaderValue::from_static("compounder"));
        Ok(Market {
            http: reqwest::Client::builder()
                .user_agent("compounder-operator")
                .default_headers(headers)
                .timeout(Duration::from_secs(20))
                .build()?,
            provider,
            chain: provider.get_chain_id().await.context("the chain did not answer")?,
            domain: IIssuer::new(settings.issuer, provider)
                .getDomainSeparator()
                .call()
                .await
                .context("the issuer's contract did not answer")?,
            probe: U256::from(settings.quote_probe_dollars) * unit(),
            settings,
            last_request: None,
            signers: HashMap::new(),
            routes: HashMap::new(),
        })
    }

    // ---- the routing client ----

    /// The routing service allows about two requests a second.
    async fn pace(&mut self) {
        if let Some(last) = self.last_request {
            tokio::time::sleep(Duration::from_millis(500).saturating_sub(last.elapsed())).await;
        }
        self.last_request = Some(Instant::now());
    }

    /// What the market gives for `amount_in` right now, without building a route.
    pub async fn expect(&mut self, token_in: Address, token_out: Address, amount_in: U256) -> Result<U256> {
        self.pace().await;
        Ok(routes::find(&self.http, self.settings.routing_api, token_in, token_out, amount_in, self.settings.venues).await?.amount_out)
    }

    async fn build(&mut self, token_in: Address, token_out: Address, amount_in: U256) -> Result<routes::Route> {
        self.pace().await;
        let found = routes::find(&self.http, self.settings.routing_api, token_in, token_out, amount_in, self.settings.venues).await?;
        self.pace().await;
        routes::build(&self.http, self.settings.routing_api, &found, self.settings.taker, self.settings.slippage_bps).await
    }

    // ---- the quote source ----

    /// The issuer's quote inside a route's call: the one with the longest to
    /// live among those the issuer really signed. Otherwise why there is none.
    async fn quote_in(&mut self, call: &[u8], token: Address, issuer_sells: bool) -> Result<std::result::Result<(ISettlement::Quote, Bytes), &'static str>> {
        if call.len() > LARGEST_CALL {
            return Ok(Err("the route's call is too large to be a route"));
        }
        let scan = quote::scan(call, self.chain, token, issuer_sells, self.domain);
        let mut best: Option<quote::Candidate> = None;
        let mut signed_by_issuer = false;
        for candidate in scan.candidates {
            let allowed = match self.signers.get(&candidate.signer) {
                Some(allowed) => *allowed,
                None => {
                    let allowed = IIssuer::new(self.settings.issuer, self.provider).hasRole(quote::signer_role(), candidate.signer).call().await?;
                    self.signers.insert(candidate.signer, allowed);
                    allowed
                }
            };
            if !allowed {
                continue;
            }
            signed_by_issuer = true;
            let fresh = seconds_left(&candidate.quote) >= RESERVE_SECONDS;
            if fresh && best.as_ref().is_none_or(|kept| candidate.quote.expiration > kept.quote.expiration) {
                best = Some(candidate);
            }
        }
        Ok(match best {
            Some(candidate) => Ok((candidate.quote, candidate.signature)),
            None if signed_by_issuer => Err("the issuer's quote in the route is about to expire"),
            None if scan.quotes_seen == 0 => Err("the route does not pass through the issuer"),
            None if scan.signatures_seen == 0 => Err("the route carries a quote but no signature for it"),
            None => Err("the quote in the route is not signed by the issuer"),
        })
    }

    /// The issuer's live quote for `token`, in the direction of a trade that
    /// spends `amount_in` through `route`. A route that trades with the issuer
    /// carries one. A small trade's route does not, so larger routes for the
    /// same trade are built only to read a quote, and are never used. Which
    /// size reaches the issuer changes from minute to minute, so three are
    /// tried, smallest first.
    async fn quote_for(&mut self, route: &routes::Route, token_in: Address, token_out: Address, amount_in: U256) -> Result<(ISettlement::Quote, Bytes)> {
        let buying = token_in == self.settings.cash;
        let token = if buying { token_out } else { token_in };
        let mut why = match self.quote_in(&route.call, token, buying).await? {
            Ok(found) => return Ok(found),
            Err(why) => why,
        };
        for times in [1u8, 3, 10] {
            let dollars = self.probe * U256::from(times);
            let size = if buying { dollars } else { dollars * amount_in / route.amount_out };
            if size <= amount_in {
                continue;
            }
            // A route that cannot be built at this size says nothing about the next.
            let Ok(larger) = self.build(token_in, token_out, size).await else { continue };
            match self.quote_in(&larger.call, token, buying).await? {
                Ok(found) => return Ok(found),
                Err(reason) => why = reason,
            }
        }
        bail!("there is no issuer price to hold the trade to: {why}")
    }

    /// A route that spends exactly `amount_in`, with the issuer's live quote
    /// for the stock token in the same direction. One fetched moments ago is
    /// reused while its quote has time left.
    pub async fn route(&mut self, token_in: Address, token_out: Address, amount_in: U256) -> Result<Priced> {
        let key = (token_in, token_out, amount_in);
        if let Some(priced) = self.routes.get(&key) {
            if priced.seconds_left() >= RESERVE_SECONDS + FETCH_SECONDS {
                return Ok(priced.clone());
            }
            self.routes.remove(&key);
        }
        let route = self.build(token_in, token_out, amount_in).await?;
        if route.amount_out.is_zero() || route.min_out.is_zero() {
            return Err(anyhow!("the market offers nothing for it"));
        }
        let (quote, signature) = self.quote_for(&route, token_in, token_out, amount_in).await?;
        let priced = Priced { route, quote, signature };
        self.routes.insert(key, priced.clone());
        Ok(priced)
    }

    /// Drop every route kept so far, so the next ones are fetched afresh.
    pub fn forget(&mut self) {
        self.routes.clear();
    }
}
