// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/* ========== OpenZeppelin Interface & Helpers (Simplified) ========== */

interface IERC20 {
    function totalSupply() external view returns (uint256);

    function balanceOf(address account) external view returns (uint256);

    function transfer(address recipient, uint256 amount)
        external
        returns (bool);

    function allowance(address owner, address spender)
        external
        view
        returns (uint256);

    function approve(address spender, uint256 amount) external returns (bool);

    function transferFrom(
        address sender,
        address recipient,
        uint256 amount
    ) external returns (bool);
}

/**
 * @dev Simplified Ownable
 */
contract Ownable {
    address public owner;

    event OwnershipTransferred(
        address indexed previousOwner,
        address indexed newOwner
    );

    constructor() {
        owner = msg.sender;
    }

    modifier onlyOwner() {
        require(msg.sender == owner, "Not owner");
        _;
    }

    function transferOwnership(address newOwner) external onlyOwner {
        require(newOwner != address(0), "zero address");
        emit OwnershipTransferred(owner, newOwner);
        owner = newOwner;
    }
}

/**
 * @dev ReentrancyGuard (simplified)
 */
abstract contract ReentrancyGuard {
    bool private _notEntered;

    constructor() {
        _notEntered = true;
    }

    modifier nonReentrant() {
        require(_notEntered, "reentrancy");
        _notEntered = false;
        _;
        _notEntered = true;
    }
}

/* ========== XenblocksHashMarket Contract ========== */

