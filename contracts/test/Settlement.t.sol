// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {Test} from "forge-std/Test.sol";
import {Settlement, Collector, IPriceFeed, IIssuer} from "../src/Settlement.sol";

contract MockToken {
    uint8 public decimals;
    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    constructor(uint8 decimals_) {
        decimals = decimals_;
    }

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
        totalSupply += amount;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        return true;
    }
}

contract MockFeed is IPriceFeed {
    mapping(address => uint256) public price;
    mapping(address => uint64) public updated;

    function set(address token, uint256 price_) external {
        price[token] = price_;
        updated[token] = uint64(block.timestamp);
    }

    function prices(address token) external view returns (uint256, uint64, uint32, uint16) {
        return (price[token], updated[token], 0, 0);
    }
}

/// @dev The issuer, as far as naming who may sign its quotes.
contract MockIssuer is IIssuer {
    bytes32 public constant getDomainSeparator = keccak256("a test issuer");
    mapping(address => bool) public signers;

    function setSigner(address who, bool allowed) external {
        signers[who] = allowed;
    }

    function hasRole(bytes32 role, address who) external view returns (bool) {
        return role == keccak256("ATTESTATION_SIGNER_ROLE") && signers[who];
    }
}

/// @dev A market that fills at whatever amounts the call names. It pulls
///      `amountIn` from its caller and pays `amountOut` to `recipient`.
contract MockRouter {
    uint256 public lastAmountIn;

    function swap(MockToken tokenIn, MockToken tokenOut, uint256 amountIn, uint256 amountOut, address recipient) external {
        lastAmountIn = amountIn;
        tokenIn.transferFrom(msg.sender, address(this), amountIn);
        tokenOut.mint(recipient, amountOut);
    }

    /// Try to spend a third party's approval, as a hostile route would.
    function raid(MockToken token, address victim, address to, uint256 amount) external {
        token.transferFrom(victim, to, amount);
    }

    /// Try to use the collector directly.
    function raidCollector(Collector collector, address token, address victim, uint256 amount) external {
        collector.collect(token, victim, amount);
    }
}

