//! Orders on the venue, signed by an agent key. An agent key is approved by the
//! account it trades for and can place and cancel orders, never withdraw.
//!
//! An order is a packed action, hashed with its nonce, signed as typed data for
//! a "phantom agent" whose source says mainnet or testnet.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use alloy_primitives::{keccak256, Address, B256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{eip712_domain, sol, SolStruct};
use serde::Serialize;
use serde_json::{json, Value};

sol! {
    struct Agent {
        string source;
        bytes32 connectionId;
    }
}

pub const MAINNET: &str = "https://api.hyperliquid.xyz";
pub const TESTNET: &str = "https://api.hyperliquid-testnet.xyz";

#[derive(Serialize)]
struct Limit {
    tif: &'static str,
}

#[derive(Serialize)]
struct OrderType {
    limit: Limit,
}

/// One order as the venue reads it. Field order matters: it is packed as written.
#[derive(Serialize)]
struct OrderWire {
    a: u32,
    b: bool,
    p: String,
    s: String,
    r: bool,
    t: OrderType,
}

#[derive(Serialize)]
struct OrderAction {
    #[serde(rename = "type")]
    kind: &'static str,
    orders: Vec<OrderWire>,
    grouping: &'static str,
}

/// A number as the venue wants it: at most eight decimals, no trailing zeros.
pub fn wire(x: f64) -> String {
    let text = format!("{x:.8}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" || text.is_empty() {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// A price the venue accepts: five significant figures, and no more decimals than
/// six less the market's size decimals.
pub fn price(px: f64, size_decimals: u32) -> f64 {
    if px <= 0.0 {
        return 0.0;
    }
    let magnitude = px.abs().log10().floor() as i32;
    let significant = 10f64.powi(4 - magnitude);
    let five = (px * significant).round() / significant;
    let decimals = 10f64.powi(6 - size_decimals as i32);
    (five * decimals).round() / decimals
}

pub struct Exchange {
    http: reqwest::blocking::Client,
    url: String,
    signer: PrivateKeySigner,
    mainnet: bool,
}

impl Exchange {
    /// `key` is the agent's private key in hex. `url` is `MAINNET` or `TESTNET`.
    pub fn new(key: &str, url: &str) -> Result<Exchange, String> {
        let signer: PrivateKeySigner = key.trim().parse().map_err(|_| "the agent key is not a private key".to_string())?;
        let http = reqwest::blocking::Client::builder().timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
        Ok(Exchange { http, url: url.to_string(), signer, mainnet: url == MAINNET })
    }

    pub fn agent(&self) -> Address {
        self.signer.address()
    }

    fn sign(&self, action: &impl Serialize, nonce: u64) -> Result<Value, String> {
        let mut data = rmp_serde::to_vec_named(action).map_err(|e| e.to_string())?;
        data.extend_from_slice(&nonce.to_be_bytes());
        data.push(0);
        let agent = Agent { source: if self.mainnet { "a" } else { "b" }.to_string(), connectionId: keccak256(&data) };
        let domain = eip712_domain! { name: "Exchange", version: "1", chain_id: 1337, verifying_contract: Address::ZERO, };
        let hash: B256 = agent.eip712_signing_hash(&domain);
        let signature = self.signer.sign_hash_sync(&hash).map_err(|e| e.to_string())?;
        Ok(json!({
            "r": format!("0x{:064x}", signature.r()),
            "s": format!("0x{:064x}", signature.s()),
            "v": 27 + signature.v() as u8,
        }))
    }

    /// An immediate-or-cancel order: whatever of it fills now at `px` or better fills, the rest is dropped.
    pub fn order(&self, asset: u32, buy: bool, px: f64, size: f64, reduce_only: bool) -> Result<Value, String> {
        let action = OrderAction {
            kind: "order",
            orders: vec![OrderWire { a: asset, b: buy, p: wire(px), s: wire(size), r: reduce_only, t: OrderType { limit: Limit { tif: "Ioc" } } }],
            grouping: "na",
        };
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        let signature = self.sign(&action, nonce)?;
        let body = json!({ "action": action, "nonce": nonce, "signature": signature, "vaultAddress": null });
        let response = self.http.post(format!("{}/exchange", self.url)).json(&body).send().map_err(|e| e.to_string())?;
        let status = response.status();
        let text = response.text().map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|_| format!("{status}: {text}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_prices_are_written_as_the_venue_reads_them() {
        assert_eq!(wire(1.0), "1");
        assert_eq!(wire(0.00012300), "0.000123");
        assert_eq!(wire(-0.0), "0");
        assert_eq!(price(3906.4567, 2), 3906.5);
        assert_eq!(price(77002.51, 5), 77003.0);
        assert_eq!(price(0.123456, 0), 0.12346);
    }

    #[test]
    fn orders_are_signed_exactly_as_the_venue_client_signs_them() {
        let exchange = Exchange::new(concat!("0x01234567890123456789012345678901", "23456789012345678901234567890123"), TESTNET).unwrap();
        let action = OrderAction {
            kind: "order",
            orders: vec![OrderWire { a: 0, b: true, p: wire(100.0), s: wire(0.1), r: false, t: OrderType { limit: Limit { tif: "Ioc" } } }],
            grouping: "na",
        };
        // The venue's own client signs this order, nonce 1, to exactly these values.
        let testnet = exchange.sign(&action, 1).unwrap();
        assert_eq!(testnet["r"], concat!("0xe8692ef081c10ea0a284b8e2c4458e00", "70717b5edd0259c51a495ce88038dd2b"));
        assert_eq!(testnet["s"], concat!("0x122b41f37e6660ad5a8c996318caec3c", "7f7f7e1e01a8f4b738a9c072e1b0f2a3"));
        assert_eq!(testnet["v"], 28);
        let mainnet = Exchange::new(concat!("0x01234567890123456789012345678901", "23456789012345678901234567890123"), MAINNET).unwrap();
        let signed = mainnet.sign(&action, 1).unwrap();
        assert_eq!(signed["r"], concat!("0x80b52e179d153abfb38f27ef80933280", "9f9677d1a3fa1d3530a7c603f5cff330"));
        assert_eq!(signed["s"], concat!("0x7dde6261f4211cbef8570c7b5a504404", "62e5202a65f1697a733939758ab72885"));
    }
}