contract XenblocksHashMarket is Ownable, ReentrancyGuard {
    // -----------------------------
    //  CONSTANTS / GLOBAL SETTINGS
    // -----------------------------
    IERC20 public immutable usdtToken; // BSC上官方USDT合约地址
    uint256 public minTradeAmount;     // 最小成交金额(USDT), 例如 50 USDT
    bool public paused;                // 是否暂停新订单的创建

    // 押金比例
    // （示例：Seller 21%，Buyer 5%）
    uint256 public sellerDepositRate;  // 卖家押金比例(默认21)
    uint256 public buyerDepositRate;   // 买家额外押金比例(默认5)

    // 手续费接收者
    address public feeReceiver;

    // -----------------------------
    //   数据结构 (订单 / 交易)
    // -----------------------------

    // 卖家累积成交额 => 用于判断费率折扣
    mapping(address => uint256) public sellerVolume;

    // 买单结构
    struct BuyOrder {
        address buyer;
        uint256 xnmAmount;       // Buyer想买的XNM数量
        uint256 price;           // 单价(USDT)
        uint256 buyerDeposit;    // 该订单锁定的 Buyer押金 ( = 买单金额 * buyerDepositRate% )
        uint256 usdtTotal;       // 买单总金额( xnmAmount * price )
        uint256 maxDeliveryDays; // 最长交割天数
        bool active;             // 是否有效(未被吃/未取消)
    }

    // 卖单结构
    struct SellOrder {
        address seller;
        uint256 price;            // 单价(USDT)
        uint256 minXNM;           // 最小可成交数量
        uint256 maxXNM;           // 最大可成交数量
        uint256 sellerDeposit;    // 卖家挂单时已提交押金(针对maxXNM)
        uint256 maxDeliveryDays;  // 最长交割天数
        bool active;              // 是否有效(未被吃/未取消)
    }

    // 交易结构(撮合后生成)
    enum TradeStatus { Active, Completed, Released }

    struct TradeInfo {
        address buyer;
        address seller;
        uint256 xnmAmount;       // 实际成交的XNM数量
        uint256 price;           // 成交单价
        uint256 usdtAmount;      // 成交金额( xnmAmount * price )
        uint256 buyerDeposit;    // Buyer在此交易中实际锁定的押金
        uint256 sellerDeposit;   // Seller在此交易中实际锁定的押金
        uint256 startTime;       // 交易开始时间
        uint256 maxDeliveryDays; // 最长交割天数
        TradeStatus status;      // 交易状态
    }

    // buyOrderId => BuyOrder
    mapping(uint256 => BuyOrder) public buyOrders;
    uint256 public nextBuyOrderId;

    // sellOrderId => SellOrder
    mapping(uint256 => SellOrder) public sellOrders;
    uint256 public nextSellOrderId;

    // tradeId => TradeInfo
    mapping(uint256 => TradeInfo) public trades;
    uint256 public nextTradeId;

    // -----------------------------
    //           Events
    // -----------------------------
    event BuyOrderCreated(
        uint256 indexed orderId,
        address indexed buyer,
        uint256 xnmAmount,
        uint256 price,
        uint256 usdtTotal,
        uint256 maxDeliveryDays
    );

    event BuyOrderCancelled(uint256 indexed orderId);

    event SellOrderCreated(
        uint256 indexed orderId,
        address indexed seller,
        uint256 price,
        uint256 minXNM,
        uint256 maxXNM,
        uint256 maxDeliveryDays
    );

    event SellOrderCancelled(uint256 indexed orderId);

    event TradeMatched(
        uint256 indexed tradeId,
        address indexed buyer,
        address indexed seller,
        uint256 xnmAmount,
        uint256 price,
        uint256 usdtAmount
    );

    event TradeCompleted(uint256 indexed tradeId);

    event TradeReleased(
        uint256 indexed tradeId,
        uint256 usdtToSeller,
        uint256 usdtRefundToBuyer,
        uint256 sellerDepositPenalty,   // 被扣Seller押金
        uint256 buyerDepositPenalty     // 被扣Buyer押金
    );

    event Paused(bool paused);

    event ParamsUpdated(
        uint256 sellerDepositRate,
        uint256 buyerDepositRate,
        uint256 minTradeAmount
    );

    // -----------------------------
    //        Constructor
    // -----------------------------
    constructor(
        address _usdtAddress,
        address _feeReceiver,
        uint256 _minTradeAmount,
        uint256 _sellerDepositRate,
        uint256 _buyerDepositRate
    ) {
        require(_usdtAddress != address(0), "Invalid USDT address");
        require(_feeReceiver != address(0), "Invalid feeReceiver");

        usdtToken = IERC20(_usdtAddress);
        feeReceiver = _feeReceiver;
        minTradeAmount = _minTradeAmount; // e.g. 50e18
        sellerDepositRate = _sellerDepositRate; // e.g. 21
        buyerDepositRate = _buyerDepositRate;   // e.g. 5

        // 初始化id从1开始，避免与0冲突
        nextBuyOrderId = 1;
        nextSellOrderId = 1;
        nextTradeId = 1;
    }

    // -----------------------------
    //         Modifiers
    // -----------------------------
    modifier whenNotPaused() {
        require(!paused, "New orders paused");
        _;
    }

    // -----------------------------
    //    管理员/Owner 函数
    // -----------------------------

    // 暂停或恢复新订单创建
    function pauseNewOrders(bool _paused) external onlyOwner {
        paused = _paused;
        emit Paused(_paused);
    }

    // 修改关键参数
    function setParams(
        uint256 _sellerDepositRate,
        uint256 _buyerDepositRate,
        uint256 _minTradeAmount
    ) external onlyOwner {
        sellerDepositRate = _sellerDepositRate;
        buyerDepositRate = _buyerDepositRate;
        minTradeAmount = _minTradeAmount;

        emit ParamsUpdated(_sellerDepositRate, _buyerDepositRate, _minTradeAmount);
    }

    // 修改手续费接收地址
    function setFeeReceiver(address _feeReceiver) external onlyOwner {
        require(_feeReceiver != address(0), "zero address");
        feeReceiver = _feeReceiver;
    }

    /**
     * @notice 平台介入，强制释放订单(Trade)，
     *         由平台决定如何分配USDT与扣除押金。
     * @param tradeId 指定交易ID
     * @param usdtToSeller    从买家锁定的金额中，最终要发给卖家多少USDT
     * @param usdtToBuyer     退还给买家的USDT（剩余部分）
     * @param sellerDepositPenalty 扣除卖家押金的数量
     * @param buyerDepositPenalty  扣除买家押金的数量
     * @dev 扣除的押金进入平台和对方的分配可以线下处理，也可以在这直接给 feeReceiver 或给对方。
     */
    function releaseTrade(
        uint256 tradeId,
        uint256 usdtToSeller,
        uint256 usdtToBuyer,
        uint256 sellerDepositPenalty,
        uint256 buyerDepositPenalty
    ) external onlyOwner nonReentrant {
        TradeInfo storage t = trades[tradeId];
        require(t.status == TradeStatus.Active, "Trade not active");

        // 1. 处理 Buyer 资金
        //    Buyer 在 trade 中锁定了 t.usdtAmount + t.buyerDeposit
        //    这里要检查要不要超出这个范围
        uint256 buyerLocked = t.usdtAmount + t.buyerDeposit;
        require(usdtToSeller + usdtToBuyer <= buyerLocked, "Exceed buyer locked");
        require(buyerDepositPenalty <= t.buyerDeposit, "Exceed buyer deposit");

        // 2. 处理 Seller 押金
        require(sellerDepositPenalty <= t.sellerDeposit, "Exceed seller deposit");

        // 3. 先扣除 Seller 押金
        uint256 sellerDepositLeft = t.sellerDeposit - sellerDepositPenalty;
        // 你可以把扣除的押金全给 feeReceiver或对半给Buyer/Platform，也可做更灵活的分配
        if (sellerDepositPenalty > 0) {
            // 这里示例：扣除的押金 -> feeReceiver
            safeTransferOut(address(usdtToken), feeReceiver, sellerDepositPenalty);
        }

        // 4. 退回剩余的 Seller 押金
        if (sellerDepositLeft > 0) {
            safeTransferOut(address(usdtToken), t.seller, sellerDepositLeft);
        }

        // 5. 给Seller的USDT
        if (usdtToSeller > 0) {
            // 要不要抽手续费？若平台认定仍然是“卖家部分成交”，则抽手续费
            // 这里假设：平台照常对 usdtToSeller 收取卖家手续费
            uint256 feeRate = _getSellerFeeRate(t.seller);
            uint256 feeAmount = (usdtToSeller * feeRate) / 10000; // feeRate基于万分比
            if (feeAmount > 0) {
                safeTransferOut(address(usdtToken), feeReceiver, feeAmount);
                usdtToSeller = usdtToSeller - feeAmount;
            }
            // 剩下给Seller
            safeTransferOut(address(usdtToken), t.seller, usdtToSeller);

            // 这里可以把( usdtToSeller + feeAmount )累加进sellerVolume
            sellerVolume[t.seller] += (usdtToSeller + feeAmount);
        }

        // 6. 退还给Buyer的USDT
        if (usdtToBuyer > 0) {
            safeTransferOut(address(usdtToken), t.buyer, usdtToBuyer);
        }

        // 7. 扣除Buyer押金
        uint256 buyerDepositLeft = t.buyerDeposit - buyerDepositPenalty;
        if (buyerDepositPenalty > 0) {
            // 示例：扣除的押金 -> feeReceiver
            safeTransferOut(address(usdtToken), feeReceiver, buyerDepositPenalty);
        }

        // 8. 退还Buyer押金剩余
        if (buyerDepositLeft > 0) {
            safeTransferOut(address(usdtToken), t.buyer, buyerDepositLeft);
        }

        // 更新交易状态
        t.status = TradeStatus.Released;

        emit TradeReleased(
            tradeId,
            usdtToSeller,
            usdtToBuyer,
            sellerDepositPenalty,
            buyerDepositPenalty
        );
    }

    // -----------------------------
    //       买单相关函数
    // -----------------------------

    /**
     * @notice 创建买单
     * @param xnmAmount    Buyer 期望买的XNM数量
     * @param price        单价(USDT, 18位精度假设)
     * @param maxDeliveryDays 最长交割天数
     */
    function createBuyOrder(
        uint256 xnmAmount,
        uint256 price,
        uint256 maxDeliveryDays
    ) external nonReentrant whenNotPaused {
        require(xnmAmount > 0, "Invalid XNM amount");
        // 总金额
        uint256 usdtTotal = xnmAmount * price;
        require(usdtTotal >= minTradeAmount, "Below minTradeAmount");

        // Buyer额外押金 = 订单金额 * buyerDepositRate%
        uint256 buyerDep = (usdtTotal * buyerDepositRate) / 100;

        // 合约需从Buyer扣 (usdtTotal + buyerDep)
        uint256 totalNeed = usdtTotal + buyerDep;

        // 检查用户是否先approve足够的USDT给本合约
        require(
            usdtToken.allowance(msg.sender, address(this)) >= totalNeed,
            "Not enough allowance"
        );

        // transferFrom Buyer
        safeTransferIn(address(usdtToken), msg.sender, totalNeed);

        // 存储BuyOrder
        uint256 orderId = nextBuyOrderId;
        nextBuyOrderId++;

        buyOrders[orderId] = BuyOrder({
            buyer: msg.sender,
            xnmAmount: xnmAmount,
            price: price,
            buyerDeposit: buyerDep,
            usdtTotal: usdtTotal,
            maxDeliveryDays: maxDeliveryDays,
            active: true
        });

        emit BuyOrderCreated(
            orderId,
            msg.sender,
            xnmAmount,
            price,
            usdtTotal,
            maxDeliveryDays
        );
    }

    // 撤销买单(仅当还未被吃单撮合)
    function cancelBuyOrder(uint256 orderId) external nonReentrant {
        BuyOrder storage o = buyOrders[orderId];
        require(o.active, "Not active");
        require(o.buyer == msg.sender, "Not your order");

        // 解锁资金: usdtTotal + buyerDeposit
        uint256 refundAmount = o.usdtTotal + o.buyerDeposit;

        // 退回Buyer
        safeTransferOut(address(usdtToken), msg.sender, refundAmount);

        // 标记为失效
        o.active = false;

        emit BuyOrderCancelled(orderId);
    }

    /**
     * @notice 卖家主动吃一个BuyOrder
     * @param buyOrderId 目标买单
     */
    function sellOrderMatchBuy(uint256 buyOrderId) external nonReentrant whenNotPaused {
        BuyOrder storage b = buyOrders[buyOrderId];
        require(b.active, "Buy order not active");

        // 卖家押金(按订单USDT金额 * sellerDepositRate%)
        uint256 sellerDep = (b.usdtTotal * sellerDepositRate) / 100;

        // 检查seller是否approve足够的USDT来做押金
        require(
            usdtToken.allowance(msg.sender, address(this)) >= sellerDep,
            "Not enough allowance for deposit"
        );
        // transferFrom Seller
        safeTransferIn(address(usdtToken), msg.sender, sellerDep);

        // 创建Trade
        uint256 tradeId = nextTradeId;
        nextTradeId++;

        trades[tradeId] = TradeInfo({
            buyer: b.buyer,
            seller: msg.sender,
            xnmAmount: b.xnmAmount,
            price: b.price,
            usdtAmount: b.usdtTotal,
            buyerDeposit: b.buyerDeposit,
            sellerDeposit: sellerDep,
            startTime: block.timestamp,
            maxDeliveryDays: b.maxDeliveryDays,
            status: TradeStatus.Active
        });

        // buyOrder失效
        b.active = false;

        emit TradeMatched(
            tradeId,
            b.buyer,
            msg.sender,
            b.xnmAmount,
            b.price,
            b.usdtTotal
        );
    }

    // -----------------------------
    //       卖单相关函数
    // -----------------------------

    /**
     * @notice 创建卖单
     * @param price         单价(USDT)
     * @param minXNM        最小可成交数量
     * @param maxXNM        最大可成交数量
     * @param maxDeliveryDays 最长交割天数
     */
    function createSellOrder(
        uint256 price,
        uint256 minXNM,
        uint256 maxXNM,
        uint256 maxDeliveryDays
    ) external nonReentrant whenNotPaused {
        require(minXNM <= maxXNM, "min>max");
        // 卖家押金 = (maxXNM * price) * sellerDepositRate%
        uint256 usdtTotalMax = maxXNM * price;
        require(usdtTotalMax >= minTradeAmount, "Below minTradeAmount for maxXNM");

        uint256 sellerDep = (usdtTotalMax * sellerDepositRate) / 100;

        // transferFrom Seller押金
        require(
            usdtToken.allowance(msg.sender, address(this)) >= sellerDep,
            "Not enough allowance for deposit"
        );
        safeTransferIn(address(usdtToken), msg.sender, sellerDep);

        uint256 orderId = nextSellOrderId;
        nextSellOrderId++;

        sellOrders[orderId] = SellOrder({
            seller: msg.sender,
            price: price,
            minXNM: minXNM,
            maxXNM: maxXNM,
            sellerDeposit: sellerDep,
            maxDeliveryDays: maxDeliveryDays,
            active: true
        });

        emit SellOrderCreated(
            orderId,
            msg.sender,
            price,
            minXNM,
            maxXNM,
            maxDeliveryDays
        );
    }

    // 撤销卖单(仅当未被吃单)
    function cancelSellOrder(uint256 orderId) external nonReentrant {
        SellOrder storage o = sellOrders[orderId];
        require(o.active, "Not active");
        require(o.seller == msg.sender, "Not your order");

        // 退还押金
        safeTransferOut(address(usdtToken), msg.sender, o.sellerDeposit);

        o.active = false;

        emit SellOrderCancelled(orderId);
    }

    /**
     * @notice Buyer 主动吃卖单
     * @param sellOrderId 卖单ID
     * @param xnmAmount   Buyer选择的成交数量(必须在[minXNM, maxXNM]之间)
     */
    function buyOrderMatchSell(uint256 sellOrderId, uint256 xnmAmount)
        external
        nonReentrant
        whenNotPaused
    {
        SellOrder storage s = sellOrders[sellOrderId];
        require(s.active, "Sell order not active");
        require(
            xnmAmount >= s.minXNM && xnmAmount <= s.maxXNM,
            "Amount out of range"
        );

        // 计算本次实际成交的USDT金额
        uint256 usdtAmount = xnmAmount * s.price;
        require(usdtAmount >= minTradeAmount, "Below minTradeAmount");

        // Buyer押金 = usdtAmount * buyerDepositRate%
        uint256 buyerDep = (usdtAmount * buyerDepositRate) / 100;

        // Buyer需支付 (usdtAmount + buyerDep)
        uint256 totalNeed = usdtAmount + buyerDep;

        // 先检查Buyer approve
        require(
            usdtToken.allowance(msg.sender, address(this)) >= totalNeed,
            "Not enough allowance"
        );

        // transferFrom Buyer
        safeTransferIn(address(usdtToken), msg.sender, totalNeed);

        // 计算卖家实际需要锁定的押金(对应此次实际成交金额)
        uint256 sellerDepFull = s.sellerDeposit; // 卖家之前存的押金(基于maxXNM)
        uint256 intendedDep = (usdtAmount * sellerDepositRate) / 100;
        require(intendedDep <= sellerDepFull, "Logic error? Not enough deposit locked");

        // 剩余的押金退还给卖家(若此次成交量 < maxXNM)
        uint256 refundDep = sellerDepFull - intendedDep;
        if (refundDep > 0) {
            safeTransferOut(address(usdtToken), s.seller, refundDep);
        }

        // 生成Trade
        uint256 tradeId = nextTradeId;
        nextTradeId++;

        trades[tradeId] = TradeInfo({
            buyer: msg.sender,
            seller: s.seller,
            xnmAmount: xnmAmount,
            price: s.price,
            usdtAmount: usdtAmount,
            buyerDeposit: buyerDep,
            sellerDeposit: intendedDep,
            startTime: block.timestamp,
            maxDeliveryDays: s.maxDeliveryDays,
            status: TradeStatus.Active
        });

        // 卖单失效(不管部分或全部，只此一次成交)
        s.active = false;

        emit TradeMatched(
            tradeId,
            msg.sender,
            s.seller,
            xnmAmount,
            s.price,
            usdtAmount
        );
    }

    // -----------------------------
    //       交易结算/完成
    // -----------------------------

    /**
     * @notice Buyer确认已经收到XNM(链下确认), 主动完成交易
     * @dev 此时合约把USDT付给卖家(扣手续费) + 退还买家押金 + 退还卖家押金
     */
    function completeTrade(uint256 tradeId) external nonReentrant {
        TradeInfo storage t = trades[tradeId];
        require(t.status == TradeStatus.Active, "Trade not active");
        require(msg.sender == t.buyer, "Only buyer can confirm");

        // 计算卖家应收 + 手续费
        uint256 usdtToSeller = t.usdtAmount;
        // 卖家费率
        uint256 feeRate = _getSellerFeeRate(t.seller);
        uint256 feeAmount = (usdtToSeller * feeRate) / 10000;
        if (feeAmount > 0) {
            safeTransferOut(address(usdtToken), feeReceiver, feeAmount);
            usdtToSeller = usdtToSeller - feeAmount;
        }

        // 1. 付款给卖家
        safeTransferOut(address(usdtToken), t.seller, usdtToSeller);
        // 累加卖家成交额(含手续费部分)
        sellerVolume[t.seller] += (t.usdtAmount);

        // 2. 退还Buyer押金
        if (t.buyerDeposit > 0) {
            safeTransferOut(address(usdtToken), t.buyer, t.buyerDeposit);
        }

        // 3. 退还Seller押金
        if (t.sellerDeposit > 0) {
            safeTransferOut(address(usdtToken), t.seller, t.sellerDeposit);
        }

        // 更新交易状态
        t.status = TradeStatus.Completed;

        emit TradeCompleted(tradeId);
    }

    // -----------------------------
    //       内部 & 辅助函数
    // -----------------------------

    /**
     * @dev 返回卖家当前手续费(万分比)，根据其历史成交额判断
     *
     *  < 10,000 => 5%(=500)
     *  [10k, 50k) => 4.5%(=450)
     *  [50k, 150k) => 4%(=400)
     *  [150k, 500k) => 3%(=300)
     *  [500k, 1M) => 2%(=200)
     *  >=1M => 1%(=100)
     *
     *  注意：卖家累积成交额 sellerVolume[seller] 需要和你在USDT的小数位保持一致(假设18位)
     */
    function _getSellerFeeRate(address seller) internal view returns (uint256) {
        uint256 vol = sellerVolume[seller];

        // 以18位精度计, 1e18 = 1 USDT
        // 10,000 USDT => 10000 * 1e18
        if (vol < 10000e18) {
            return 500; // 5%
        } else if (vol < 50000e18) {
            return 450; // 4.5%
        } else if (vol < 150000e18) {
            return 400; // 4%
        } else if (vol < 500000e18) {
            return 300; // 3%
        } else if (vol < 1000000e18) {
            return 200; // 2%
        } else {
            return 100; // 1%
        }
    }

    // 安全转入(collect token from user)
    function safeTransferIn(
        address token,
        address from,
        uint256 amount
    ) internal {
        bool ok = IERC20(token).transferFrom(from, address(this), amount);
        require(ok, "transferFrom failed");
    }

    // 安全转出(transfer token to user)
    function safeTransferOut(
        address token,
        address to,
        uint256 amount
    ) internal {
        bool ok = IERC20(token).transfer(to, amount);
        require(ok, "transfer failed");
    }
}