// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {Test, console2} from "forge-std/Test.sol";
import {Settlement, Collector, IPriceFeed} from "../src/Settlement.sol";

interface IToken {
    function balanceOf(address account) external view returns (uint256);
    function approve(address spender, uint256 amount) external returns (bool);
}

/// @notice The settlement contract on a private copy of the real chain:
///         real stock token, real cash token, real public price feed, and
///         real swap routes and issuer quotes fetched moments earlier by
///         `live/check.sh`. Skipped unless LIVE=1, so the ordinary test run
///         stays offline.
contract LiveTest is Test {
    address constant FEED = 0x14032815B65f6B65f23d2532ad5F5DFf7BE48C03;

    Settlement settlement;
    Collector collector;
    address cash;
    address token;
    string fixture;

    address admin = makeAddr("admin");
    address operator = makeAddr("operator");
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");

    function setUp() public {
        if (!vm.envOr("LIVE", false)) {
            vm.skip(true);
            return;
        }
        vm.createSelectFork(vm.envString("FORK_URL"));
        fixture = vm.readFile("./test/fixtures/live.json");
        cash = vm.parseJsonAddress(fixture, ".cash");
        token = vm.parseJsonAddress(fixture, ".token");
        address where = vm.parseJsonAddress(fixture, ".settlement");

        // Batch prices must sit within 0.5% of the issuer's live quote, and
        // the quote within 5% of the issuer's public feed.
        deployCodeTo(
            "Settlement.sol:Settlement",
            abi.encode(
                cash, FEED, vm.parseJsonAddress(fixture, ".issuer"), admin, operator, uint256(50), uint256(60), uint256(500), uint256(48 hours)
            ),
            where
        );
        settlement = Settlement(where);
        collector = settlement.collector();
        // A route normally waits two days. The delay is set aside here, because
        // moving this copy of the chain two days ahead would make the real
        // price feed and the fetched routes stale.
        _allowRoute(vm.parseJsonAddress(fixture, ".buy.router"));
        _allowRoute(vm.parseJsonAddress(fixture, ".sell.router"));

        _join(alice, 900_000_000);
        _join(bob, 900_000_000);
    }

    function _allowRoute(address route) private {
        bytes32 slot = keccak256(abi.encode(route, uint256(3))); // isRouter[route]
        vm.store(address(settlement), slot, bytes32(uint256(1)));
        require(settlement.isRouter(route), "the route flag is not where this test expects it");
    }

    /// Give an account cash, approve the collector and open a fund whose target share of the stock is `share`, out of one billion.
    function _join(address who, uint256 share) private {
        deal(cash, who, 1_000e18);
        vm.startPrank(who);
        IToken(cash).approve(address(collector), type(uint256).max);
        IToken(token).approve(address(collector), type(uint256).max);
        vm.stopPrank();
        _open(who, share);
    }

    function _open(address who, uint256 share) private {
        address[] memory tokens = new address[](1);
        tokens[0] = token;
        uint256[] memory targets = new uint256[](1);
        targets[0] = share;
        vm.prank(who);
        settlement.open(tokens, targets, 10_000e18);
    }

    /// A trade through the fixture's `side` route ("buy" or "sell"), held to the issuer quote fetched with it.
    function _trade(string memory side, uint256 net, uint256 cashLimit, Settlement.Fill[] memory buys, Settlement.Fill[] memory sells)
        private
        view
        returns (Settlement.Trade memory trade)
    {
        string memory at = string.concat(".", side);
        trade.token = token;
        trade.netBuy = keccak256(bytes(side)) == keccak256("buy");
        trade.netQuantity = net;
        trade.cashLimit = cashLimit;
        trade.router = vm.parseJsonAddress(fixture, string.concat(at, ".router"));
        trade.routerCall = vm.parseJsonBytes(fixture, string.concat(at, ".data"));
        trade.buys = buys;
        trade.sells = sells;
        trade.quote = _quote(string.concat(at, ".quote"));
        trade.quoteSignature = vm.parseJsonBytes(fixture, string.concat(at, ".signature"));
        console2.log("issuer quote (cents)     ", trade.quote.price / 1e16);
    }

    function _quote(string memory at) private view returns (Settlement.Quote memory quote) {
        quote.chainId = vm.parseJsonUint(fixture, string.concat(at, ".chainId"));
        quote.attestationId = vm.parseJsonUint(fixture, string.concat(at, ".attestationId"));
        quote.userId = vm.parseJsonBytes32(fixture, string.concat(at, ".userId"));
        quote.asset = vm.parseJsonAddress(fixture, string.concat(at, ".asset"));
        quote.price = vm.parseJsonUint(fixture, string.concat(at, ".price"));
        quote.quantity = vm.parseJsonUint(fixture, string.concat(at, ".quantity"));
        quote.expiration = vm.parseJsonUint(fixture, string.concat(at, ".expiration"));
        quote.side = uint8(vm.parseJsonUint(fixture, string.concat(at, ".side")));
        quote.additionalData = vm.parseJsonBytes32(fixture, string.concat(at, ".additionalData"));
    }

    function _settle(Settlement.Trade memory trade) private returns (uint256 gasUsed) {
        Settlement.Trade[] memory trades = new Settlement.Trade[](1);
        trades[0] = trade;
        vm.prank(operator);
        uint256 before = gasleft();
        settlement.settle(trades);
        gasUsed = before - gasleft();
    }

    // Each check runs on its own fresh copy of the chain, with routes fetched
    // seconds before it.

    function _logFeed() private view {
        (uint256 feedPrice, uint64 updated,,) = IPriceFeed(FEED).prices(token);
        console2.log("feed price (cents)       ", feedPrice / 1e16);
        console2.log("feed age (minutes)       ", (block.timestamp - updated) / 60);
    }

    function test_live_twoAccountsBuyTogether() public {
        _logFeed();
        uint256 spend = vm.parseJsonUint(fixture, ".buy.amountIn");
        uint256 net = vm.parseJsonUint(fixture, ".buy.minAmountOut");
        Settlement.Fill[] memory buys = new Settlement.Fill[](2);
        buys[0] = Settlement.Fill(alice, (net * 60) / 100);
        buys[1] = Settlement.Fill(bob, net - (net * 60) / 100);
        uint256 gasUsed = _settle(_trade("buy", net, spend, buys, new Settlement.Fill[](0)));

        uint256 aliceTokens = IToken(token).balanceOf(alice);
        uint256 bobTokens = IToken(token).balanceOf(bob);
        uint256 paid = (1_000e18 - IToken(cash).balanceOf(alice)) + (1_000e18 - IToken(cash).balanceOf(bob));
        assertGe(aliceTokens, buys[0].quantity, "alice got less than her fill");
        assertGe(bobTokens, buys[1].quantity, "bob got less than his fill");
        assertApproxEqAbs(paid, spend, 10, "buyers paid something other than what the market took");
        assertEq(IToken(token).balanceOf(address(settlement)), 0, "tokens left in the contract");
        assertLt(IToken(cash).balanceOf(address(settlement)), 100, "cash left in the contract");
        console2.log("BUY  cash spent (cents)  ", paid / 1e16);
        console2.log("BUY  tokens (millionths) ", (aliceTokens + bobTokens) / 1e12);
        console2.log("BUY  price (cents)       ", ((paid * 1e18) / (aliceTokens + bobTokens)) / 1e16);
        console2.log("BUY  gas                 ", gasUsed);
    }

    /// Alice sells, Bob buys part of it from her, and only the rest goes to market.
    function test_live_oneSellsOneBuysTheRestGoesToMarket() public {
        _logFeed();
        deal(token, alice, 1e18);
        _open(alice, 0); // Alice's fund is getting out of the stock
        uint256 netSell = vm.parseJsonUint(fixture, ".sell.amountIn");
        uint256 aliceSells = netSell + 1e17;
        Settlement.Fill[] memory matchedBuy = new Settlement.Fill[](1);
        matchedBuy[0] = Settlement.Fill(bob, 1e17);
        Settlement.Fill[] memory sells = new Settlement.Fill[](1);
        sells[0] = Settlement.Fill(alice, aliceSells);
        uint256 gasUsed = _settle(_trade("sell", netSell, vm.parseJsonUint(fixture, ".sell.minAmountOut"), matchedBuy, sells));

        uint256 sellPrice = ((IToken(cash).balanceOf(alice) - 1_000e18) * 1e18) / aliceSells;
        uint256 bobPaid = 1_000e18 - IToken(cash).balanceOf(bob);
        assertEq(IToken(token).balanceOf(alice), 1e18 - aliceSells, "alice sold the wrong amount");
        assertEq(IToken(token).balanceOf(bob), 1e17, "bob did not get his matched tokens");
        assertApproxEqAbs(bobPaid, (1e17 * sellPrice) / 1e18, 10, "the matched buyer paid a different price from the seller");
        assertEq(IToken(token).balanceOf(address(settlement)), 0, "tokens left in the contract");
        assertLt(IToken(cash).balanceOf(address(settlement)), 100, "cash left in the contract");
        console2.log("SELL price (cents)       ", sellPrice / 1e16);
        console2.log("SELL matched buyer paid (cents)", bobPaid / 1e16);
        console2.log("SELL gas                 ", gasUsed);
    }

    /// Alice buys, Bob sells part of it to her, and only the rest comes from the market.
    function test_live_oneBuysOneSellsTheRestComesFromMarket() public {
        _logFeed();
        deal(token, bob, 1e18);
        _open(bob, 0); // Bob's fund is getting out of the stock
        uint256 spend = vm.parseJsonUint(fixture, ".buy.amountIn");
        uint256 net = vm.parseJsonUint(fixture, ".buy.minAmountOut");
        Settlement.Fill[] memory buys = new Settlement.Fill[](1);
        buys[0] = Settlement.Fill(alice, net + 1e17);
        Settlement.Fill[] memory matchedSell = new Settlement.Fill[](1);
        matchedSell[0] = Settlement.Fill(bob, 1e17);
        uint256 gasUsed = _settle(_trade("buy", net, spend, buys, matchedSell));

        uint256 bobGot = IToken(cash).balanceOf(bob) - 1_000e18;
        uint256 alicePaid = 1_000e18 - IToken(cash).balanceOf(alice);
        assertEq(IToken(token).balanceOf(bob), 9e17, "bob sold the wrong amount");
        assertGe(IToken(token).balanceOf(alice), net + 1e17, "alice got less than her fill");
        assertApproxEqAbs(alicePaid, spend + bobGot, 10, "alice paid something other than the market's cash plus bob's");
        assertEq(IToken(token).balanceOf(address(settlement)), 0, "tokens left in the contract");
        assertLt(IToken(cash).balanceOf(address(settlement)), 100, "cash left in the contract");
        console2.log("MATCHED BUY price (cents)", (bobGot * 10) / 1e16);
        console2.log("MATCHED BUY seller got (cents)", bobGot / 1e16);
        console2.log("MATCHED BUY gas          ", gasUsed);
    }
}
