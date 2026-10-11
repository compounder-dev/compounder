// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

interface IERC20 {
    function balanceOf(address account) external view returns (uint256);
}

/// @notice A public on-chain price per token: dollars per whole token scaled
///         by 1e18, and when it was last posted.
interface IPriceFeed {
    function prices(address token)
        external
        view
        returns (uint256 price, uint64 lastUpdated, uint32 maxTimeDelay, uint16 deviationBps);
}

/// @title Collector
/// @notice The only contract accounts approve. It moves an account's tokens
///         to the settlement contract when, and only when, the settlement
///         contract asks. Keeping approvals here means no call the settlement
///         contract makes to a trading route can ever spend them.
contract Collector {
    error NotSettlement();
    error TokenCallFailed(address token);

    address public immutable settlement;

    constructor() {
        settlement = msg.sender;
    }

    function collect(address token, address from, uint256 amount) external {
        if (msg.sender != settlement) revert NotSettlement();
        (bool ok, bytes memory result) = token.call(abi.encodeWithSelector(0x23b872dd, from, settlement, amount));
        if (!ok || token.code.length == 0 || (result.length != 0 && !abi.decode(result, (bool)))) {
            revert TokenCallFailed(token);
        }
    }
}

/// @title Settlement
/// @notice Trades stock tokens for many accounts in one transaction, at one
///         price per token.
///
///         An account opts in by approving the collector for the tokens it
///         wants traded and calling `open` with those tokens and a daily
///         limit. From then on the operator can include the account in a
///         batch, and every fill for it obeys rules the operator cannot bend:
///           1. Only tokens the account listed are touched.
///           2. Everyone in a batch gets the same price for a token: the
///              price the batch's own market trade achieved, measured from
///              balances, never reported by the caller.
///           3. That price must sit within a fixed band of a public price
///              feed that is fresh. Otherwise the batch is undone.
///           4. What an account gives up is paid for in full, in the same
///              transaction.
///           5. An account's traded value per day never passes its limit.
///
///         Buys and sells of one token are matched against each other first.
///         Only the difference is traded on the open market, through an
///         approved route. If any step fails, nothing moves for anyone.
///
///         The contract holds funds only inside `settle`.
///
/// @dev Stock tokens and the cash token are assumed to have 18 decimals, as
///      they do on the chain this is written for. Listing checks it.
contract Settlement {
    struct Fill {
        address account;
        uint256 quantity; // stock tokens
    }

    /// @dev One token's trades. `netQuantity` is what the market leg trades:
    ///      buys minus sells when `netBuy`, sells minus buys otherwise. It
    ///      must be more than zero, so every batch has a real market price.
    struct Trade {
        address token;
        bool netBuy;
        uint256 netQuantity;
        /// Net buy: the most cash the market leg may spend.
        /// Net sell: the least cash the market leg must return.
        uint256 cashLimit;
        address router;
        bytes routerCall;
        Fill[] buys;
        Fill[] sells;
    }

    struct Account {
        bool open;
        uint64 day; // the day `usedToday` counts
        uint256 dailyLimit; // cash value tradable per day
        uint256 usedToday;
    }

    error NotOperator();
    error NotAdmin();
    error ZeroAddress();
    error BadConfig();
    error WrongDecimals(address token);
    error AccountNotOpen(address account);
    error TokenNotListed(address account, address token);
    error DailyLimitExceeded(address account, uint256 wouldUse, uint256 limit);
    error RouterNotAllowed(address router);
    error NetQuantityMismatch(uint256 expected, uint256 stated);
    error EmptyFill();
    error PriceUnavailable(address token);
    error PriceOutOfBand(uint256 price, uint256 lowest, uint256 highest);
    error MarketLegShort(uint256 expected, uint256 got);
    error LeftShort(address token, uint256 before, uint256 afterwards);
    error TokenCallFailed(address token);
    error Reentered();

    event Opened(address indexed account, uint256 dailyLimit);
    event Closed(address indexed account);
    event TokenListed(address indexed account, address indexed token, bool listed);
    event DailyLimitSet(address indexed account, uint256 dailyLimit);
    event OperatorSet(address indexed operator);
    event RouterSet(address indexed router, bool allowed);
    event Filled(address indexed account, address indexed token, bool buy, uint256 quantity, uint256 cash);
    event Settled(address indexed token, uint256 price, bool netBuy, uint256 netQuantity, uint256 bought, uint256 sold);

    uint256 private constant ONE = 1e18;
    uint256 private constant BPS = 10_000;

    address public immutable cash;
    IPriceFeed public immutable priceFeed;
    Collector public immutable collector;
    /// @notice How far the batch price may sit from the feed price, in basis points.
    uint256 public immutable maxDeviationBps;
    /// @notice The oldest feed price a batch may rely on, in seconds.
    uint256 public immutable maxPriceAge;
    /// @notice Can replace the operator and approve routes. Has no power over funds.
    address public immutable admin;
    /// @notice The only caller of `settle`.
    address public operator;

    mapping(address => bool) public isRouter;
    mapping(address => Account) public accounts;
    mapping(address => mapping(address => bool)) public listed;

    uint256 private _entered = 1;

    modifier nonReentrant() {
        if (_entered != 1) revert Reentered();
        _entered = 2;
        _;
        _entered = 1;
    }

    constructor(address cash_, address priceFeed_, address admin_, address operator_, uint256 maxDeviationBps_, uint256 maxPriceAge_) {
        if (cash_ == address(0) || priceFeed_ == address(0) || admin_ == address(0) || operator_ == address(0)) {
            revert ZeroAddress();
        }
        if (maxDeviationBps_ == 0 || maxDeviationBps_ >= BPS || maxPriceAge_ == 0) revert BadConfig();
        _requireEighteenDecimals(cash_);
        cash = cash_;
        priceFeed = IPriceFeed(priceFeed_);
        admin = admin_;
        operator = operator_;
        maxDeviationBps = maxDeviationBps_;
        maxPriceAge = maxPriceAge_;
        collector = new Collector();
        emit OperatorSet(operator_);
    }

    // ---- the account's own controls ----

    /// @notice Opt in. The caller is the account. It must also approve the
    ///         collector for the cash token and each listed token.
    function open(address[] calldata tokens, uint256 dailyLimit) external {
        Account storage account = accounts[msg.sender];
        account.open = true;
        account.dailyLimit = dailyLimit;
        emit Opened(msg.sender, dailyLimit);
        for (uint256 i = 0; i < tokens.length; i++) {
            _list(tokens[i], true);
        }
    }

    /// @notice Opt out. Takes effect at once; revoking approvals does too.
    function close() external {
        accounts[msg.sender].open = false;
        emit Closed(msg.sender);
    }

    function setListed(address token, bool isListed) external {
        _list(token, isListed);
    }

    function setDailyLimit(uint256 dailyLimit) external {
        accounts[msg.sender].dailyLimit = dailyLimit;
        emit DailyLimitSet(msg.sender, dailyLimit);
    }

    // ---- admin ----

    function setOperator(address newOperator) external {
        if (msg.sender != admin) revert NotAdmin();
        if (newOperator == address(0)) revert ZeroAddress();
        operator = newOperator;
        emit OperatorSet(newOperator);
    }

    /// @notice Approve or remove a trading route. The collector, the cash
    ///         token and this contract can never be routes, so a route call
    ///         can never be turned into a pull from an account.
    function setRouter(address router, bool allowed) external {
        if (msg.sender != admin) revert NotAdmin();
        if (allowed && (router == address(0) || router == address(collector) || router == cash || router == address(this))) {
            revert RouterNotAllowed(router);
        }
        isRouter[router] = allowed;
        emit RouterSet(router, allowed);
    }

    // ---- settlement ----

    /// @notice Settle a batch. Each trade covers one token.
    function settle(Trade[] calldata trades) external nonReentrant {
        if (msg.sender != operator) revert NotOperator();
        uint256 cashBefore = IERC20(cash).balanceOf(address(this));
        for (uint256 i = 0; i < trades.length; i++) {
            _settleToken(trades[i]);
        }
        // Rounding always favours the contract by under one unit per fill, so
        // it can only end level or ahead. Anything else means a bug or a
        // misbehaving token or route, and the whole batch is undone.
        uint256 cashAfter = IERC20(cash).balanceOf(address(this));
        if (cashAfter < cashBefore) revert LeftShort(cash, cashBefore, cashAfter);
    }

    function _settleToken(Trade calldata trade) private {
        address token = trade.token;
        if (token == cash || trade.netQuantity == 0) revert NetQuantityMismatch(0, trade.netQuantity);
        if (!isRouter[trade.router] || trade.router == token) revert RouterNotAllowed(trade.router);
        (uint256 lowest, uint256 highest) = _band(token);
        uint256 tokenBefore = IERC20(token).balanceOf(address(this));

        uint256 sold;
        for (uint256 i = 0; i < trade.sells.length; i++) {
            Fill calldata fill = trade.sells[i];
            if (fill.quantity == 0) revert EmptyFill();
            collector.collect(token, fill.account, fill.quantity);
            sold += fill.quantity;
        }
        uint256 bought;
        for (uint256 i = 0; i < trade.buys.length; i++) {
            if (trade.buys[i].quantity == 0) revert EmptyFill();
            bought += trade.buys[i].quantity;
        }

        uint256 price = trade.netBuy
            ? _buyOnMarket(trade, bought, sold, lowest, highest)
            : _sellOnMarket(trade, bought, sold, lowest, highest);

        for (uint256 i = 0; i < trade.sells.length; i++) {
            Fill calldata fill = trade.sells[i];
            uint256 proceeds = _floorValue(fill.quantity, price);
            _spend(fill.account, token, proceeds);
            _push(cash, fill.account, proceeds);
            emit Filled(fill.account, token, false, fill.quantity, proceeds);
        }

        uint256 tokenAfter = IERC20(token).balanceOf(address(this));
        if (tokenAfter < tokenBefore) revert LeftShort(token, tokenBefore, tokenAfter);
        emit Settled(token, price, trade.netBuy, trade.netQuantity, bought, sold);
    }

    /// @dev Buyers outnumber sellers. Collect each buyer's worst-case cost,
    ///      buy the difference on the market, then charge everyone the price
    ///      that trade achieved and refund the rest.
    function _buyOnMarket(Trade calldata trade, uint256 bought, uint256 sold, uint256 lowest, uint256 highest)
        private
        returns (uint256 price)
    {
        uint256 net = trade.netQuantity;
        if (bought <= sold || bought - sold != net) revert NetQuantityMismatch(bought > sold ? bought - sold : 0, net);
        // The most the caller is willing to pay per token must already be inside the band.
        uint256 worstPrice = _ceilDiv(trade.cashLimit * ONE, net);
        if (worstPrice > highest) revert PriceOutOfBand(worstPrice, lowest, highest);

        for (uint256 i = 0; i < trade.buys.length; i++) {
            collector.collect(cash, trade.buys[i].account, _ceilValue(trade.buys[i].quantity, worstPrice));
        }

        (uint256 spent, uint256 received) = _callRouter(trade, cash, trade.cashLimit, trade.token);
        if (received < net) revert MarketLegShort(net, received);
        price = _ceilDiv(spent * ONE, net);
        if (price < lowest || price > highest) revert PriceOutOfBand(price, lowest, highest);

        _payBuyers(trade, bought, price, worstPrice, received - net);
    }

    /// @dev Charge each buyer the batch price, refund what was collected
    ///      beyond it, and deliver the tokens. `surplus` is tokens the market
    ///      delivered beyond the difference: a better fill than asked for.
    ///      Buyers share it by size and the last buyer takes the rounding
    ///      remainder, so no token is left behind.
    function _payBuyers(Trade calldata trade, uint256 bought, uint256 price, uint256 worstPrice, uint256 surplus) private {
        uint256 surplusLeft = surplus;
        uint256 last = trade.buys.length - 1;
        for (uint256 i = 0; i <= last; i++) {
            Fill calldata fill = trade.buys[i];
            uint256 cost = _ceilValue(fill.quantity, price);
            uint256 extra = i == last ? surplusLeft : (surplus * fill.quantity) / bought;
            surplusLeft -= extra;
            _spend(fill.account, trade.token, cost);
            _push(cash, fill.account, _ceilValue(fill.quantity, worstPrice) - cost);
            _push(trade.token, fill.account, fill.quantity + extra);
            emit Filled(fill.account, trade.token, true, fill.quantity, cost);
        }
    }

    /// @dev Sellers outnumber buyers. Sell the difference on the market, then
    ///      charge buyers and pay sellers the price that trade achieved.
    function _sellOnMarket(Trade calldata trade, uint256 bought, uint256 sold, uint256 lowest, uint256 highest)
        private
        returns (uint256 price)
    {
        uint256 net = trade.netQuantity;
        if (sold <= bought || sold - bought != net) revert NetQuantityMismatch(sold > bought ? sold - bought : 0, net);

        (uint256 spent, uint256 received) = _callRouter(trade, trade.token, net, cash);
        if (spent != net) revert MarketLegShort(net, spent);
        if (received < trade.cashLimit) revert MarketLegShort(trade.cashLimit, received);
        price = (received * ONE) / net;
        if (price < lowest || price > highest) revert PriceOutOfBand(price, lowest, highest);

        for (uint256 i = 0; i < trade.buys.length; i++) {
            Fill calldata fill = trade.buys[i];
            uint256 cost = _ceilValue(fill.quantity, price);
            _spend(fill.account, trade.token, cost);
            collector.collect(cash, fill.account, cost);
            _push(trade.token, fill.account, fill.quantity);
            emit Filled(fill.account, trade.token, true, fill.quantity, cost);
        }
    }

    /// @dev Let the route spend up to `allowance` of `tokenIn`, call it, and
    ///      measure from this contract's own balances what it took and what
    ///      it delivered.
    function _callRouter(Trade calldata trade, address tokenIn, uint256 allowance, address tokenOut)
        private
        returns (uint256 spent, uint256 received)
    {
        uint256 inBefore = IERC20(tokenIn).balanceOf(address(this));
        uint256 outBefore = IERC20(tokenOut).balanceOf(address(this));
        _approve(tokenIn, trade.router, allowance);
        (bool ok, bytes memory reason) = trade.router.call(trade.routerCall);
        if (!ok) {
            assembly {
                revert(add(reason, 32), mload(reason))
            }
        }
        _approve(tokenIn, trade.router, 0);
        uint256 inAfter = IERC20(tokenIn).balanceOf(address(this));
        uint256 outAfter = IERC20(tokenOut).balanceOf(address(this));
        spent = inBefore > inAfter ? inBefore - inAfter : 0;
        received = outAfter > outBefore ? outAfter - outBefore : 0;
        if (spent > allowance) revert MarketLegShort(allowance, spent);
    }

    /// @dev The lowest and highest batch price the feed allows right now.
    function _band(address token) private view returns (uint256 lowest, uint256 highest) {
        (uint256 feedPrice, uint64 lastUpdated,,) = priceFeed.prices(token);
        if (feedPrice == 0 || lastUpdated > block.timestamp || block.timestamp - lastUpdated > maxPriceAge) {
            revert PriceUnavailable(token);
        }
        lowest = (feedPrice * (BPS - maxDeviationBps)) / BPS;
        highest = (feedPrice * (BPS + maxDeviationBps)) / BPS;
    }

    /// @dev Check the account's rules for one fill and count it against today's limit.
    function _spend(address who, address token, uint256 value) private {
        Account storage account = accounts[who];
        if (!account.open) revert AccountNotOpen(who);
        if (!listed[who][token]) revert TokenNotListed(who, token);
        uint64 today = uint64(block.timestamp / 1 days);
        if (account.day != today) {
            account.day = today;
            account.usedToday = 0;
        }
        uint256 wouldUse = account.usedToday + value;
        if (wouldUse > account.dailyLimit) revert DailyLimitExceeded(who, wouldUse, account.dailyLimit);
        account.usedToday = wouldUse;
    }

    function _list(address token, bool isListed) private {
        if (isListed) _requireEighteenDecimals(token);
        listed[msg.sender][token] = isListed;
        emit TokenListed(msg.sender, token, isListed);
    }

    function _floorValue(uint256 quantity, uint256 price) private pure returns (uint256) {
        return (quantity * price) / ONE;
    }

    function _ceilValue(uint256 quantity, uint256 price) private pure returns (uint256) {
        return _ceilDiv(quantity * price, ONE);
    }

    function _ceilDiv(uint256 a, uint256 b) private pure returns (uint256) {
        return a == 0 ? 0 : (a - 1) / b + 1;
    }

    function _requireEighteenDecimals(address token) private view {
        (bool ok, bytes memory result) = token.staticcall(abi.encodeWithSelector(0x313ce567));
        if (!ok || result.length != 32 || abi.decode(result, (uint256)) != 18) revert WrongDecimals(token);
    }

    function _push(address token, address to, uint256 amount) private {
        if (amount == 0) return;
        _callToken(token, abi.encodeWithSelector(0xa9059cbb, to, amount));
    }

    function _approve(address token, address spender, uint256 amount) private {
        _callToken(token, abi.encodeWithSelector(0x095ea7b3, spender, amount));
    }

    /// @dev Accepts tokens that return nothing, rejects ones that return false.
    function _callToken(address token, bytes memory data) private {
        (bool ok, bytes memory result) = token.call(data);
        if (!ok || token.code.length == 0 || (result.length != 0 && !abi.decode(result, (bool)))) {
            revert TokenCallFailed(token);
        }
    }
}
