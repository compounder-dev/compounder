// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {Test} from "forge-std/Test.sol";
import {Settlement, Collector, IPriceFeed} from "../src/Settlement.sol";

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

    MockToken cash;
    MockToken nvda;
    MockToken amzn;
    MockFeed feed;
    MockRouter router;
    Settlement settlement;
    Collector collector;

    address admin = makeAddr("admin");
    address operator = makeAddr("operator");
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address carol = makeAddr("carol");

    function setUp() public {
        vm.warp(1_800_000_000);
        cash = new MockToken(18);
        nvda = new MockToken(18);
        amzn = new MockToken(18);
        feed = new MockFeed();
        router = new MockRouter();
        // Batch prices must sit within 1% of a feed price no older than 12 hours.
        settlement = new Settlement(address(cash), address(feed), admin, operator, 100, 12 hours);
        collector = settlement.collector();
        vm.prank(admin);
        settlement.setRouter(address(router), true);
        feed.set(address(nvda), 250e18);
        feed.set(address(amzn), 200e18);
        _join(alice);
        _join(bob);
        _join(carol);
    }

    // ---- helpers ----

    /// Fund an account, approve the collector and opt in.
    function _join(address who) private {
        cash.mint(who, START_CASH);
        nvda.mint(who, START_STOCK);
        amzn.mint(who, START_STOCK);
        address[] memory tokens = new address[](2);
        tokens[0] = address(nvda);
        tokens[1] = address(amzn);
        vm.startPrank(who);
        cash.approve(address(collector), type(uint256).max);
        nvda.approve(address(collector), type(uint256).max);
        amzn.approve(address(collector), type(uint256).max);
        settlement.open(tokens, LIMIT);
        vm.stopPrank();
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

    /// A batch whose market leg buys `net` tokens for exactly `spend` cash.
    function _netBuy(MockToken token, uint256 net, uint256 spend, Settlement.Fill[] memory buys, Settlement.Fill[] memory sells)
        private
        view
        returns (Settlement.Trade memory)
    {
        bytes memory call = abi.encodeCall(MockRouter.swap, (cash, token, spend, net, address(settlement)));
        return Settlement.Trade(address(token), true, net, spend, address(router), call, buys, sells);
    }

    /// A batch whose market leg sells `net` tokens for exactly `proceeds` cash.
    function _netSell(MockToken token, uint256 net, uint256 proceeds, Settlement.Fill[] memory buys, Settlement.Fill[] memory sells)
        private
        view
        returns (Settlement.Trade memory)
    {
        bytes memory call = abi.encodeCall(MockRouter.swap, (token, cash, net, proceeds, address(settlement)));
        return Settlement.Trade(address(token), false, net, proceeds, address(router), call, buys, sells);
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
        trades[0] = _netSell(nvda, 1e18, 250e18, _none(), _fills(alice, 1e18));
        trades[1] = _netBuy(amzn, 1e18, 200e18, _fills(alice, 1e18), _none());
        vm.prank(operator);
        settlement.settle(trades);
        assertEq(nvda.balanceOf(alice), START_STOCK - 1e18);
        assertEq(amzn.balanceOf(alice), START_STOCK + 1e18);
        assertEq(cash.balanceOf(alice), START_CASH + 250e18 - 200e18);
        _assertHoldsNothing();
    }

    function test_buyersPayWhatTheMarketCharged_notTheLimit() public {
        // The caller allowed up to 251 a token; the market filled at 249.
        bytes memory call = abi.encodeCall(MockRouter.swap, (cash, nvda, 498e18, 2e18, address(settlement)));
        _settle(Settlement.Trade(address(nvda), true, 2e18, 502e18, address(router), call, _fills(alice, 2e18), _none()));
        assertEq(cash.balanceOf(alice), START_CASH - 498e18, "the unused part of the limit was not refunded");
        _assertHoldsNothing();
    }

    function test_aBetterFillThanAskedIsSharedByTheBuyers() public {
        // The market delivers 2.2 tokens for the price of 2.
        bytes memory call = abi.encodeCall(MockRouter.swap, (cash, nvda, 500e18, 2.2e18, address(settlement)));
        Settlement.Fill[] memory buys = new Settlement.Fill[](2);
        buys[0] = Settlement.Fill(alice, 1.5e18);
        buys[1] = Settlement.Fill(carol, 0.5e18);
        _settle(Settlement.Trade(address(nvda), true, 2e18, 500e18, address(router), call, buys, _none()));
        assertEq(nvda.balanceOf(alice), START_STOCK + 1.5e18 + 0.15e18);
        assertEq(nvda.balanceOf(carol), START_STOCK + 0.5e18 + 0.05e18);
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
        assertEq(cash.balanceOf(bob), START_CASH + (bobSells * price) / 1e18, "a seller is paid the floor of what it sold");
        assertEq(cash.balanceOf(alice), START_CASH - _ceil(aliceBuys, price), "a buyer pays the ceiling");
        _assertHoldsNothing();
    }

    // ---- the price is checked, not trusted ----

    function test_aBuyLimitAboveTheBandIsRefusedBeforeAnyTrade() public {
        // The feed says 250. 1% above is 252.50; this limit allows 255.
        Settlement.Trade memory trade = _netBuy(nvda, 2e18, 510e18, _fills(alice, 2e18), _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.PriceOutOfBand.selector, 255e18, 247.5e18, 252.5e18));
        _settle(trade);
    }

    function test_aSaleBelowTheBandIsUndone() public {
        // The market pays 240 a token; 1% below 250 is 247.50.
        Settlement.Trade memory trade = _netSell(nvda, 2e18, 480e18, _none(), _fills(bob, 2e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.PriceOutOfBand.selector, 240e18, 247.5e18, 252.5e18));
        _settle(trade);
        _assertUntouched(bob);
    }

    function test_aMatchedBuyerIsProtectedFromACheapMarketLeg() public {
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

    function test_aTokenWithNoFeedPriceCannotBeTraded() public {
        MockToken other = new MockToken(18);
        Settlement.Trade memory trade = _netBuy(other, 2e18, 500e18, _fills(alice, 2e18), _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.PriceUnavailable.selector, address(other)));
        _settle(trade);
    }

    // ---- a route cannot take anything ----

    function test_aRouteThatDeliversNothingIsUndone() public {
        bytes memory call = abi.encodeCall(MockRouter.swap, (cash, nvda, 500e18, 0, address(settlement)));
        Settlement.Trade memory trade = Settlement.Trade(address(nvda), true, 2e18, 500e18, address(router), call, _fills(alice, 2e18), _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.MarketLegShort.selector, 2e18, 0));
        _settle(trade);
        _assertUntouched(alice);
    }

    function test_aRouteThatPaysSomeoneElseIsUndone() public {
        bytes memory call = abi.encodeCall(MockRouter.swap, (cash, nvda, 500e18, 2e18, makeAddr("thief")));
        Settlement.Trade memory trade = Settlement.Trade(address(nvda), true, 2e18, 500e18, address(router), call, _fills(alice, 2e18), _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.MarketLegShort.selector, 2e18, 0));
        _settle(trade);
        _assertUntouched(alice);
    }

    function test_aRouteCannotSpendAnAccountsApproval() public {
        // Bob approved the collector, not the settlement contract or any route.
        bytes memory call = abi.encodeCall(MockRouter.raid, (cash, bob, makeAddr("thief"), 1_000e18));
        Settlement.Trade memory trade = Settlement.Trade(address(nvda), true, 2e18, 500e18, address(router), call, _fills(alice, 2e18), _none());
        vm.expectRevert();
        _settle(trade);
        _assertUntouched(bob);
    }

    function test_aRouteCannotUseTheCollector() public {
        bytes memory call = abi.encodeCall(MockRouter.raidCollector, (collector, address(cash), bob, 1_000e18));
        Settlement.Trade memory trade = Settlement.Trade(address(nvda), true, 2e18, 500e18, address(router), call, _fills(alice, 2e18), _none());
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
        bytes memory call = abi.encodeCall(MockRouter.swap, (cash, nvda, 500e18, 2e18, address(settlement)));
        Settlement.Trade memory trade = Settlement.Trade(address(nvda), true, 2e18, 500e18, address(other), call, _fills(alice, 2e18), _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.RouterNotAllowed.selector, address(other)));
        _settle(trade);
    }

    function test_theCollectorAndTheCashTokenCanNeverBeRoutes() public {
        vm.startPrank(admin);
        vm.expectRevert(abi.encodeWithSelector(Settlement.RouterNotAllowed.selector, address(collector)));
        settlement.setRouter(address(collector), true);
        vm.expectRevert(abi.encodeWithSelector(Settlement.RouterNotAllowed.selector, address(cash)));
        settlement.setRouter(address(cash), true);
        vm.expectRevert(abi.encodeWithSelector(Settlement.RouterNotAllowed.selector, address(settlement)));
        settlement.setRouter(address(settlement), true);
        vm.stopPrank();
    }

    function test_theTokenBeingTradedCannotBeItsOwnRoute() public {
        vm.prank(admin);
        settlement.setRouter(address(nvda), true);
        bytes memory call = abi.encodeCall(MockToken.transfer, (makeAddr("thief"), 1e18));
        Settlement.Trade memory trade = Settlement.Trade(address(nvda), false, 1e18, 250e18, address(nvda), call, _none(), _fills(bob, 1e18));
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

    function test_aTokenTheAccountDidNotListCannotBeTraded() public {
        MockToken other = new MockToken(18);
        feed.set(address(other), 50e18);
        other.mint(alice, 10e18);
        vm.prank(alice);
        other.approve(address(collector), type(uint256).max);
        Settlement.Trade memory trade = _netSell(other, 1e18, 50e18, _none(), _fills(alice, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.TokenNotListed.selector, alice, address(other)));
        _settle(trade);
    }

    function test_theDailyLimitCapsTradedValue_andResetsTheNextDay() public {
        vm.prank(alice);
        settlement.setDailyLimit(600e18);
        _settle(_netBuy(nvda, 2e18, 500e18, _fills(alice, 2e18), _none()));

        Settlement.Trade memory second = _netBuy(nvda, 1e18, 250e18, _fills(alice, 1e18), _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.DailyLimitExceeded.selector, alice, 750e18, 600e18));
        _settle(second);

        vm.warp(block.timestamp + 1 days);
        feed.set(address(nvda), 250e18);
        _settle(_netBuy(nvda, 1e18, 250e18, _fills(alice, 1e18), _none()));
        assertEq(nvda.balanceOf(alice), START_STOCK + 3e18);
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
        trades[0] = _netSell(nvda, 1e18, 250e18, _none(), _fills(alice, 1e18));
        trades[1] = _netBuy(amzn, 1e18, 150e18, _fills(bob, 1e18), _none()); // far below the band
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

    // ---- what the account controls ----

    function test_closingStopsAllTradingAtOnce() public {
        vm.prank(alice);
        settlement.close();
        Settlement.Trade memory trade = _netSell(nvda, 1e18, 250e18, _none(), _fills(alice, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.AccountNotOpen.selector, alice));
        _settle(trade);
        _assertUntouched(alice);
    }

    function test_revokingAnApprovalStopsThatTokenAtOnce() public {
        vm.prank(alice);
        nvda.approve(address(collector), 0);
        Settlement.Trade memory trade = _netSell(nvda, 1e18, 250e18, _none(), _fills(alice, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Collector.TokenCallFailed.selector, address(nvda)));
        _settle(trade);
    }

    function test_unlistingATokenStopsIt() public {
        vm.prank(alice);
        settlement.setListed(address(nvda), false);
        Settlement.Trade memory trade = _netSell(nvda, 1e18, 250e18, _none(), _fills(alice, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.TokenNotListed.selector, alice, address(nvda)));
        _settle(trade);
    }

    function test_aTokenWithOtherDecimalsCannotBeListed() public {
        MockToken six = new MockToken(6);
        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(Settlement.WrongDecimals.selector, address(six)));
        settlement.setListed(address(six), true);
    }

    // ---- administration ----

    function test_onlyTheAdminReplacesTheOperator_andTheOldOneLosesAccess() public {
        address replacement = makeAddr("replacement");
        vm.prank(operator);
        vm.expectRevert(Settlement.NotAdmin.selector);
        settlement.setOperator(replacement);

        vm.prank(admin);
        settlement.setOperator(replacement);

        Settlement.Trade[] memory trades = new Settlement.Trade[](1);
        trades[0] = _netBuy(nvda, 1e18, 250e18, _fills(alice, 1e18), _none());
        vm.prank(operator);
        vm.expectRevert(Settlement.NotOperator.selector);
        settlement.settle(trades);
        vm.prank(replacement);
        settlement.settle(trades);
        assertEq(nvda.balanceOf(alice), START_STOCK + 1e18);
    }

    function test_theAdminCannotSettle_andOnlyTheAdminApprovesRoutes() public {
        Settlement.Trade[] memory trades = new Settlement.Trade[](1);
        trades[0] = _netBuy(nvda, 1e18, 250e18, _fills(alice, 1e18), _none());
        vm.prank(admin);
        vm.expectRevert(Settlement.NotOperator.selector);
        settlement.settle(trades);

        vm.prank(operator);
        vm.expectRevert(Settlement.NotAdmin.selector);
        settlement.setRouter(makeAddr("route"), true);
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

    /// Three accounts, each a buyer or a seller of a random quantity.
    function _mix(uint256 seed, uint256 feedPrice) private view returns (Mix memory mix) {
        address[3] memory who = [alice, bob, carol];
        Settlement.Fill[] memory buys = new Settlement.Fill[](3);
        Settlement.Fill[] memory sells = new Settlement.Fill[](3);
        uint256 nBuys;
        uint256 nSells;
        for (uint256 i = 0; i < 3; i++) {
            uint256 roll = uint256(keccak256(abi.encode(seed, i)));
            // Keep each fill's value inside what an account holds and may trade.
            uint256 quantity = bound(roll >> 8, 1e6, 10_000e36 / feedPrice);
            if (quantity > 90e18) quantity = 90e18;
            if (roll % 2 == 0) {
                mix.buyQty[i] = quantity;
                buys[nBuys++] = Settlement.Fill(who[i], quantity);
                mix.bought += quantity;
            } else {
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

    function _assertSettledAt(Mix memory mix, uint256 price) private view {
        address[3] memory who = [alice, bob, carol];
        for (uint256 i = 0; i < 3; i++) {
            if (mix.buyQty[i] != 0) {
                assertEq(nvda.balanceOf(who[i]), START_STOCK + mix.buyQty[i], "a buyer did not receive what it paid for");
                assertEq(cash.balanceOf(who[i]), START_CASH - _ceil(mix.buyQty[i], price), "a buyer paid the wrong amount");
            } else {
                assertEq(nvda.balanceOf(who[i]), START_STOCK - mix.sellQty[i], "a seller gave up the wrong amount");
                assertEq(cash.balanceOf(who[i]), START_CASH + (mix.sellQty[i] * price) / 1e18, "a seller was not paid in full");
            }
        }
        assertEq(nvda.balanceOf(address(settlement)), 0, "settlement keeps stock tokens");
        assertLt(cash.balanceOf(address(settlement)), 1e6, "settlement keeps more than dust");
    }

    /// Whatever the mix of buyers and sellers and wherever the market fills
    /// inside the band, every account trades at the one batch price and the
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
            _assertSettledAt(mix, (spend * 1e18 + net - 1) / net);
        } else {
            uint256 net = mix.sold - mix.bought;
            uint256 proceeds = (net * feedPrice * skewBps) / (1e18 * 10_000);
            vm.assume(proceeds > 0);
            _settle(_netSell(nvda, net, proceeds, mix.buys, mix.sells));
            _assertSettledAt(mix, (proceeds * 1e18) / net);
        }
    }
}
