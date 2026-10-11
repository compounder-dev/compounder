// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import {Test} from "forge-std/Test.sol";
import {Settlement, IStockIssuer} from "../src/Settlement.sol";

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

    function burn(address from, uint256 amount) external {
        balanceOf[from] -= amount;
        totalSupply -= amount;
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

/// @dev Follows the real issuer's rules: the caller must be registered under
///      the quote's user id, a quote is used once before it expires, value is
///      quantity times price rounded down, and trades under 0.98 are refused.
contract MockIssuer is IStockIssuer {
    MockToken public immutable cash;
    mapping(address => bytes32) public registered;
    mapping(uint256 => bool) public used;

    constructor(MockToken cash_) {
        cash = cash_;
    }

    function register(address who, bytes32 id) external {
        registered[who] = id;
    }

    function sign(Quote memory quote) public pure returns (bytes memory) {
        return abi.encode(keccak256(abi.encode(quote, "issuer key")));
    }

    function _verify(Quote calldata quote, bytes calldata signature) private {
        bytes32 id = registered[msg.sender];
        require(id != bytes32(0), "caller not registered");
        require(!used[quote.attestationId], "quote already used");
        require(block.timestamp <= quote.expiration, "quote expired");
        require(id == quote.userId, "quote is for another user");
        require(quote.chainId == block.chainid, "wrong chain");
        require(keccak256(signature) == keccak256(sign(quote)), "bad signature");
        used[quote.attestationId] = true;
    }

    function mintWithAttestation(Quote calldata quote, bytes calldata signature, address depositToken, uint256 depositTokenAmount)
        external
        returns (uint256)
    {
        require(quote.side == 0, "not a buy quote");
        _verify(quote, signature);
        uint256 value = (quote.quantity * quote.price) / 1e18;
        require(value >= 0.98e18, "below minimum");
        require(depositToken == address(cash) && depositTokenAmount >= value, "underpaid");
        cash.transferFrom(msg.sender, address(this), depositTokenAmount);
        MockToken(quote.asset).mint(msg.sender, quote.quantity);
        return quote.quantity;
    }

    function redeemWithAttestation(Quote calldata quote, bytes calldata signature, address receiveToken, uint256 minimumReceiveAmount)
        external
        returns (uint256)
    {
        require(quote.side == 1, "not a sell quote");
        _verify(quote, signature);
        MockToken(quote.asset).transferFrom(msg.sender, address(this), quote.quantity);
        MockToken(quote.asset).burn(address(this), quote.quantity);
        uint256 value = (quote.quantity * quote.price) / 1e18;
        require(value >= 0.98e18, "below minimum");
        require(receiveToken == address(cash) && value >= minimumReceiveAmount, "pays too little");
        cash.mint(msg.sender, value);
        return value;
    }
}

contract SettlementTest is Test {
    bytes32 constant ID = bytes32("our settlement id");
    uint256 constant LIMIT = 100_000e18;

    MockToken cash;
    MockToken nvda;
    MockToken amzn;
    MockIssuer issuer;
    Settlement settlement;

    address admin = makeAddr("admin");
    address operator = makeAddr("operator");
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address carol = makeAddr("carol");
    uint256 nextQuoteId;

    function setUp() public {
        vm.warp(1_800_000_000);
        cash = new MockToken(18);
        nvda = new MockToken(18);
        amzn = new MockToken(18);
        issuer = new MockIssuer(cash);
        settlement = new Settlement(address(issuer), address(cash), admin, operator);
        issuer.register(address(settlement), ID);
        _join(alice);
        _join(bob);
        _join(carol);
    }

    // ---- helpers ----

    /// Fund an account, approve the settlement contract and opt in.
    function _join(address who) private {
        cash.mint(who, 50_000e18);
        nvda.mint(who, 100e18);
        amzn.mint(who, 100e18);
        address[] memory tokens = new address[](2);
        tokens[0] = address(nvda);
        tokens[1] = address(amzn);
        vm.startPrank(who);
        cash.approve(address(settlement), type(uint256).max);
        nvda.approve(address(settlement), type(uint256).max);
        amzn.approve(address(settlement), type(uint256).max);
        settlement.open(tokens, LIMIT);
        vm.stopPrank();
    }

    function _quote(MockToken token, uint8 side, uint256 price, uint256 quantity) private returns (IStockIssuer.Quote memory) {
        return IStockIssuer.Quote({
            chainId: block.chainid,
            attestationId: ++nextQuoteId,
            userId: ID,
            asset: address(token),
            price: price,
            quantity: quantity,
            expiration: block.timestamp + 15,
            side: side,
            additionalData: bytes32(0)
        });
    }

    function _fills(address a, uint256 qa) private pure returns (Settlement.Fill[] memory fills) {
        fills = new Settlement.Fill[](1);
        fills[0] = Settlement.Fill(a, qa);
    }

    function _none() private pure returns (Settlement.Fill[] memory) {
        return new Settlement.Fill[](0);
    }

    function _trade(IStockIssuer.Quote memory quote, Settlement.Fill[] memory buys, Settlement.Fill[] memory sells)
        private
        view
        returns (Settlement.Trade memory)
    {
        return Settlement.Trade(quote, issuer.sign(quote), buys, sells);
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
        assertEq(cash.allowance(address(settlement), address(issuer)), 0, "an approval is left standing");
        assertEq(nvda.allowance(address(settlement), address(issuer)), 0, "an approval is left standing");
    }

    // ---- what a batch does ----

    function test_buy_paysTheQuotedPriceAndDeliversTheTokens() public {
        _settle(_trade(_quote(nvda, 0, 250e18, 2e18), _fills(alice, 2e18), _none()));
        assertEq(cash.balanceOf(alice), 50_000e18 - 500e18);
        assertEq(nvda.balanceOf(alice), 102e18);
        _assertHoldsNothing();
    }

    function test_sell_paysTheQuotedPriceForTheTokens() public {
        _settle(_trade(_quote(nvda, 1, 250e18, 2e18), _none(), _fills(bob, 2e18)));
        assertEq(cash.balanceOf(bob), 50_000e18 + 500e18);
        assertEq(nvda.balanceOf(bob), 98e18);
        _assertHoldsNothing();
    }

    function test_buysAndSellsAreMatched_onlyTheDifferenceGoesToTheIssuer() public {
        uint256 supply = nvda.totalSupply();
        // Alice buys 3, Bob sells 1: the issuer mints only 2.
        _settle(_trade(_quote(nvda, 0, 250e18, 2e18), _fills(alice, 3e18), _fills(bob, 1e18)));
        assertEq(nvda.totalSupply(), supply + 2e18, "the issuer filled more than the difference");
        assertEq(nvda.balanceOf(alice), 103e18);
        assertEq(cash.balanceOf(alice), 50_000e18 - 750e18);
        assertEq(nvda.balanceOf(bob), 99e18);
        assertEq(cash.balanceOf(bob), 50_000e18 + 250e18);
        _assertHoldsNothing();
    }

    function test_matchedBatch_canNetToASell() public {
        uint256 supply = nvda.totalSupply();
        _settle(_trade(_quote(nvda, 1, 250e18, 4e18), _fills(alice, 1e18), _fills(bob, 5e18)));
        assertEq(nvda.totalSupply(), supply - 4e18);
        assertEq(nvda.balanceOf(alice), 101e18);
        assertEq(cash.balanceOf(bob), 50_000e18 + 1_250e18);
        _assertHoldsNothing();
    }

    function test_severalTokensSettleInOneTransaction() public {
        Settlement.Trade[] memory trades = new Settlement.Trade[](2);
        trades[0] = _trade(_quote(nvda, 1, 250e18, 1e18), _none(), _fills(alice, 1e18));
        trades[1] = _trade(_quote(amzn, 0, 200e18, 1e18), _fills(alice, 1e18), _none());
        vm.prank(operator);
        settlement.settle(trades);
        assertEq(nvda.balanceOf(alice), 99e18);
        assertEq(amzn.balanceOf(alice), 101e18);
        assertEq(cash.balanceOf(alice), 50_000e18 + 250e18 - 200e18);
        _assertHoldsNothing();
    }

    function test_rounding_neverLeavesTheContractShort() public {
        // A price and quantities whose products are not whole units.
        uint256 price = 241_658_100_000_000_000_333;
        uint256 aliceBuys = 47_140_000_000_000_007;
        uint256 carolBuys = 10_000_000_000_000_003;
        uint256 bobSells = 20_000_000_000_000_001;
        Settlement.Fill[] memory buys = new Settlement.Fill[](2);
        buys[0] = Settlement.Fill(alice, aliceBuys);
        buys[1] = Settlement.Fill(carol, carolBuys);
        _settle(_trade(_quote(nvda, 0, price, aliceBuys + carolBuys - bobSells), buys, _fills(bob, bobSells)));
        assertEq(cash.balanceOf(bob), 50_000e18 + (bobSells * price) / 1e18, "a seller is paid the floor of what it sold");
        assertEq(cash.balanceOf(alice), 50_000e18 - ((aliceBuys * price + 1e18 - 1) / 1e18), "a buyer pays the ceiling");
        _assertHoldsNothing();
    }

    // ---- what the operator cannot do ----

    function test_onlyTheOperatorCanSettle() public {
        Settlement.Trade[] memory trades = new Settlement.Trade[](1);
        trades[0] = _trade(_quote(nvda, 0, 250e18, 1e18), _fills(alice, 1e18), _none());
        vm.prank(alice);
        vm.expectRevert(Settlement.NotOperator.selector);
        settlement.settle(trades);
    }

    function test_anAccountThatNeverOptedInCannotBeTraded() public {
        address dave = makeAddr("dave");
        nvda.mint(dave, 10e18);
        vm.prank(dave);
        nvda.approve(address(settlement), type(uint256).max);
        Settlement.Trade memory trade = _trade(_quote(nvda, 1, 250e18, 1e18), _none(), _fills(dave, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.AccountNotOpen.selector, dave));
        _settle(trade);
    }

    function test_aTokenTheAccountDidNotListCannotBeTraded() public {
        MockToken other = new MockToken(18);
        other.mint(alice, 10e18);
        vm.prank(alice);
        other.approve(address(settlement), type(uint256).max);
        Settlement.Trade memory trade = _trade(_quote(other, 1, 50e18, 1e18), _none(), _fills(alice, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.TokenNotListed.selector, alice, address(other)));
        _settle(trade);
    }

    function test_theDailyLimitCapsTradedValue_andResetsTheNextDay() public {
        vm.prank(alice);
        settlement.setDailyLimit(600e18);
        _settle(_trade(_quote(nvda, 0, 250e18, 2e18), _fills(alice, 2e18), _none()));

        Settlement.Trade memory second = _trade(_quote(nvda, 0, 250e18, 1e18), _fills(alice, 1e18), _none());
        vm.expectRevert(abi.encodeWithSelector(Settlement.DailyLimitExceeded.selector, alice, 750e18, 600e18));
        _settle(second);

        vm.warp(block.timestamp + 1 days);
        _settle(_trade(_quote(nvda, 0, 250e18, 1e18), _fills(alice, 1e18), _none()));
        assertEq(nvda.balanceOf(alice), 103e18);
    }

    function test_thePriceCannotBeChangedFromTheIssuersQuote() public {
        IStockIssuer.Quote memory quote = _quote(nvda, 1, 250e18, 1e18);
        bytes memory signature = issuer.sign(quote);
        quote.price = 1e18; // pay the seller one dollar instead of 250
        Settlement.Trade[] memory trades = new Settlement.Trade[](1);
        trades[0] = Settlement.Trade(quote, signature, _none(), _fills(alice, 1e18));
        vm.prank(operator);
        vm.expectRevert(bytes("bad signature"));
        settlement.settle(trades);
        assertEq(nvda.balanceOf(alice), 100e18);
    }

    function test_theQuotedQuantityMustEqualTheDifference() public {
        Settlement.Trade memory trade = _trade(_quote(nvda, 0, 250e18, 3e18), _fills(alice, 3e18), _fills(bob, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.NetQuantityMismatch.selector, 2e18, 3e18));
        _settle(trade);
    }

    function test_aBatchWithNoIssuerLegIsRefused_soThePriceIsAlwaysVerified() public {
        // Buys equal sells: nothing would go to the issuer, so nothing would check the price.
        Settlement.Trade memory trade = _trade(_quote(nvda, 0, 250e18, 1e18), _fills(alice, 1e18), _fills(bob, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.NetQuantityMismatch.selector, 0, 1e18));
        _settle(trade);
    }

    function test_aQuoteCannotBeUsedTwice() public {
        Settlement.Trade memory trade = _trade(_quote(nvda, 0, 250e18, 1e18), _fills(alice, 1e18), _none());
        _settle(trade);
        vm.expectRevert(bytes("quote already used"));
        _settle(trade);
    }

    function test_oneFailingTradeUndoesTheWholeBatch() public {
        Settlement.Trade[] memory trades = new Settlement.Trade[](2);
        trades[0] = _trade(_quote(nvda, 1, 250e18, 1e18), _none(), _fills(alice, 1e18));
        trades[1] = _trade(_quote(amzn, 0, 200e18, 1e18), _fills(bob, 1e18), _none());
        vm.warp(block.timestamp + 16); // both quotes are now expired
        vm.prank(operator);
        vm.expectRevert(bytes("quote expired"));
        settlement.settle(trades);
        assertEq(nvda.balanceOf(alice), 100e18);
        assertEq(cash.balanceOf(alice), 50_000e18);
        assertEq(cash.balanceOf(bob), 50_000e18);
        _assertHoldsNothing();
    }

    function test_anEmptyFillIsRefused() public {
        Settlement.Fill[] memory buys = new Settlement.Fill[](2);
        buys[0] = Settlement.Fill(alice, 1e18);
        buys[1] = Settlement.Fill(carol, 0);
        Settlement.Trade memory trade = _trade(_quote(nvda, 0, 250e18, 1e18), buys, _none());
        vm.expectRevert(Settlement.EmptyFill.selector);
        _settle(trade);
    }

    // ---- what the account controls ----

    function test_closingStopsAllTradingAtOnce() public {
        vm.prank(alice);
        settlement.close();
        Settlement.Trade memory trade = _trade(_quote(nvda, 1, 250e18, 1e18), _none(), _fills(alice, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.AccountNotOpen.selector, alice));
        _settle(trade);
    }

    function test_revokingAnApprovalStopsThatTokenAtOnce() public {
        vm.prank(alice);
        nvda.approve(address(settlement), 0);
        Settlement.Trade memory trade = _trade(_quote(nvda, 1, 250e18, 1e18), _none(), _fills(alice, 1e18));
        vm.expectRevert(abi.encodeWithSelector(Settlement.TokenCallFailed.selector, address(nvda)));
        _settle(trade);
    }

    function test_unlistingATokenStopsIt() public {
        vm.prank(alice);
        settlement.setListed(address(nvda), false);
        Settlement.Trade memory trade = _trade(_quote(nvda, 1, 250e18, 1e18), _none(), _fills(alice, 1e18));
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
        trades[0] = _trade(_quote(nvda, 0, 250e18, 1e18), _fills(alice, 1e18), _none());
        vm.prank(operator);
        vm.expectRevert(Settlement.NotOperator.selector);
        settlement.settle(trades);
        vm.prank(replacement);
        settlement.settle(trades);
        assertEq(nvda.balanceOf(alice), 101e18);
    }

    function test_theAdminCannotSettle() public {
        Settlement.Trade[] memory trades = new Settlement.Trade[](1);
        trades[0] = _trade(_quote(nvda, 0, 250e18, 1e18), _fills(alice, 1e18), _none());
        vm.prank(admin);
        vm.expectRevert(Settlement.NotOperator.selector);
        settlement.settle(trades);
    }

    // ---- any batch ----

    /// Whatever the mix of buyers and sellers, every account is paid in full
    /// at the quoted price and the contract ends holding nothing but dust.
    function testFuzz_everyFillIsPaidInFullAtTheQuotedPrice(uint256 seed, uint256 price) public {
        price = bound(price, 1e15, 100_000e18);
        address[3] memory who = [alice, bob, carol];
        uint256[3] memory buyQty;
        uint256[3] memory sellQty;
        uint256 bought;
        uint256 sold;
        Settlement.Fill[] memory buys = new Settlement.Fill[](3);
        Settlement.Fill[] memory sells = new Settlement.Fill[](3);
        uint256 nBuys;
        uint256 nSells;
        for (uint256 i = 0; i < 3; i++) {
            uint256 roll = uint256(keccak256(abi.encode(seed, i)));
            // Keep each fill's value inside what an account holds and may trade.
            uint256 quantity = bound(roll >> 8, 1, 10_000e36 / price);
            if (quantity > 90e18) quantity = 90e18;
            if (roll % 2 == 0) {
                buyQty[i] = quantity;
                buys[nBuys++] = Settlement.Fill(who[i], quantity);
                bought += quantity;
            } else {
                sellQty[i] = quantity;
                sells[nSells++] = Settlement.Fill(who[i], quantity);
                sold += quantity;
            }
        }
        assembly {
            mstore(buys, nBuys)
            mstore(sells, nSells)
        }
        vm.assume(bought != sold);
        uint256 net = bought > sold ? bought - sold : sold - bought;
        vm.assume((net * price) / 1e18 >= 0.98e18);

        _settle(_trade(_quote(nvda, bought > sold ? 0 : 1, price, net), buys, sells));

        for (uint256 i = 0; i < 3; i++) {
            if (buyQty[i] != 0) {
                assertEq(nvda.balanceOf(who[i]), 100e18 + buyQty[i], "a buyer did not receive what it paid for");
                assertEq(cash.balanceOf(who[i]), 50_000e18 - (buyQty[i] * price + 1e18 - 1) / 1e18, "a buyer paid the wrong amount");
            } else {
                assertEq(nvda.balanceOf(who[i]), 100e18 - sellQty[i], "a seller gave up the wrong amount");
                assertEq(cash.balanceOf(who[i]), 50_000e18 + (sellQty[i] * price) / 1e18, "a seller was not paid in full");
            }
        }
        assertEq(nvda.balanceOf(address(settlement)), 0, "settlement keeps stock tokens");
        assertLt(cash.balanceOf(address(settlement)), 4, "settlement keeps more than one unit of dust per fill");
    }
}
