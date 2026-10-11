//! The engine compiled for the browser, so anyone can re-run it and check the
//! server's answers for themselves.
//!
//! The interface is JSON in, JSON out. Every answer is either
//! `{"ok": ...}` or `{"error": {"message": ..., "issues": [...]}}`.

use compounder_engine::rebalance::{self, Position, Rules};
use compounder_engine::strategy::{Issue, Strategy};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn failure(message: impl Into<String>, issues: Vec<Issue>) -> Value {
    json!({ "error": { "message": message.into(), "issues": issues } })
}

fn fixed_weights(strategy: &Strategy) -> Result<BTreeMap<String, u64>, Value> {
    let targets = strategy.targets().map_err(|issues| failure("the strategy has problems", issues))?;
    rebalance::fix_weights(&targets).map_err(|reason| failure(reason, Vec::new()))
}

/// A strategy document in; each asset's target in parts per billion out.
pub fn strategy_weights_json(input: &str) -> String {
    let answer = match serde_json::from_str::<Strategy>(input) {
        Err(error) => failure(format!("not a strategy document: {error}"), Vec::new()),
        Ok(strategy) => match fixed_weights(&strategy) {
            Ok(weights) => json!({ "ok": { "weights": weights } }),
            Err(error) => error,
        },
    };
    answer.to_string()
}

#[derive(Deserialize)]
struct PlanRequest {
    strategy: Strategy,
    positions: Vec<Position>,
    rules: Rules,
}

/// A strategy, the holdings and the rules in; the rebalance plan out.
pub fn rebalance_plan_json(input: &str) -> String {
    let answer = match serde_json::from_str::<PlanRequest>(input) {
        Err(error) => failure(format!("not a plan request: {error}"), Vec::new()),
        Ok(request) => match fixed_weights(&request.strategy) {
            Err(error) => error,
            Ok(weights) => match rebalance::plan(&weights, &request.positions, &request.rules) {
                Ok(plan) => json!({ "ok": plan }),
                Err(reason) => failure(reason, Vec::new()),
            },
        },
    };
    answer.to_string()
}

// ---- the boundary the browser calls ----
//
// The page writes its input into memory from `alloc`, calls a function with
// that pointer and length, and reads the answer from the pointer and length
// packed into the returned number. It hands both buffers back with `dealloc`.

/// Reserve `len` bytes for the caller to write into.
#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    Box::into_raw(vec![0u8; len].into_boxed_slice()) as *mut u8
}

/// Release a buffer obtained from `alloc` or returned by a call.
///
/// # Safety
/// `ptr` and `len` must describe exactly one such buffer, not yet released.
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)));
}

/// Run `answer` on the caller's bytes and return where the reply is: the
/// pointer in the high 32 bits, the length in the low 32.
///
/// # Safety
/// `ptr` and `len` must describe a readable buffer.
unsafe fn respond(ptr: *const u8, len: usize, answer: fn(&str) -> String) -> u64 {
    let input = std::slice::from_raw_parts(ptr, len);
    let reply = match std::str::from_utf8(input) {
        Ok(text) => answer(text),
        Err(_) => failure("input is not UTF-8", Vec::new()).to_string(),
    };
    let bytes = reply.into_bytes().into_boxed_slice();
    let reply_len = bytes.len() as u64;
    let reply_ptr = Box::into_raw(bytes) as *mut u8 as u64;
    (reply_ptr << 32) | reply_len
}

/// # Safety
/// `ptr` and `len` must describe a readable buffer.
#[no_mangle]
pub unsafe extern "C" fn strategy_weights(ptr: *const u8, len: usize) -> u64 {
    respond(ptr, len, strategy_weights_json)
}

/// # Safety
/// `ptr` and `len` must describe a readable buffer.
#[no_mangle]
pub unsafe extern "C" fn rebalance_plan(ptr: *const u8, len: usize) -> u64 {
    respond(ptr, len, rebalance_plan_json)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0x00000000000000000000000000000000000000a1:56";
    const B: &str = "0x00000000000000000000000000000000000000b2:56";
    const CASH: &str = "0x00000000000000000000000000000000000000c0:56";

    fn strategy(first: &str, second: &str) -> Value {
        json!({
            "entry": {
                "blockType": "topLevel", "name": "Test", "blockIdForInstance": "top",
                "children": {
                    "blockType": "weight", "weightType": "specified-percentage", "weightings": [first, second],
                    "blockIdForInstance": "w",
                    "children": [
                        { "blockType": "asset", "assetId": A, "blockIdForInstance": "a" },
                        { "blockType": "asset", "assetId": B, "blockIdForInstance": "b" }
                    ]
                }
            },
            "version": "1.0.0",
            "tradingSettings": { "type": "threshold", "triggerPercentage": 10 }
        })
    }

    fn parsed(reply: String) -> Value {
        serde_json::from_str(&reply).expect("the reply is JSON")
    }

    #[test]
    fn weights_come_back_as_exact_parts_per_billion() {
        let reply = parsed(strategy_weights_json(&strategy("70", "30").to_string()));
        assert_eq!(reply["ok"]["weights"][A], 700_000_000);
        assert_eq!(reply["ok"]["weights"][B], 300_000_000);
    }

    #[test]
    fn a_broken_strategy_comes_back_with_its_problems() {
        let reply = parsed(strategy_weights_json(&strategy("70", "20").to_string()));
        assert_eq!(reply["error"]["issues"][0]["code"], "INVALID_WEIGHT_TOTAL");
        let reply = parsed(strategy_weights_json("{"));
        assert!(reply["error"]["message"].as_str().unwrap().starts_with("not a strategy document"));
    }

    #[test]
    fn a_deposit_is_planned_into_the_strategy() {
        let request = json!({
            "strategy": strategy("70", "30"),
            "positions": [
                { "asset_id": CASH, "amount": "1000000000", "decimals": 6, "price": "1000000000000000000" },
                { "asset_id": A, "amount": "0", "decimals": 18, "price": "250000000000000000000" },
                { "asset_id": B, "amount": "0", "decimals": 18, "price": "40000000000000000000" }
            ],
            "rules": { "quote_asset_id": CASH, "trigger_bps": 1000, "min_trade_usd": "1000000000000000000" }
        });
        let reply = parsed(rebalance_plan_json(&request.to_string()));
        let trades = reply["ok"]["trades"].as_array().expect("trades");
        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0]["quote_amount"], "700000000");
        assert_eq!(trades[1]["quote_amount"], "300000000");
    }

    #[test]
    fn the_boundary_round_trips_bytes() {
        let input = strategy("70", "30").to_string();
        let ptr = alloc(input.len());
        unsafe {
            std::ptr::copy_nonoverlapping(input.as_ptr(), ptr, input.len());
            // On a 64-bit test host the pointer does not fit the packed form,
            // so exercise the pieces the packed call is built from.
            let text = std::str::from_utf8(std::slice::from_raw_parts(ptr, input.len())).unwrap();
            assert_eq!(parsed(strategy_weights_json(text))["ok"]["weights"][A], 700_000_000);
            dealloc(ptr, input.len());
        }
    }
}
