// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

interface IERC20 {
    function balanceOf(address account) external view returns (uint256);
}

/// @notice The stock-token issuer's mint and redeem entry points. A quote is
///         signed by the issuer for one token, side, price and quantity, and
///         can be used once before it expires.
interface IStockIssuer {
    struct Quote {
        uint256 chainId;
        uint256 attestationId;
        bytes32 userId;
        address asset;
        uint256 price; // dollars per whole token, scaled by 1e18
        uint256 quantity; // tokens, 18 decimals
        uint256 expiration;
        uint8 side; // 0 buy, 1 sell
        bytes32 additionalData;
    }

    function mintWithAttestation(Quote calldata quote, bytes calldata signature, address depositToken, uint256 depositTokenAmount)
        external
        returns (uint256);

    function redeemWithAttestation(Quote calldata quote, bytes calldata signature, address receiveToken, uint256 minimumReceiveAmount)
        external
        returns (uint256);
}

/// @title Settlement
/// @notice Trades stock tokens for many accounts in one transaction, at one
///         issuer-signed price per token.
///
///         An account opts in by approving this contract for the tokens it
///         wants traded and calling `open` with those tokens and a daily
///         limit. From then on the operator can include the account in a
///         batch, and every fill for it obeys four rules the operator cannot
///         bend:
///           1. Only tokens the account listed are touched.
///           2. The price is the issuer's signed quote for that token, the
///              same for everyone in the batch.
///           3. What an account gives up is paid for in full, in the same
///              transaction: a seller is paid for exactly what it sold and a
///              buyer receives exactly what it paid for.
///           4. An account's traded value per day never passes its limit.
///
///         Buys and sells of one token are matched against each other first.
///         Only the difference goes to the issuer, in a single mint or
///         redeem. If any step fails, nothing moves for anyone.
///
///         The contract holds funds only inside `settle`. No function sends
///         tokens to anyone except the accounts in the batch and the issuer.
///
/// @dev Stock tokens and the cash token are assumed to have 18 decimals, as
///      they do on the chain this is written for. The constructor checks it.
contract Settlement {
    struct Fill {
        address account;
        uint256 quantity; // stock tokens
    }

    /// @dev One token's trades. `quote.quantity` is the net amount the issuer
    ///      fills: buys minus sells for a buy quote, sells minus buys for a
    ///      sell quote. It must be more than zero, so every batch is priced
    ///      by a quote the issuer verifies in the same transaction.
    struct Trade {
        IStockIssuer.Quote quote;
        bytes signature;
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
    error WrongDecimals(address token);
    error AccountNotOpen(address account);
    error TokenNotListed(address account, address token);
    error DailyLimitExceeded(address account, uint256 wouldUse, uint256 limit);
    error BadQuoteSide(uint8 side);
    error NetQuantityMismatch(uint256 expected, uint256 quoted);
    error EmptyFill();
    error LeftShort(address token, uint256 before, uint256 afterwards);
    error TokenCallFailed(address token);
    error Reentered();

    event Opened(address indexed account, uint256 dailyLimit);
    event Closed(address indexed account);
    event TokenListed(address indexed account, address indexed token, bool listed);
    event DailyLimitSet(address indexed account, uint256 dailyLimit);
    event OperatorSet(address indexed operator);
    event Filled(address indexed account, address indexed token, bool buy, uint256 quantity, uint256 cash);
    event Settled(address indexed token, uint256 price, bool netBuy, uint256 netQuantity, uint256 bought, uint256 sold);

    uint256 private constant ONE = 1e18;
    uint8 private constant BUY = 0;
    uint8 private constant SELL = 1;

    IStockIssuer public immutable issuer;
    address public immutable cash;
    /// @notice Can replace the operator. Has no power over funds.
    address public immutable admin;
    /// @notice The only caller of `settle`.
    address public operator;

    mapping(address => Account) public accounts;
    mapping(address => mapping(address => bool)) public listed;

    uint256 private _entered = 1;

    modifier nonReentrant() {
        if (_entered != 1) revert Reentered();
        _entered = 2;
        _;
        _entered = 1;
    }

    constructor(address issuer_, address cash_, address admin_, address operator_) {
        if (issuer_ == address(0) || cash_ == address(0) || admin_ == address(0) || operator_ == address(0)) {
            revert ZeroAddress();
        }
        _requireEighteenDecimals(cash_);
        issuer = IStockIssuer(issuer_);
        cash = cash_;
        admin = admin_;
        operator = operator_;
        emit OperatorSet(operator_);
    }

    // ---- the account's own controls ----

    /// @notice Opt in. The caller is the account. It must also approve this
    ///         contract for the cash token and each listed token.
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
        // misbehaving token, and the whole batch is undone.
        uint256 cashAfter = IERC20(cash).balanceOf(address(this));
        if (cashAfter < cashBefore) revert LeftShort(cash, cashBefore, cashAfter);
    }

    function _settleToken(Trade calldata trade) private {
        address token = trade.quote.asset;
        uint256 price = trade.quote.price;
        uint256 tokenBefore = IERC20(token).balanceOf(address(this));

        // Collect. Sellers hand over tokens, buyers hand over cash.
        uint256 sold;
        for (uint256 i = 0; i < trade.sells.length; i++) {
            Fill calldata fill = trade.sells[i];
            if (fill.quantity == 0) revert EmptyFill();
            _spend(fill.account, token, _floorValue(fill.quantity, price));
            _pull(token, fill.account, fill.quantity);
            sold += fill.quantity;
        }
        uint256 bought;
        for (uint256 i = 0; i < trade.buys.length; i++) {
            Fill calldata fill = trade.buys[i];
            if (fill.quantity == 0) revert EmptyFill();
            uint256 cost = _ceilValue(fill.quantity, price);
            _spend(fill.account, token, cost);
            _pull(cash, fill.account, cost);
            bought += fill.quantity;
        }

        // Trade the difference with the issuer.
        uint256 net = trade.quote.quantity;
        bool netBuy = trade.quote.side == BUY;
        if (netBuy) {
            if (bought <= sold || bought - sold != net) revert NetQuantityMismatch(bought > sold ? bought - sold : 0, net);
            uint256 payment = _floorValue(net, price);
            _approve(cash, address(issuer), payment);
            issuer.mintWithAttestation(trade.quote, trade.signature, cash, payment);
            _approve(cash, address(issuer), 0);
        } else if (trade.quote.side == SELL) {
            if (sold <= bought || sold - bought != net) revert NetQuantityMismatch(sold > bought ? sold - bought : 0, net);
            _approve(token, address(issuer), net);
            issuer.redeemWithAttestation(trade.quote, trade.signature, cash, _floorValue(net, price));
            _approve(token, address(issuer), 0);
        } else {
            revert BadQuoteSide(trade.quote.side);
        }

        // Pay out. Buyers get their tokens, sellers get their cash.
        for (uint256 i = 0; i < trade.buys.length; i++) {
            Fill calldata fill = trade.buys[i];
            _push(token, fill.account, fill.quantity);
            emit Filled(fill.account, token, true, fill.quantity, _ceilValue(fill.quantity, price));
        }
        for (uint256 i = 0; i < trade.sells.length; i++) {
            Fill calldata fill = trade.sells[i];
            uint256 proceeds = _floorValue(fill.quantity, price);
            _push(cash, fill.account, proceeds);
            emit Filled(fill.account, token, false, fill.quantity, proceeds);
        }

        uint256 tokenAfter = IERC20(token).balanceOf(address(this));
        if (tokenAfter < tokenBefore) revert LeftShort(token, tokenBefore, tokenAfter);
        emit Settled(token, price, netBuy, net, bought, sold);
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
        return (quantity * price + ONE - 1) / ONE;
    }

    function _requireEighteenDecimals(address token) private view {
        (bool ok, bytes memory result) = token.staticcall(abi.encodeWithSelector(0x313ce567));
        if (!ok || result.length != 32 || abi.decode(result, (uint256)) != 18) revert WrongDecimals(token);
    }

    function _pull(address token, address from, uint256 amount) private {
        _callToken(token, abi.encodeWithSelector(0x23b872dd, from, address(this), amount));
    }

    function _push(address token, address to, uint256 amount) private {
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