contract SettlementTest is Test {
    uint256 constant LIMIT = 100_000e18;
    uint256 constant START_CASH = 50_000e18;
    uint256 constant START_STOCK = 100e18;
    uint256 constant PERCENT = 10_000_000; // target shares are out of one billion

    MockToken cash;
    MockToken nvda;
    MockToken amzn;
    MockFeed feed;
    MockIssuer issuer;
    MockRouter router;
    Settlement settlement;
    Collector collector;

    address admin = makeAddr("admin");
    address operator = makeAddr("operator");
    // Each starts with 50,000 cash, 100 NVDA (25,000) and 100 AMZN (20,000): a fund worth 95,000.
    address alice = makeAddr("alice"); // wants more of both: targets 40% and 30%
    address bob = makeAddr("bob"); // wants less of both: targets 10% and 10%
    address carol = makeAddr("carol"); // wants more of both, like alice
    uint256 issuerKey = uint256(keccak256("the issuer's quote signer"));
    uint256 nextQuote = 1;

    function setUp() public {
        vm.warp(1_800_000_000);
        cash = new MockToken(18);
        nvda = new MockToken(18);
        amzn = new MockToken(18);
        feed = new MockFeed();
        issuer = new MockIssuer();
        issuer.setSigner(vm.addr(issuerKey), true);
        router = new MockRouter();
        // Batch prices within 1% of an issuer quote that expires within a minute. The
        // quote within 5% of a feed price no older than 12 hours.
        settlement = new Settlement(address(cash), address(feed), address(issuer), admin, operator, 100, 60, 500, 12 hours);
        collector = settlement.collector();
        vm.prank(admin);
        settlement.proposeRouter(address(router));
        vm.warp(block.timestamp + 2 days);
        settlement.activateRouter(address(router));
        feed.set(address(nvda), 250e18);
        feed.set(address(amzn), 200e18);
        _join(alice, 40 * PERCENT, 30 * PERCENT);
        _join(bob, 10 * PERCENT, 10 * PERCENT);
        _join(carol, 40 * PERCENT, 30 * PERCENT);
    }

    // ---- helpers ----

    /// Fund an account, approve the collector and opt in with target shares.
    function _join(address who, uint256 nvdaShare, uint256 amznShare) private {
        cash.mint(who, START_CASH);
        nvda.mint(who, START_STOCK);
        amzn.mint(who, START_STOCK);
        vm.startPrank(who);
        cash.approve(address(collector), type(uint256).max);
        nvda.approve(address(collector), type(uint256).max);
        amzn.approve(address(collector), type(uint256).max);
        vm.stopPrank();
        _open(who, nvdaShare, amznShare);
    }

    function _open(address who, uint256 nvdaShare, uint256 amznShare) private {
        address[] memory tokens = new address[](2);
        tokens[0] = address(nvda);
        tokens[1] = address(amzn);
        uint256[] memory targets = new uint256[](2);
        targets[0] = nvdaShare;
        targets[1] = amznShare;
        vm.prank(who);
        settlement.open(tokens, targets, LIMIT);
    }

    function _openOne(address who, MockToken token, uint256 share) private {
        address[] memory tokens = new address[](1);
        tokens[0] = address(token);
        uint256[] memory targets = new uint256[](1);
        targets[0] = share;
        vm.prank(who);
        settlement.open(tokens, targets, LIMIT);
    }

    function _fills(address a, uint256 qa) private pure returns (Settlement.Fill[] memory fills) {
        fills = new Settlement.Fill[](1);
        fills[0] = Settlement.Fill(a, qa);
    }

    function _none() private pure returns (Settlement.Fill[] memory) {
        return new Settlement.Fill[](0);
    }

    function _ceil(uint256 quantity, uint256 price) private pure returns (uint256) {
        return (quantity * price + 1e18 - 1) / 1e18;
    }

    function _ceilDiv(uint256 a, uint256 b) private pure returns (uint256) {
        return a == 0 ? 0 : (a - 1) / b + 1;
    }

    function _sign(Settlement.Quote memory quote, uint256 key) private view returns (bytes memory) {
        bytes32 typeHash = keccak256(
            "Quote(uint256 chainId,uint256 attestationId,bytes32 userId,address asset,uint256 price,uint256 quantity,uint256 expiration,uint8 side,bytes32 additionalData)"
        );
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", issuer.getDomainSeparator(), keccak256(abi.encode(typeHash, quote))));
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, digest);
        return abi.encodePacked(r, s, v);
    }

    /// Price a trade against a fresh issuer quote at `price`.
    function _requote(Settlement.Trade memory trade, uint256 price) private {
        trade.quote = Settlement.Quote(
            block.chainid, nextQuote++, "the route", trade.token, price, trade.netQuantity, block.timestamp + 15, trade.netBuy ? 0 : 1, 0
        );
        trade.quoteSignature = _sign(trade.quote, issuerKey);
    }

    /// The market leg: a swap of `amountIn` for `amountOut`, paid to the settlement contract.
    function _swap(MockToken tokenIn, MockToken tokenOut, uint256 amountIn, uint256 amountOut) private view returns (bytes memory) {
        return abi.encodeCall(MockRouter.swap, (tokenIn, tokenOut, amountIn, amountOut, address(settlement)));
    }

    /// A batch whose market leg buys `net` tokens for exactly `spend` cash, on an issuer quote at the feed price.
    function _netBuy(MockToken token, uint256 net, uint256 spend, Settlement.Fill[] memory buys, Settlement.Fill[] memory sells)
        private
        returns (Settlement.Trade memory trade)
    {
        trade.token = address(token);
        trade.netBuy = true;
        trade.netQuantity = net;
        trade.cashLimit = spend;
        trade.router = address(router);
        trade.buys = buys;
        trade.sells = sells;
        _requote(trade, feed.price(address(token)));
        trade.routerCall = _swap(cash, token, spend, net);
    }

    /// A batch whose market leg sells `net` tokens for exactly `proceeds` cash, on an issuer quote at the feed price.
    function _netSell(MockToken token, uint256 net, uint256 proceeds, Settlement.Fill[] memory buys, Settlement.Fill[] memory sells)
        private
        returns (Settlement.Trade memory trade)
    {
        trade.token = address(token);
        trade.netQuantity = net;
        trade.cashLimit = proceeds;
        trade.router = address(router);
        trade.buys = buys;
        trade.sells = sells;
        _requote(trade, feed.price(address(token)));
        trade.routerCall = _swap(token, cash, net, proceeds);
    }

    function _settle(Settlement.Trade memory trade) private {
        Settlement.Trade[] memory trades = new Settlement.Trade[](1);
        trades[0] = trade;
        vm.prank(operator);
        settlement.settle(trades);
    }

    function _assertHoldsNothing() private view {
        assertEq(nvda.balanceOf(address(settlement)), 0, "settlement keeps stock tokens");
        assertEq(amzn.balanceOf(address(settlement)), 0, "settlement keeps stock tokens");
        assertLt(cash.balanceOf(address(settlement)), 100, "settlement keeps more than rounding dust");
        assertEq(cash.allowance(address(settlement), address(router)), 0, "an approval is left standing");
        assertEq(nvda.allowance(address(settlement), address(router)), 0, "an approval is left standing");
    }

    function _assertUntouched(address who) private view {
        assertEq(cash.balanceOf(who), START_CASH);
        assertEq(nvda.balanceOf(who), START_STOCK);
        assertEq(amzn.balanceOf(who), START_STOCK);
    }

    // ---- what a batch does ----

    function test_buy_paysTheMarketPriceAndDeliversTheTokens() public {
        _settle(_netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none()));
        assertEq(cash.balanceOf(alice), START_CASH - 500e18);
        assertEq(nvda.balanceOf(alice), START_STOCK + 2e18);
        _assertHoldsNothing();
    }

    function test_sell_paysTheMarketPriceForTheTokens() public {
        _settle(_netSell(nvda, 2e18, 500e18, _none(), _fills(bob, 2e18)));
        assertEq(cash.balanceOf(bob), START_CASH + 500e18);
        assertEq(nvda.balanceOf(bob), START_STOCK - 2e18);
        _assertHoldsNothing();
    }

    function test_buysAndSellsAreMatched_onlyTheDifferenceGoesToMarket() public {
        // Alice buys 3, Bob sells 1: the market only sees a buy of 2.
        _settle(_netBuy(nvda, 2e18, 500e18, _fills(alice, 3e18), _fills(bob, 1e18)));
        assertEq(router.lastAmountIn(), 500e18, "the market saw more than the difference");
        assertEq(nvda.balanceOf(alice), START_STOCK + 3e18);
        assertEq(cash.balanceOf(alice), START_CASH - 750e18);
        assertEq(nvda.balanceOf(bob), START_STOCK - 1e18);
        assertEq(cash.balanceOf(bob), START_CASH + 250e18, "the seller gets the same price the buyer paid");
        _assertHoldsNothing();
    }

    function test_matchedBatch_canNetToASell() public {
        _settle(_netSell(nvda, 4e18, 1_000e18, _fills(alice, 1e18), _fills(bob, 5e18)));
        assertEq(router.lastAmountIn(), 4e18);
        assertEq(nvda.balanceOf(alice), START_STOCK + 1e18);
        assertEq(cash.balanceOf(alice), START_CASH - 250e18);
        assertEq(cash.balanceOf(bob), START_CASH + 1_250e18);
        _assertHoldsNothing();
    }

    function test_severalTokensSettleInOneTransaction() public {
        Settlement.Trade[] memory trades = new Settlement.Trade[](2);
        trades[0] = _netSell(nvda, 1e18, 250e18, _none(), _fills(bob, 1e18));
        trades[1] = _netBuy(amzn, 1e18, 200e18, _fills(alice, 1e18), _none());
        vm.prank(operator);
        settlement.settle(trades);
        assertEq(nvda.balanceOf(bob), START_STOCK - 1e18);
        assertEq(cash.balanceOf(bob), START_CASH + 250e18);
        assertEq(amzn.balanceOf(alice), START_STOCK + 1e18);
        assertEq(cash.balanceOf(alice), START_CASH - 200e18);
        _assertHoldsNothing();
    }

    function test_buyersPayWhatTheMarketCharged_notTheLimit() public {
        // The caller allowed up to 251 a token; the market filled at 249.
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 502e18, _fills(alice, 2e18), _none());
        trade.routerCall = _swap(cash, nvda, 498e18, 2e18);
        _settle(trade);
        assertEq(cash.balanceOf(alice), START_CASH - 498e18, "the unused part of the limit was not refunded");
        _assertHoldsNothing();
    }

    function test_aBetterFillThanAskedIsSharedByTheBuyers() public {
        // The route promised 2 tokens for 500 and delivers 2.01.
        Settlement.Fill[] memory buys = new Settlement.Fill[](2);
        buys[0] = Settlement.Fill(alice, 1.5e18);
        buys[1] = Settlement.Fill(carol, 0.5e18);
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 500e18, buys, _none());
        trade.routerCall = _swap(cash, nvda, 500e18, 2.01e18);
        _settle(trade);
        assertEq(nvda.balanceOf(alice), START_STOCK + 1.5e18 + 0.0075e18);
        assertEq(nvda.balanceOf(carol), START_STOCK + 0.5e18 + 0.0025e18);
        assertEq(cash.balanceOf(alice), START_CASH - 375e18);
        assertEq(cash.balanceOf(carol), START_CASH - 125e18);
        _assertHoldsNothing();
    }

    function test_aMatchedSellerGetsThePriceTheMarketGave_notTheWorstAllowed() public {
        // The route promised at least 2 tokens for 502 and delivers 2.008: a price of 250.
        // Bob, matched against Alice, is paid 250 a token, not the 251 the route allowed for.
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 502e18, _fills(alice, 3e18), _fills(bob, 1e18));
        trade.routerCall = _swap(cash, nvda, 502e18, 2.008e18);
        _settle(trade);
        assertEq(cash.balanceOf(bob), START_CASH + 250e18, "the seller was paid something other than the market price");
        assertEq(nvda.balanceOf(alice), START_STOCK + 3.008e18);
        assertEq(cash.balanceOf(alice), START_CASH - 752e18, "the buyer paid something other than the market's cash plus the seller's");
        _assertHoldsNothing();
    }

    function test_rounding_neverLeavesTheContractShort() public {
        feed.set(address(nvda), 241_658_100_000_000_000_333);
        uint256 aliceBuys = 47_140_000_000_000_007;
        uint256 carolBuys = 10_000_000_000_000_003;
        uint256 bobSells = 20_000_000_000_000_001;
        uint256 net = aliceBuys + carolBuys - bobSells;
        uint256 spend = (net * 241_658_100_000_000_000_333) / 1e18 + 7;
        Settlement.Fill[] memory buys = new Settlement.Fill[](2);
        buys[0] = Settlement.Fill(alice, aliceBuys);
        buys[1] = Settlement.Fill(carol, carolBuys);
        _settle(_netBuy(nvda, net, spend, buys, _fills(bob, bobSells)));
        uint256 price = (spend * 1e18 + net - 1) / net;
        uint256 owed = spend + (bobSells * price) / 1e18;
        assertEq(cash.balanceOf(bob), START_CASH + (bobSells * price) / 1e18, "a seller is paid the floor of what it sold");
        assertEq(cash.balanceOf(alice), START_CASH - _ceilDiv(owed * aliceBuys, aliceBuys + carolBuys), "a buyer pays its share, rounded up");
        _assertHoldsNothing();
    }

    // ---- a fill may only move a fund toward its targets ----

    function test_aSellCannotTakeAHoldingBelowItsTarget() public {
        // Bob's NVDA target is 10% of 95,000: 9,500, or 38 tokens. Selling 62 lands exactly there.
        _settle(_netSell(nvda, 62e18, 15_500e18, _none(), _fills(bob, 62e18)));
        assertEq(nvda.balanceOf(bob), 38e18);
    }

    function test_aSellPastTheTargetIsRefused() public {
        // Selling 63 would leave 9,250, below the 9,500 target.
        Settlement.Trade memory trade = _netSell(nvda, 63e18, 15_750e18, _none(), _fills(bob, 63e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.PastTarget.selector, bob, address(nvda), 9_250e18, 9_500e18));
        _settle(trade);
        _assertUntouched(bob);
    }

    function test_aBuyPastTheTargetIsRefused() public {
        // Alice's NVDA target is 40% of 95,000: 38,000, or 152 tokens. She holds 100.
        _settle(_netBuy(nvda, 52e18, 13_000e18, _fills(alice, 52e18), _none()));
        Settlement.Trade memory more = _netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.PastTarget.selector, alice, address(nvda), 38_500e18, 38_000e18));
        _settle(more);
    }

    function test_aHoldingAboveTargetCannotBeBought_norOneBelowSold() public {
        Settlement.Trade memory buy = _netBuy(nvda, 2e18, 500e18, _fills(bob, 2e18), _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.PastTarget.selector, bob, address(nvda), 25_500e18, 9_500e18));
        _settle(buy);
        Settlement.Trade memory sell = _netSell(nvda, 2e18, 500e18, _none(), _fills(alice, 2e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.PastTarget.selector, alice, address(nvda), 24_500e18, 38_000e18));
        _settle(sell);
    }

    function test_aFundCannotBeTradedBackAndForth() public {
        // A hostile operator sells Bob's stock cheaply, then tries to sell it back to him dearer.
        _settle(_netSell(nvda, 30e18, 7_440e18, _none(), _fills(bob, 30e18)));
        Settlement.Trade memory back = _netBuy(nvda, 30e18, 7_560e18, _fills(bob, 30e18), _none());
        // His fund is now worth 94,940, so his target is 9,494. He holds 17,500 and cannot buy.
        vm.expectRevert(abi.encodeWithSelector(Settlement.PastTarget.selector, bob, address(nvda), 25_000e18, 9_494e18));
        _settle(back);
    }

    function test_aFundOnTargetCannotBeTraded() public {
        address erin = makeAddr("erin");
        cash.mint(erin, 75_000e18);
        nvda.mint(erin, 100e18);
        vm.startPrank(erin);
        cash.approve(address(collector), type(uint256).max);
        nvda.approve(address(collector), type(uint256).max);
        vm.stopPrank();
        _openOne(erin, nvda, 25 * PERCENT); // 25,000 of 100,000: exactly on target

        Settlement.Trade memory sell = _netSell(nvda, 0.01e18, 2.5e18, _none(), _fills(erin, 0.01e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.PastTarget.selector, erin, address(nvda), 24_997.5e18, 25_000e18));
        _settle(sell);
        Settlement.Trade memory buy = _netBuy(nvda, 0.01e18, 2.5e18, _fills(erin, 0.01e18), _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.PastTarget.selector, erin, address(nvda), 25_002.5e18, 25_000e18));
        _settle(buy);
    }

    function test_aBuyerIsValuedBeforeItsCashIsCollected() public {
        // Alice may buy up to 52 tokens: 40% of a fund worth 95,000. Valuing her
        // after her 13,000 had been collected would wrongly shrink that target.
        _settle(_netBuy(nvda, 52e18, 13_000e18, _fills(alice, 52e18), _none()));
        assertEq(nvda.balanceOf(alice), 152e18);
    }

    function test_theSameFundListedTwiceCannotSlipPastItsTarget() public {
        Settlement.Fill[] memory buys = new Settlement.Fill[](2);
        buys[0] = Settlement.Fill(alice, 30e18);
        buys[1] = Settlement.Fill(alice, 30e18); // 60 in all, against room for 52
        Settlement.Trade memory trade = _netBuy(nvda, 60e18, 15_000e18, buys, _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.PastTarget.selector, alice, address(nvda), 40_000e18, 38_000e18));
        _settle(trade);
        _assertUntouched(alice);
    }

    function test_aBetterFillThanAskedCannotPushABuyerPastItsTarget() public {
        // Alice asks for exactly the 52 tokens she has room for; the market delivers 52.1.
        Settlement.Trade memory trade = _netBuy(nvda, 52e18, 13_000e18, _fills(alice, 52e18), _none());
        trade.routerCall = _swap(cash, nvda, 13_000e18, 52.1e18);
        vm.expectRevert(abi.encodeWithSelector(Settlement.PastTarget.selector, alice, address(nvda), 38_025e18, 38_000e18));
        _settle(trade);
    }

    function test_openingAgainReplacesTheWholeSetup() public {
        _openOne(bob, amzn, 10 * PERCENT);
        Settlement.Trade memory trade = _netSell(nvda, 1e18, 250e18, _none(), _fills(bob, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.TokenNotListed.selector, bob, address(nvda)));
        _settle(trade);
        (bool isListed,) = settlement.targetOf(bob, address(nvda));
        assertFalse(isListed);
        assertEq(settlement.tokensOf(bob).length, 1);
    }

    function test_badSetupsAreRefused() public {
        address[] memory tokens = new address[](2);
        tokens[0] = address(nvda);
        tokens[1] = address(nvda);
        uint256[] memory targets = new uint256[](2);
        targets[0] = 10 * PERCENT;
        targets[1] = 10 * PERCENT;
        vm.startPrank(alice);
        vm.expectRevert(Settlement.BadTargets.selector); // a token listed twice
        settlement.open(tokens, targets, LIMIT);
        tokens[1] = address(amzn);
        targets[0] = 60 * PERCENT;
        targets[1] = 50 * PERCENT;
        vm.expectRevert(Settlement.BadTargets.selector); // shares above 100%
        settlement.open(tokens, targets, LIMIT);
        tokens[1] = address(cash);
        targets[1] = 10 * PERCENT;
        vm.expectRevert(Settlement.BadTargets.selector); // cash is what is left over, never listed
        settlement.open(tokens, targets, LIMIT);
        vm.stopPrank();
    }

    function test_aTokenTheFeedDoesNotPriceCannotBeListed() public {
        MockToken unknown = new MockToken(18);
        address[] memory tokens = new address[](1);
        tokens[0] = address(unknown);
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(Settlement.PriceUnavailable.selector, address(unknown)));
        settlement.open(tokens, new uint256[](1), LIMIT);
    }

    function test_aTokenWithOtherDecimalsCannotBeListed() public {
        MockToken six = new MockToken(6);
        address[] memory tokens = new address[](1);
        tokens[0] = address(six);
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(Settlement.WrongDecimals.selector, address(six)));
        settlement.open(tokens, new uint256[](1), LIMIT);
    }

    // ---- the price is checked, not trusted ----

    function test_aBuyAboveTheBandIsUndone() public {
        // The feed says 250. 1% above is 252.50; the market charges 255.
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 510e18, _fills(alice, 2e18), _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.PriceOutOfBand.selector, 255e18, 247.5e18, 252.5e18));
        _settle(trade);
        _assertUntouched(alice);
    }

    function test_roomForSlippageDoesNotCountAgainstTheBand() public {
        // The caller allows up to 255 a token, above the band, but the market charges 252.
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 510e18, _fills(alice, 2e18), _none());
        trade.routerCall = _swap(cash, nvda, 504e18, 2e18);
        _settle(trade);
        assertEq(cash.balanceOf(alice), START_CASH - 504e18);
        _assertHoldsNothing();
    }

    function test_aTokenCannotSettleTwiceInOneBatch() public {
        Settlement.Trade[] memory trades = new Settlement.Trade[](2);
        trades[0] = _netSell(nvda, 1e18, 248e18, _none(), _fills(bob, 1e18));
        trades[1] = _netBuy(nvda, 1e18, 252e18, _fills(alice, 1e18), _none());
        vm.prank(operator);
        vm.expectRevert(abi.encodeWithSelector(Settlement.TokenTwice.selector, address(nvda)));
        settlement.settle(trades);
        // The same token in the next batch is fine.
        _settle(_netSell(nvda, 1e18, 248e18, _none(), _fills(bob, 1e18)));
        _settle(_netBuy(nvda, 1e18, 252e18, _fills(alice, 1e18), _none()));
    }

    function test_aSaleBelowTheBandIsUndone() public {
        // The market pays 240 a token; 1% below 250 is 247.50.
        Settlement.Trade memory trade = _netSell(nvda, 2e18, 480e18, _none(), _fills(bob, 2e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.PriceOutOfBand.selector, 240e18, 247.5e18, 252.5e18));
        _settle(trade);
        _assertUntouched(bob);
    }

    function test_aMatchedSellerIsProtectedFromACheapMarketLeg() public {
        // A buy filled far below the band would short-change the matched seller.
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 400e18, _fills(alice, 3e18), _fills(bob, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.PriceOutOfBand.selector, 200e18, 247.5e18, 252.5e18));
        _settle(trade);
        _assertUntouched(bob);
    }

    function test_aStaleFeedStopsTrading() public {
        vm.warp(block.timestamp + 12 hours + 1);
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.PriceUnavailable.selector, address(nvda)));
        _settle(trade);
    }

    function test_aFundCannotBeTradedWhileAnyOfItsPricesIsStale() public {
        vm.warp(block.timestamp + 12 hours + 1);
        feed.set(address(nvda), 250e18); // AMZN's price is now too old to value Alice's fund
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.PriceUnavailable.selector, address(amzn)));
        _settle(trade);
    }

    function test_aTokenWithNoFeedPriceCannotBeTraded() public {
        MockToken other = new MockToken(18);
        Settlement.Trade memory trade = _netBuy(other, 2e18, 500e18, _fills(alice, 2e18), _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.PriceUnavailable.selector, address(other)));
        _settle(trade);
    }

    // ---- the price is held to the issuer's own quote ----

    function test_theBatchPriceIsHeldToTheQuote_notTheSlowFeed() public {
        // The feed still says 250, but the issuer now quotes 255. A fill at 255 is honest.
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 510e18, _fills(alice, 2e18), _none());
        _requote(trade, 255e18);
        trade.routerCall = _swap(cash, nvda, 510e18, 2e18);
        _settle(trade);
        assertEq(cash.balanceOf(alice), START_CASH - 510e18);

        // A fill at the feed's 250 is now 2% from the quote, and is undone.
        Settlement.Trade memory cheap = _netBuy(nvda, 2e18, 500e18, _fills(alice, 3e18), _fills(bob, 1e18));
        _requote(cheap, 255e18);
        cheap.routerCall = _swap(cash, nvda, 500e18, 2e18);
        vm.expectRevert(abi.encodeWithSelector(Settlement.PriceOutOfBand.selector, 250e18, 252.45e18, 257.55e18));
        _settle(cheap);
    }

    function test_aQuoteFarFromTheFeedIsRefused() public {
        // 5% above the feed's 250 is 262.50.
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 530e18, _fills(alice, 2e18), _none());
        _requote(trade, 265e18);
        trade.routerCall = _swap(cash, nvda, 530e18, 2e18);
        vm.expectRevert(abi.encodeWithSelector(Settlement.QuoteOutOfBand.selector, 265e18, 250e18));
        _settle(trade);
        _assertUntouched(alice);
    }

    function test_aQuoteTheIssuerDidNotSignIsRefused() public {
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none());
        trade.quoteSignature = _sign(trade.quote, uint256(keccak256("somebody else")));
        vm.expectRevert(Settlement.QuoteNotSigned.selector);
        _settle(trade);

        // A real signature does not cover a quote changed afterwards.
        trade.quoteSignature = _sign(trade.quote, issuerKey);
        trade.quote.price = 249e18;
        vm.expectRevert(Settlement.QuoteNotSigned.selector);
        _settle(trade);

        trade.quote.price = 250e18;
        trade.quoteSignature = bytes.concat(trade.quoteSignature, hex"00");
        vm.expectRevert(Settlement.QuoteNotSigned.selector);
        _settle(trade);
        _assertUntouched(alice);
    }

    function test_onlyOneFormOfASignatureCounts() public {
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none());
        bytes memory signature = trade.quoteSignature;
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly {
            r := mload(add(signature, 32))
            s := mload(add(signature, 64))
            v := byte(0, mload(add(signature, 96)))
        }
        // The same signature mirrored: equally valid to the curve, refused here.
        uint256 order = (uint256(0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFE) << 128) | 0xBAAEDCE6AF48A03BBFD25E8CD0364141;
        trade.quoteSignature = abi.encodePacked(r, bytes32(order - uint256(s)), v == 27 ? uint8(28) : uint8(27));
        vm.expectRevert(Settlement.QuoteNotSigned.selector);
        _settle(trade);
    }

    function test_aSignerTheIssuerDroppedNoLongerCounts() public {
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none());
        issuer.setSigner(vm.addr(issuerKey), false);
        vm.expectRevert(Settlement.QuoteNotSigned.selector);
        _settle(trade);
    }

    function test_aQuoteForAnotherTokenDirectionOrChainIsRefused() public {
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none());
        trade.quote.asset = address(amzn);
        trade.quoteSignature = _sign(trade.quote, issuerKey);
        vm.expectRevert(abi.encodeWithSelector(Settlement.QuoteMismatch.selector, address(nvda)));
        _settle(trade);

        // A quote at which the issuer buys cannot price a batch that buys from it.
        trade.quote.asset = address(nvda);
        trade.quote.side = 1;
        trade.quoteSignature = _sign(trade.quote, issuerKey);
        vm.expectRevert(abi.encodeWithSelector(Settlement.QuoteMismatch.selector, address(nvda)));
        _settle(trade);

        trade.quote.side = 0;
        trade.quote.chainId = block.chainid + 1;
        trade.quoteSignature = _sign(trade.quote, issuerKey);
        vm.expectRevert(abi.encodeWithSelector(Settlement.QuoteMismatch.selector, address(nvda)));
        _settle(trade);
    }

    function test_aQuoteMustBeLive() public {
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none());
        uint256 expiration = trade.quote.expiration;
        vm.warp(expiration + 1);
        vm.expectRevert(abi.encodeWithSelector(Settlement.QuoteNotLive.selector, expiration));
        _settle(trade);

        // Nor may one be dated far ahead, to be kept for a better moment.
        Settlement.Trade memory early = _netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none());
        early.quote.expiration = block.timestamp + 61;
        early.quoteSignature = _sign(early.quote, issuerKey);
        vm.expectRevert(abi.encodeWithSelector(Settlement.QuoteNotLive.selector, block.timestamp + 61));
        _settle(early);
    }

    function test_aSellingBatchIsHeldToAQuoteAtWhichTheIssuerBuys() public {
        Settlement.Trade memory trade = _netSell(nvda, 2e18, 496e18, _none(), _fills(bob, 2e18));
        _requote(trade, 246e18);
        trade.routerCall = _swap(nvda, cash, 2e18, 496e18);
        // 248 a token is 0.8% above the quote of 246: inside the band.
        _settle(trade);
        assertEq(cash.balanceOf(bob), START_CASH + 496e18);

        Settlement.Trade memory low = _netSell(nvda, 2e18, 486e18, _none(), _fills(bob, 2e18));
        _requote(low, 246e18);
        low.routerCall = _swap(nvda, cash, 2e18, 486e18);
        vm.expectRevert(abi.encodeWithSelector(Settlement.PriceOutOfBand.selector, 243e18, 243.54e18, 248.46e18));
        _settle(low);
    }

    function test_theQuoteBandCannotBeWiderThanTheFeedBand() public {
        vm.expectRevert(Settlement.BadConfig.selector);
        new Settlement(address(cash), address(feed), address(issuer), admin, operator, 600, 60, 500, 12 hours);
    }

    // ---- a route cannot take anything ----

    function test_aRouteThatDeliversNothingIsUndone() public {
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none());
        trade.routerCall = _swap(cash, nvda, 500e18, 0);
        vm.expectRevert(abi.encodeWithSelector(Settlement.MarketLegShort.selector, 2e18, 0));
        _settle(trade);
        _assertUntouched(alice);
    }

    function test_aRouteThatPaysSomeoneElseIsUndone() public {
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none());
        trade.routerCall = abi.encodeCall(MockRouter.swap, (cash, nvda, 500e18, 2e18, makeAddr("thief")));
        vm.expectRevert(abi.encodeWithSelector(Settlement.MarketLegShort.selector, 2e18, 0));
        _settle(trade);
        _assertUntouched(alice);
    }

    function test_aRouteCannotSpendAFundsApproval() public {
        // Bob approved the collector, not the settlement contract or any route.
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none());
        trade.routerCall = abi.encodeCall(MockRouter.raid, (cash, bob, makeAddr("thief"), 1_000e18));
        vm.expectRevert();
        _settle(trade);
        _assertUntouched(bob);
    }

    function test_aRouteCannotUseTheCollector() public {
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none());
        trade.routerCall = abi.encodeCall(MockRouter.raidCollector, (collector, address(cash), bob, 1_000e18));
        vm.expectRevert(Collector.NotSettlement.selector);
        _settle(trade);
        _assertUntouched(bob);
    }

    function test_onlyTheSettlementContractCanCollect() public {
        vm.prank(operator);
        vm.expectRevert(Collector.NotSettlement.selector);
        collector.collect(address(cash), bob, 1e18);
    }

    function test_anUnapprovedRouteIsRefused() public {
        MockRouter other = new MockRouter();
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none());
        trade.router = address(other);
        vm.expectRevert(abi.encodeWithSelector(Settlement.RouterNotAllowed.selector, address(other)));
        _settle(trade);
    }

    function test_theCollectorAndTheCashTokenCanNeverBeRoutes() public {
        vm.startPrank(admin);
        vm.expectRevert(abi.encodeWithSelector(Settlement.RouterNotAllowed.selector, address(collector)));
        settlement.proposeRouter(address(collector));
        vm.expectRevert(abi.encodeWithSelector(Settlement.RouterNotAllowed.selector, address(cash)));
        settlement.proposeRouter(address(cash));
        vm.expectRevert(abi.encodeWithSelector(Settlement.RouterNotAllowed.selector, address(settlement)));
        settlement.proposeRouter(address(settlement));
        vm.stopPrank();
    }

    function test_theTokenBeingTradedCannotBeItsOwnRoute() public {
        vm.prank(admin);
        settlement.proposeRouter(address(nvda));
        vm.warp(block.timestamp + 2 days);
        settlement.activateRouter(address(nvda));
        Settlement.Trade memory trade = _netSell(nvda, 1e18, 250e18, _none(), _fills(bob, 1e18));
        trade.router = address(nvda);
        trade.routerCall = abi.encodeCall(MockToken.transfer, (makeAddr("thief"), 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.RouterNotAllowed.selector, address(nvda)));
        _settle(trade);
    }

    // ---- what the operator cannot do ----

    function test_onlyTheOperatorCanSettle() public {
        Settlement.Trade[] memory trades = new Settlement.Trade[](1);
        trades[0] = _netBuy(nvda, 1e18, 250e18, _fills(alice, 1e18), _none());
        vm.prank(alice);
        vm.expectRevert(Settlement.NotOperator.selector);
        settlement.settle(trades);
    }

    function test_anAccountThatNeverOptedInCannotBeTraded() public {
        address dave = makeAddr("dave");
        nvda.mint(dave, 10e18);
        vm.prank(dave);
        nvda.approve(address(collector), type(uint256).max);
        Settlement.Trade memory trade = _netSell(nvda, 1e18, 250e18, _none(), _fills(dave, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.AccountNotOpen.selector, dave));
        _settle(trade);
        assertEq(nvda.balanceOf(dave), 10e18);
    }

    function test_aTokenTheFundDidNotListCannotBeTraded() public {
        MockToken other = new MockToken(18);
        feed.set(address(other), 50e18);
        other.mint(alice, 10e18);
        vm.prank(alice);
        other.approve(address(collector), type(uint256).max);
        Settlement.Trade memory trade = _netSell(other, 1e18, 50e18, _none(), _fills(alice, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.TokenNotListed.selector, alice, address(other)));
        _settle(trade);
    }

    function test_theDailyLimitCapsTradedValue_andRefillsEvenly() public {
        vm.prank(alice);
        settlement.setDailyLimit(600e18);
        _settle(_netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none()));

        Settlement.Trade memory second = _netBuy(nvda, 1e18, 250e18, _fills(alice, 1e18), _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.DailyLimitExceeded.selector, alice, 250e18, 100e18));
        _settle(second);

        // Half a day later half the limit has come back: 100 left plus 300.
        vm.warp(block.timestamp + 12 hours);
        feed.set(address(nvda), 250e18);
        feed.set(address(amzn), 200e18);
        Settlement.Trade memory tooMuch = _netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.DailyLimitExceeded.selector, alice, 500e18, 400e18));
        _settle(tooMuch);
        _settle(_netBuy(nvda, 1e18, 250e18, _fills(alice, 1e18), _none()));
        assertEq(nvda.balanceOf(alice), START_STOCK + 3e18);
    }

    function test_theDailyLimitCapsSellersToo() public {
        vm.prank(bob);
        settlement.setDailyLimit(600e18);
        Settlement.Trade memory trade = _netSell(nvda, 3e18, 750e18, _none(), _fills(bob, 3e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.DailyLimitExceeded.selector, bob, 750e18, 600e18));
        _settle(trade);
        _assertUntouched(bob);
    }

    function test_aMatchedBuyerInASellingBatchCannotPassItsTarget() public {
        // Alice has room for 52 tokens. Bob sells 60: 53 to her and 7 to the market.
        Settlement.Trade memory trade = _netSell(nvda, 7e18, 1_750e18, _fills(alice, 53e18), _fills(bob, 60e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.PastTarget.selector, alice, address(nvda), 38_250e18, 38_000e18));
        _settle(trade);
        _assertUntouched(alice);
        _assertUntouched(bob);
    }

    function test_theLimitDoesNotDoubleAcrossMidnight() public {
        vm.prank(alice);
        settlement.setDailyLimit(600e18);
        // One second before midnight UTC, then one second after.
        vm.warp((block.timestamp / 1 days + 1) * 1 days - 1);
        feed.set(address(nvda), 250e18);
        feed.set(address(amzn), 200e18);
        _settle(_netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none()));
        vm.warp(block.timestamp + 2);
        Settlement.Trade memory again = _netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none());
        vm.expectRevert();
        _settle(again);
    }

    function test_theStatedDifferenceMustMatchTheFills() public {
        Settlement.Trade memory trade = _netBuy(nvda, 3e18, 750e18, _fills(alice, 3e18), _fills(bob, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.NetQuantityMismatch.selector, 2e18, 3e18));
        _settle(trade);
    }

    function test_aBatchWithNoMarketLegIsRefused_soThereIsAlwaysARealPrice() public {
        Settlement.Trade memory trade = _netBuy(nvda, 0, 0, _fills(alice, 1e18), _fills(bob, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.NetQuantityMismatch.selector, 0, 0));
        _settle(trade);
    }

    function test_oneFailingTradeUndoesTheWholeBatch() public {
        Settlement.Trade[] memory trades = new Settlement.Trade[](2);
        trades[0] = _netSell(nvda, 1e18, 250e18, _none(), _fills(bob, 1e18));
        trades[1] = _netBuy(amzn, 1e18, 150e18, _fills(alice, 1e18), _none()); // far below the band
        vm.prank(operator);
        vm.expectRevert();
        settlement.settle(trades);
        _assertUntouched(alice);
        _assertUntouched(bob);
        _assertHoldsNothing();
    }

    function test_anEmptyFillIsRefused() public {
        Settlement.Fill[] memory buys = new Settlement.Fill[](2);
        buys[0] = Settlement.Fill(alice, 1e18);
        buys[1] = Settlement.Fill(carol, 0);
        Settlement.Trade memory trade = _netBuy(nvda, 1e18, 250e18, buys, _none());
        vm.expectRevert(Settlement.EmptyFill.selector);
        _settle(trade);
    }

    // ---- what the fund controls ----

    function test_closingStopsAllTradingAtOnce() public {
        vm.prank(bob);
        settlement.close();
        Settlement.Trade memory trade = _netSell(nvda, 1e18, 250e18, _none(), _fills(bob, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.AccountNotOpen.selector, bob));
        _settle(trade);
        _assertUntouched(bob);
    }

    function test_revokingAnApprovalStopsThatTokenAtOnce() public {
        vm.prank(bob);
        nvda.approve(address(collector), 0);
        Settlement.Trade memory trade = _netSell(nvda, 1e18, 250e18, _none(), _fills(bob, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Collector.TokenCallFailed.selector, address(nvda)));
        _settle(trade);
    }

    // ---- administration ----

    function test_aNewOperatorWaitsTwoDays_andRemovalIsInstant() public {
        address replacement = makeAddr("replacement");
        vm.prank(operator);
        vm.expectRevert(Settlement.NotAdmin.selector);
        settlement.proposeOperator(replacement);

        vm.prank(admin);
        settlement.proposeOperator(replacement);
        vm.expectRevert(Settlement.NotReady.selector);
        settlement.activateOperator();
        assertEq(settlement.operator(), operator, "the new operator acted before the delay");

        vm.warp(block.timestamp + 2 days);
        settlement.activateOperator();
        assertEq(settlement.operator(), replacement);

        vm.prank(admin);
        settlement.removeOperator();
        Settlement.Trade[] memory trades = new Settlement.Trade[](1);
        trades[0] = _netBuy(nvda, 1e18, 250e18, _fills(alice, 1e18), _none());
        vm.prank(replacement);
        vm.expectRevert(Settlement.NotOperator.selector);
        settlement.settle(trades);
    }

    function test_onlyTheAdminCanRemoveAnOperatorOrARoute() public {
        vm.startPrank(operator);
        vm.expectRevert(Settlement.NotAdmin.selector);
        settlement.removeOperator();
        vm.expectRevert(Settlement.NotAdmin.selector);
        settlement.removeRouter(address(router));
        vm.stopPrank();
        assertEq(settlement.operator(), operator);
        assertTrue(settlement.isRouter(address(router)));
    }

    function test_nobodyCanActivateAnOperatorThatWasNeverProposed() public {
        vm.warp(block.timestamp + 30 days);
        vm.expectRevert(Settlement.NotReady.selector);
        settlement.activateOperator();
        assertEq(settlement.operator(), operator);
    }

    function test_removingTheOperatorCancelsAWaitingReplacement() public {
        vm.startPrank(admin);
        settlement.proposeOperator(makeAddr("replacement"));
        settlement.removeOperator();
        vm.stopPrank();
        vm.warp(block.timestamp + 2 days);
        vm.expectRevert(Settlement.NotReady.selector);
        settlement.activateOperator();
        assertEq(settlement.operator(), address(0));
    }

    function test_theAdminCannotSettle_orMakeItselfOperatorAtOnce() public {
        Settlement.Trade[] memory trades = new Settlement.Trade[](1);
        trades[0] = _netBuy(nvda, 1e18, 250e18, _fills(alice, 1e18), _none());
        vm.startPrank(admin);
        vm.expectRevert(Settlement.NotOperator.selector);
        settlement.settle(trades);
        settlement.proposeOperator(admin);
        vm.expectRevert(Settlement.NotReady.selector);
        settlement.activateOperator();
        vm.expectRevert(Settlement.NotOperator.selector);
        settlement.settle(trades);
        vm.stopPrank();
    }

    function test_aNewRouteWaitsTwoDays_andRemovalIsInstant() public {
        MockRouter other = new MockRouter();
        vm.prank(operator);
        vm.expectRevert(Settlement.NotAdmin.selector);
        settlement.proposeRouter(address(other));

        vm.prank(admin);
        settlement.proposeRouter(address(other));
        vm.expectRevert(Settlement.NotReady.selector);
        settlement.activateRouter(address(other));
        vm.warp(block.timestamp + 2 days);
        settlement.activateRouter(address(other));
        assertTrue(settlement.isRouter(address(other)));

        vm.prank(admin);
        settlement.removeRouter(address(other));
        assertFalse(settlement.isRouter(address(other)));
        vm.expectRevert(Settlement.NotReady.selector);
        settlement.activateRouter(address(other));
    }

    // ---- any batch ----

    struct Mix {
        Settlement.Fill[] buys;
        Settlement.Fill[] sells;
        uint256[3] buyQty;
        uint256[3] sellQty;
        uint256 bought;
        uint256 sold;
    }

    /// Three funds, each a buyer or a seller of a random quantity. A buyer's
    /// target is all NVDA and a seller's is none, so every fill is toward target.
    function _mix(uint256 seed, uint256 feedPrice) private returns (Mix memory mix) {
        address[3] memory who = [alice, bob, carol];
        Settlement.Fill[] memory buys = new Settlement.Fill[](3);
        Settlement.Fill[] memory sells = new Settlement.Fill[](3);
        uint256 nBuys;
        uint256 nSells;
        for (uint256 i = 0; i < 3; i++) {
            uint256 roll = uint256(keccak256(abi.encode(seed, i)));
            // Keep each fill's value inside what a fund holds and may trade.
            uint256 quantity = bound(roll >> 8, 1e6, 10_000e36 / feedPrice);
            if (quantity > 90e18) quantity = 90e18;
            if (roll % 2 == 0) {
                _openOne(who[i], nvda, 100 * PERCENT);
                mix.buyQty[i] = quantity;
                buys[nBuys++] = Settlement.Fill(who[i], quantity);
                mix.bought += quantity;
            } else {
                _openOne(who[i], nvda, 0);
                mix.sellQty[i] = quantity;
                sells[nSells++] = Settlement.Fill(who[i], quantity);
                mix.sold += quantity;
            }
        }
        assembly {
            mstore(buys, nBuys)
            mstore(sells, nSells)
        }
        mix.buys = buys;
        mix.sells = sells;
    }

    /// `owed` is what buyers share when the batch bought on the market, and zero when it sold.
    function _assertSettledAt(Mix memory mix, uint256 price, uint256 owed) private view {
        address[3] memory who = [alice, bob, carol];
        for (uint256 i = 0; i < 3; i++) {
            if (mix.buyQty[i] != 0) {
                uint256 cost = owed == 0 ? _ceil(mix.buyQty[i], price) : _ceilDiv(owed * mix.buyQty[i], mix.bought);
                assertEq(nvda.balanceOf(who[i]), START_STOCK + mix.buyQty[i], "a buyer did not receive what it paid for");
                assertEq(cash.balanceOf(who[i]), START_CASH - cost, "a buyer paid the wrong amount");
                assertLe(cost, _ceil(mix.buyQty[i], price) + 1, "a buyer paid more than the batch price");
            } else {
                assertEq(nvda.balanceOf(who[i]), START_STOCK - mix.sellQty[i], "a seller gave up the wrong amount");
                assertEq(cash.balanceOf(who[i]), START_CASH + (mix.sellQty[i] * price) / 1e18, "a seller was not paid in full");
            }
        }
        assertEq(nvda.balanceOf(address(settlement)), 0, "settlement keeps stock tokens");
        assertLt(cash.balanceOf(address(settlement)), 1e6, "settlement keeps more than dust");
    }

    /// Whatever the mix of buyers and sellers and wherever the market fills
    /// inside the band, every fund trades at the one batch price and the
    /// contract ends holding nothing but dust.
    function testFuzz_everyFillSettlesAtTheOneBatchPrice(uint256 seed, uint256 feedPrice, uint256 skewBps) public {
        feedPrice = bound(feedPrice, 1e15, 100_000e18);
        skewBps = bound(skewBps, 9_910, 10_090); // the market fills within 0.9% of the feed
        feed.set(address(nvda), feedPrice);
        Mix memory mix = _mix(seed, feedPrice);
        vm.assume(mix.bought != mix.sold);

        if (mix.bought > mix.sold) {
            uint256 net = mix.bought - mix.sold;
            uint256 spend = (net * feedPrice * skewBps) / (1e18 * 10_000);
            vm.assume(spend > 0);
            _settle(_netBuy(nvda, net, spend, mix.buys, mix.sells));
            uint256 price = (spend * 1e18 + net - 1) / net;
            _assertSettledAt(mix, price, spend + (mix.sold * price) / 1e18);
        } else {
            uint256 net = mix.sold - mix.bought;
            uint256 proceeds = (net * feedPrice * skewBps) / (1e18 * 10_000);
            vm.assume(proceeds > 0);
            _settle(_netSell(nvda, net, proceeds, mix.buys, mix.sells));
            _assertSettledAt(mix, (proceeds * 1e18) / net, 0);
        }
    }
}
