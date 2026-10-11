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

/// @notice The token issuer's own trading contract, as far as this contract
///         reads it. The issuer signs a short-lived quote for every trade it
///         is willing to make, and this says who may sign for it.
interface IIssuer {
    function getDomainSeparator() external view returns (bytes32);
    function hasRole(bytes32 role, address account) external view returns (bool);
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
/// @notice Rebalances many funds in one transaction, at one price per token.
///
///         A fund is any account that opts in. It approves the collector for
///         its tokens and calls `open` with the tokens it holds, the share of
///         the fund each one should be, and a daily limit. Shares are whole
///         numbers out of one billion, exactly as the strategy engine gives
///         them. From then on the
///         operator can include it in a batch, and every fill for it obeys
///         rules the operator cannot bend:
///           1. Only tokens the fund listed are touched.
///           2. A fill may only move a holding toward its target share: sell
///              what is above target, buy what is below, and never go past
///              the target. A fund that is on target cannot be traded at all,
///              so a fund can never be traded back and forth.
///           3. Everyone in a batch gets the same price for a token: the
///              price the batch's own market trade achieved, measured from
///              balances, never reported by the caller. A token appears at
///              most once in a batch.
///           4. That price must sit within a narrow band of a quote the
///              token's issuer signed for it seconds ago: the price at which
///              the issuer itself stands ready to trade right now. The quote
///              in turn must sit within a wider band of the issuer's slower
///              public price feed. Otherwise the batch is undone.
///           5. What a fund gives up is paid for in full, in the same
///              transaction.
///           6. A fund's traded value is capped by an allowance that holds
///              at most its daily limit and refills evenly over a day. So it
///              never trades more than the limit at once, nor more than
///              twice the limit in any one day.
///
///         Buys and sells of one token are matched against each other first.
///         Only the difference is traded on the open market, through an
///         approved route. If any step fails, nothing moves for anyone.
///
///         The contract holds funds only inside `settle`. The admin can stop
///         the operator or a route at once, but can appoint a new one only
///         after a two-day delay.
///
/// @dev Stock tokens and the cash token are assumed to have 18 decimals, as
///      they do on the chain this is written for. Listing checks it.
contract Settlement {
    struct Fill {
        address account;
        uint256 quantity; // stock tokens
    }

    /// @dev The issuer's signed quote, field for field as the issuer signs it.
    struct Quote {
        uint256 chainId;
        uint256 attestationId;
        bytes32 userId;
        address asset;
        uint256 price; // dollars per whole token, scaled by 1e18
        uint256 quantity;
        uint256 expiration;
        uint8 side; // 0: the issuer sells the token. 1: the issuer buys it.
        bytes32 additionalData;
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
        /// The issuer's live quote for the token, and its signature. A
        /// quote where the issuer sells when the market leg buys, and one
        /// where it buys when the market leg sells.
        Quote quote;
        bytes quoteSignature;
    }

    struct Account {
        bool open;
        uint32 epoch; // bumps on every `open`, so an old token list can never be read
        uint64 limitAt; // when `limitLeft` was last brought up to date
        uint256 dailyLimit; // cash value tradable per rolling day
        uint256 limitLeft;
    }

    /// @dev For one token right now: the feed price fills are valued at, and
    ///      the lowest and highest batch price the issuer's quote allows.
    struct Band {
        uint256 feed;
        uint256 lowest;
        uint256 highest;
    }

    error NotOperator();
    error NotAdmin();
    error ZeroAddress();
    error BadConfig();
    error BadTargets();
    error WrongDecimals(address token);
    error AccountNotOpen(address account);
    error TokenNotListed(address account, address token);
    error PastTarget(address account, address token, uint256 valueAfter, uint256 target);
    error DailyLimitExceeded(address account, uint256 value, uint256 left);
    error RouterNotAllowed(address router);
    error NetQuantityMismatch(uint256 expected, uint256 stated);
    error TokenTwice(address token);
    error EmptyFill();
    error PriceUnavailable(address token);
    error PriceOutOfBand(uint256 price, uint256 lowest, uint256 highest);
    error QuoteMismatch(address token);
    error QuoteNotLive(uint256 expiration);
    error QuoteNotSigned();
    error QuoteOutOfBand(uint256 quoted, uint256 feed);
    error MarketLegShort(uint256 expected, uint256 got);
    error LeftShort(address token, uint256 before, uint256 afterwards);
    error TokenCallFailed(address token);
    error NotReady();
    error Reentered();

    event Opened(address indexed account, uint256 dailyLimit, address[] tokens, uint256[] targetShares);
    event Closed(address indexed account);
    event DailyLimitSet(address indexed account, uint256 dailyLimit);
    event OperatorProposed(address indexed operator, uint256 readyAt);
    event OperatorSet(address indexed operator);
    event RouterProposed(address indexed router, uint256 readyAt);
    event RouterSet(address indexed router, bool allowed);
    event Filled(address indexed account, address indexed token, bool buy, uint256 quantity, uint256 cash);
    event Settled(address indexed token, uint256 price, bool netBuy, uint256 netQuantity, uint256 bought, uint256 sold);

    uint256 private constant ONE = 1e18;
    uint256 private constant BPS = 10_000;
    bytes32 private constant QUOTE_TYPEHASH = keccak256(
        "Quote(uint256 chainId,uint256 attestationId,bytes32 userId,address asset,uint256 price,uint256 quantity,uint256 expiration,uint8 side,bytes32 additionalData)"
    );
    bytes32 private constant QUOTE_SIGNER_ROLE = keccak256("ATTESTATION_SIGNER_ROLE");
    /// @dev Half the order of the signature curve. A signature above it has a
    ///      twin below it, and only the lower one counts.
    uint256 private constant HALF_ORDER =
        ((uint256(0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFE) << 128) | 0xBAAEDCE6AF48A03BBFD25E8CD0364141) / 2;
    /// @notice A target share of this much is the whole fund.
    uint256 public constant WHOLE = 1_000_000_000;
    uint256 public constant MAX_TOKENS = 32;
    /// @notice How long a new operator or route waits before it can act.
    uint256 public constant DELAY = 2 days;

    address public immutable cash;
    IPriceFeed public immutable priceFeed;
    IIssuer public immutable issuer;
    Collector public immutable collector;
    /// @notice How far the batch price may sit from the issuer's quote, in basis points.
    uint256 public immutable quoteBandBps;
    /// @notice The furthest ahead a quote may expire, in seconds. The issuer's
    ///         quotes live for seconds, so one dated further ahead is not one of them.
    uint256 public immutable maxQuoteLife;
    /// @notice How far the issuer's quote may sit from the feed price, in basis points.
    uint256 public immutable maxDeviationBps;
    /// @notice The oldest feed price a fund may be valued at, in seconds.
    uint256 public immutable maxPriceAge;
    /// @notice Can stop the operator or a route at once and appoint new ones after `DELAY`.
    address public immutable admin;
    /// @notice The only caller of `settle`.
    address public operator;
    address public pendingOperator;
    uint256 public pendingOperatorAt;

    mapping(address => bool) public isRouter;
    mapping(address => uint256) public routerReadyAt;
    mapping(address => Account) public accounts;
    mapping(address => address[]) private _tokens;
    /// @dev Target share out of `WHOLE`, plus one. Zero means not listed.
    mapping(address => mapping(uint32 => mapping(address => uint256))) private _targetPlusOne;

    /// @dev Counts batches, so a fund value cached for one batch is never read by another.
    uint256 private _batch;
    uint256 private _entered = 1;

    modifier nonReentrant() {
        if (_entered != 1) revert Reentered();
        _entered = 2;
        _;
        _entered = 1;
    }

    modifier onlyAdmin() {
        if (msg.sender != admin) revert NotAdmin();
        _;
    }

    constructor(
        address cash_,
        address priceFeed_,
        address issuer_,
        address admin_,
        address operator_,
        uint256 quoteBandBps_,
        uint256 maxQuoteLife_,
        uint256 maxDeviationBps_,
        uint256 maxPriceAge_
    ) {
        if (cash_ == address(0) || priceFeed_ == address(0) || issuer_ == address(0)) revert ZeroAddress();
        if (admin_ == address(0) || operator_ == address(0)) revert ZeroAddress();
        if (quoteBandBps_ == 0 || quoteBandBps_ > maxDeviationBps_ || maxDeviationBps_ >= BPS) revert BadConfig();
        if (maxQuoteLife_ == 0 || maxPriceAge_ == 0) revert BadConfig();
        _requireEighteenDecimals(cash_);
        cash = cash_;
        priceFeed = IPriceFeed(priceFeed_);
        issuer = IIssuer(issuer_);
        quoteBandBps = quoteBandBps_;
        maxQuoteLife = maxQuoteLife_;
        admin = admin_;
        operator = operator_;
        maxDeviationBps = maxDeviationBps_;
        maxPriceAge = maxPriceAge_;
        collector = new Collector();
        emit OperatorSet(operator_);
    }

    // ---- the fund's own controls ----

    /// @notice Opt in, or replace the fund's whole setup. The caller is the
    ///         fund. `targetShares[i]` is the share of the fund `tokens[i]`
    ///         should be, out of `WHOLE`; whatever the shares leave over is
    ///         the cash share.
    ///         The fund must also approve the collector for the cash token
    ///         and each listed token.
    function open(address[] calldata tokens, uint256[] calldata targetShares, uint256 dailyLimit) external {
        if (tokens.length != targetShares.length || tokens.length > MAX_TOKENS || dailyLimit > type(uint128).max) revert BadTargets();
        Account storage account = accounts[msg.sender];
        uint32 epoch = account.epoch + 1;
        uint256 total;
        for (uint256 i = 0; i < tokens.length; i++) {
            address token = tokens[i];
            if (token == cash || _targetPlusOne[msg.sender][epoch][token] != 0) revert BadTargets();
            _requireEighteenDecimals(token);
            if (targetShares[i] > WHOLE) revert BadTargets();
            // Only tokens the public feed prices can be listed. The price may be old here; it is checked for age when trading.
            (uint256 feed,,,) = priceFeed.prices(token);
            if (feed == 0) revert PriceUnavailable(token);
            _targetPlusOne[msg.sender][epoch][token] = targetShares[i] + 1;
            total += targetShares[i];
        }
        if (total > WHOLE) revert BadTargets();
        _tokens[msg.sender] = tokens;
        account.open = true;
        account.epoch = epoch;
        account.dailyLimit = dailyLimit;
        account.limitLeft = dailyLimit;
        account.limitAt = uint64(block.timestamp);
        emit Opened(msg.sender, dailyLimit, tokens, targetShares);
    }

    /// @notice Opt out. Takes effect at once; revoking approvals does too.
    function close() external {
        accounts[msg.sender].open = false;
        emit Closed(msg.sender);
    }

    function setDailyLimit(uint256 dailyLimit) external {
        if (dailyLimit > type(uint128).max) revert BadTargets();
        Account storage account = accounts[msg.sender];
        uint256 left = _limitLeft(account);
        account.dailyLimit = dailyLimit;
        account.limitLeft = left < dailyLimit ? left : dailyLimit;
        account.limitAt = uint64(block.timestamp);
        emit DailyLimitSet(msg.sender, dailyLimit);
    }

    function tokensOf(address account) external view returns (address[] memory) {
        return _tokens[account];
    }

    /// @notice Whether `account` lists `token`, and its target share out of `WHOLE`.
    function targetOf(address account, address token) external view returns (bool isListed, uint256 targetShare) {
        uint256 plusOne = _targetPlusOne[account][accounts[account].epoch][token];
        return plusOne == 0 ? (false, 0) : (true, plusOne - 1);
    }

    // ---- admin ----

    /// @notice Stop all trading at once.
    function removeOperator() external onlyAdmin {
        operator = address(0);
        pendingOperator = address(0);
        emit OperatorSet(address(0));
    }

    function proposeOperator(address next) external onlyAdmin {
        if (next == address(0)) revert ZeroAddress();
        pendingOperator = next;
        pendingOperatorAt = block.timestamp + DELAY;
        emit OperatorProposed(next, pendingOperatorAt);
    }

    /// @notice Anyone may complete a proposal once its delay has passed.
    function activateOperator() external {
        if (pendingOperator == address(0) || block.timestamp < pendingOperatorAt) revert NotReady();
        operator = pendingOperator;
        pendingOperator = address(0);
        emit OperatorSet(operator);
    }

    /// @notice Propose a trading route. The collector, the cash token and
    ///         this contract can never be routes, so a route call can never
    ///         be turned into a pull from a fund.
    function proposeRouter(address router) external onlyAdmin {
        if (router == address(0) || router == address(collector) || router == cash || router == address(this)) {
            revert RouterNotAllowed(router);
        }
        routerReadyAt[router] = block.timestamp + DELAY;
        emit RouterProposed(router, routerReadyAt[router]);
    }

    function activateRouter(address router) external {
        if (routerReadyAt[router] == 0 || block.timestamp < routerReadyAt[router]) revert NotReady();
        routerReadyAt[router] = 0;
        isRouter[router] = true;
        emit RouterSet(router, true);
    }

    function removeRouter(address router) external onlyAdmin {
        isRouter[router] = false;
        routerReadyAt[router] = 0;
        emit RouterSet(router, false);
    }

    // ---- settlement ----

    /// @notice Settle a batch. Each trade covers one token.
    function settle(Trade[] calldata trades) external nonReentrant {
        if (msg.sender != operator) revert NotOperator();
        _batch++;
        uint256 cashBefore = IERC20(cash).balanceOf(address(this));
        for (uint256 i = 0; i < trades.length; i++) {
            _settleToken(trades[i]);
        }
        // Rounding always favours the contract, by a unit or two per fill, so
        // it can only end level or ahead. Anything else means a bug or a
        // misbehaving token or route, and the whole batch is undone.
        uint256 cashAfter = IERC20(cash).balanceOf(address(this));
        if (cashAfter < cashBefore) revert LeftShort(cash, cashBefore, cashAfter);
    }

    function _settleToken(Trade calldata trade) private {
        address token = trade.token;
        if (token == cash || trade.netQuantity == 0) revert NetQuantityMismatch(0, trade.netQuantity);
        if (!isRouter[trade.router] || trade.router == token) revert RouterNotAllowed(trade.router);
        _onceThisBatch(token);
        Band memory band = _band(trade);
        uint256 tokenBefore = IERC20(token).balanceOf(address(this));

        uint256 sold;
        for (uint256 i = 0; i < trade.sells.length; i++) {
            Fill calldata fill = trade.sells[i];
            if (fill.quantity == 0) revert EmptyFill();
            _towardTarget(fill.account, token, band.feed, fill.quantity, false);
            collector.collect(token, fill.account, fill.quantity);
            sold += fill.quantity;
        }
        uint256 bought;
        for (uint256 i = 0; i < trade.buys.length; i++) {
            if (trade.buys[i].quantity == 0) revert EmptyFill();
            bought += trade.buys[i].quantity;
        }

        uint256 price = trade.netBuy ? _buyOnMarket(trade, bought, sold, band) : _sellOnMarket(trade, bought, sold, band);

        for (uint256 i = 0; i < trade.sells.length; i++) {
            Fill calldata fill = trade.sells[i];
            uint256 proceeds = _floorValue(fill.quantity, price);
            _spend(fill.account, proceeds);
            _push(cash, fill.account, proceeds);
            emit Filled(fill.account, token, false, fill.quantity, proceeds);
        }

        uint256 tokenAfter = IERC20(token).balanceOf(address(this));
        if (tokenAfter < tokenBefore) revert LeftShort(token, tokenBefore, tokenAfter);
        emit Settled(token, price, trade.netBuy, trade.netQuantity, bought, sold);
    }

    /// @dev Buyers outnumber sellers. Collect each buyer's worst-case cost,
    ///      buy the difference on the market, then settle everyone at the
    ///      price that trade achieved and refund the rest.
    function _buyOnMarket(Trade calldata trade, uint256 bought, uint256 sold, Band memory band) private returns (uint256 price) {
        uint256 net = trade.netQuantity;
        if (bought <= sold || bought - sold != net) revert NetQuantityMismatch(bought > sold ? bought - sold : 0, net);
        // The most a buyer can be charged per token. It may sit above the band
        // to leave room for slippage; the price achieved is what the band judges.
        uint256 worstPrice = _ceilDiv(trade.cashLimit * ONE, net);

        for (uint256 i = 0; i < trade.buys.length; i++) {
            Fill calldata fill = trade.buys[i];
            // Checked before the buyer's cash moves, so the fund is valued whole.
            _towardTarget(fill.account, trade.token, band.feed, fill.quantity, true);
            collector.collect(cash, fill.account, _ceilValue(fill.quantity, worstPrice));
        }

        (uint256 spent, uint256 received) = _callRouter(trade, cash, trade.cashLimit, trade.token);
        if (received < net) revert MarketLegShort(net, received);
        // The price the market really gave: what was spent, over everything
        // it delivered. Never more than `worstPrice`.
        price = _ceilDiv(spent * ONE, received);
        if (price < band.lowest || price > band.highest) revert PriceOutOfBand(price, band.lowest, band.highest);

        // Buyers owe the market's cash plus what the sellers are paid.
        _payBuyers(trade, bought, spent + _floorValue(sold, price), worstPrice, received - net, band.feed);
    }

    /// @dev Share `owed` among the buyers by size, refund what was collected
    ///      beyond it, and deliver the tokens. `surplus` is tokens the market
    ///      delivered beyond the difference. Buyers share those by size too,
    ///      so each pays the market's price for everything it receives. The
    ///      last buyer takes the rounding remainder, so no token is left
    ///      behind. The target is checked again here against what each buyer
    ///      really receives.
    function _payBuyers(Trade calldata trade, uint256 bought, uint256 owed, uint256 worstPrice, uint256 surplus, uint256 feed) private {
        uint256 surplusLeft = surplus;
        uint256 last = trade.buys.length - 1;
        for (uint256 i = 0; i <= last; i++) {
            Fill calldata fill = trade.buys[i];
            // Never more than was collected: `owed / bought` is at most `worstPrice`.
            uint256 cost = _ceilDiv(owed * fill.quantity, bought);
            uint256 extra = i == last ? surplusLeft : (surplus * fill.quantity) / bought;
            surplusLeft -= extra;
            _towardTarget(fill.account, trade.token, feed, fill.quantity + extra, true);
            _spend(fill.account, cost);
            _push(cash, fill.account, _ceilValue(fill.quantity, worstPrice) - cost);
            _push(trade.token, fill.account, fill.quantity + extra);
            emit Filled(fill.account, trade.token, true, fill.quantity + extra, cost);
        }
    }

    /// @dev Sellers outnumber buyers. Sell the difference on the market, then
    ///      charge buyers and pay sellers the price that trade achieved.
    function _sellOnMarket(Trade calldata trade, uint256 bought, uint256 sold, Band memory band) private returns (uint256 price) {
        uint256 net = trade.netQuantity;
        if (sold <= bought || sold - bought != net) revert NetQuantityMismatch(sold > bought ? sold - bought : 0, net);

        (uint256 spent, uint256 received) = _callRouter(trade, trade.token, net, cash);
        if (spent != net) revert MarketLegShort(net, spent);
        if (received < trade.cashLimit) revert MarketLegShort(trade.cashLimit, received);
        price = (received * ONE) / net;
        if (price < band.lowest || price > band.highest) revert PriceOutOfBand(price, band.lowest, band.highest);

        for (uint256 i = 0; i < trade.buys.length; i++) {
            Fill calldata fill = trade.buys[i];
            uint256 cost = _ceilValue(fill.quantity, price);
            _towardTarget(fill.account, trade.token, band.feed, fill.quantity, true);
            _spend(fill.account, cost);
            collector.collect(cash, fill.account, cost);
            _push(trade.token, fill.account, fill.quantity);
            emit Filled(fill.account, trade.token, true, fill.quantity, cost);
        }
    }

    /// @dev A second trade in the same token could settle at a second price.
    function _onceThisBatch(address token) private {
        bytes32 slot = keccak256(abi.encode(token, _batch, "token"));
        uint256 seen;
        assembly {
            seen := tload(slot)
            tstore(slot, 1)
        }
        if (seen != 0) revert TokenTwice(token);
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

    // ---- the rules a fill must pass ----

    /// @dev Refuse a fill that would take the holding past its target, or
    ///      move it away from it. Values use the public feed, never the batch
    ///      price. Called before the fill moves any of the fund's tokens, so
    ///      `held` is the balance going in.
    function _towardTarget(address who, address token, uint256 feed, uint256 quantity, bool buy) private {
        Account storage account = accounts[who];
        if (!account.open) revert AccountNotOpen(who);
        uint256 plusOne = _targetPlusOne[who][account.epoch][token];
        if (plusOne == 0) revert TokenNotListed(who, token);

        uint256 target = (_fundValue(who) * (plusOne - 1)) / WHOLE;
        uint256 held = IERC20(token).balanceOf(who);
        if (buy) {
            uint256 valueAfter = _floorValue(held + quantity, feed);
            if (valueAfter > target) revert PastTarget(who, token, valueAfter, target);
        } else {
            uint256 valueAfter = quantity > held ? 0 : _floorValue(held - quantity, feed);
            if (valueAfter < target) revert PastTarget(who, token, valueAfter, target);
        }
    }

    /// @dev What a fund is worth at feed prices: its cash plus every listed
    ///      token. Worked out once per fund per batch, before any of its
    ///      fills, and remembered for the rest of that batch only, so a
    ///      fund's targets do not shift while its own trades are settling.
    function _fundValue(address who) private returns (uint256 value) {
        bytes32 slot = keccak256(abi.encode(who, _batch));
        assembly {
            value := tload(slot)
        }
        if (value != 0) return value - 1;

        value = IERC20(cash).balanceOf(who);
        address[] storage tokens = _tokens[who];
        for (uint256 i = 0; i < tokens.length; i++) {
            uint256 held = IERC20(tokens[i]).balanceOf(who);
            if (held != 0) value += _floorValue(held, _feedPrice(tokens[i]));
        }
        assembly {
            tstore(slot, add(value, 1))
        }
    }

    /// @dev Count a fill against the fund's limit, which refills evenly over a day.
    function _spend(address who, uint256 value) private {
        if (value == 0) revert EmptyFill();
        Account storage account = accounts[who];
        uint256 left = _limitLeft(account);
        if (value > left) revert DailyLimitExceeded(who, value, left);
        account.limitLeft = left - value;
        account.limitAt = uint64(block.timestamp);
    }

    function _limitLeft(Account storage account) private view returns (uint256 left) {
        left = account.limitLeft + (account.dailyLimit * (block.timestamp - account.limitAt)) / 1 days;
        if (left > account.dailyLimit) left = account.dailyLimit;
    }

    function _feedPrice(address token) private view returns (uint256 feed) {
        uint64 lastUpdated;
        (feed, lastUpdated,,) = priceFeed.prices(token);
        if (feed == 0 || lastUpdated > block.timestamp || block.timestamp - lastUpdated > maxPriceAge) {
            revert PriceUnavailable(token);
        }
    }

    /// @dev The batch price is held to the issuer's quote, and the quote to
    ///      the slower feed. The feed alone is too stale to price a trade,
    ///      but it bounds what a stolen quote-signing key could do.
    function _band(Trade calldata trade) private view returns (Band memory band) {
        band.feed = _feedPrice(trade.token);
        uint256 quoted = _quotedPrice(trade);
        if (quoted * BPS < band.feed * (BPS - maxDeviationBps) || quoted * BPS > band.feed * (BPS + maxDeviationBps)) {
            revert QuoteOutOfBand(quoted, band.feed);
        }
        band.lowest = (quoted * (BPS - quoteBandBps)) / BPS;
        band.highest = (quoted * (BPS + quoteBandBps)) / BPS;
    }

    /// @dev The price in the trade's quote, once the quote is shown to be the
    ///      issuer's own, for this token, this direction and this moment.
    function _quotedPrice(Trade calldata trade) private view returns (uint256) {
        Quote calldata quote = trade.quote;
        // A batch that buys is held to what the issuer sells at, and one that sells to what it buys at.
        if (quote.asset != trade.token || quote.chainId != block.chainid || quote.side != (trade.netBuy ? 0 : 1)) {
            revert QuoteMismatch(trade.token);
        }
        if (block.timestamp > quote.expiration || quote.expiration - block.timestamp > maxQuoteLife) {
            revert QuoteNotLive(quote.expiration);
        }
        bytes calldata signature = trade.quoteSignature;
        if (signature.length != 65 || uint256(bytes32(signature[32:64])) > HALF_ORDER) revert QuoteNotSigned();
        bytes32 digest =
            keccak256(abi.encodePacked("\x19\x01", issuer.getDomainSeparator(), keccak256(abi.encode(QUOTE_TYPEHASH, quote))));
        address signer = ecrecover(digest, uint8(signature[64]), bytes32(signature[0:32]), bytes32(signature[32:64]));
        // Whoever the issuer lets sign today. A change of key on its side needs nothing here.
        if (signer == address(0) || !issuer.hasRole(QUOTE_SIGNER_ROLE, signer)) revert QuoteNotSigned();
        return quote.price;
    }

    // ---- arithmetic and token calls ----

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
