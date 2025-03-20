// test/xenblocksHashMarket.test.js
const { expect } = require("chai");

const TestUSDT = artifacts.require("TestUSDT");
const XenblocksHashMarket = artifacts.require("XenblocksHashMarket");

contract("XenblocksHashMarket", (accounts) => {
  let usdt;         // Mock USDT合约实例
  let market;       // XenblocksHashMarket合约实例
  const owner = accounts[0];   // 部署者
  const alice = accounts[1];   // 买家
  const bob = accounts[2];     // 卖家
  const feeReceiver = accounts[9]; // 手续费接收地址

  // 部署前准备的一些配置
  const INIT_SUPPLY = web3.utils.toWei("1000000", "ether"); // 1,000,000 USDT
  const MIN_TRADE_AMOUNT = web3.utils.toWei("50", "ether"); // 50 USDT
  const SELLER_DEPOSIT_RATE = 21; // 21%
  const BUYER_DEPOSIT_RATE = 5;   // 5%

  before(async () => {
    // 1) 部署 Mock USDT
    usdt = await TestUSDT.new(INIT_SUPPLY, { from: owner });
    // owner 拥有 1,000,000 tUSDT

    // 2) 部署市场合约
    market = await XenblocksHashMarket.new(
      usdt.address,
      feeReceiver,
      MIN_TRADE_AMOUNT,
      SELLER_DEPOSIT_RATE,
      BUYER_DEPOSIT_RATE,
      { from: owner }
    );
  });

  describe("Initial checks", () => {
    it("should have correct constructor params", async () => {
      const actualUsdt = await market.usdtToken();
      expect(actualUsdt).to.equal(usdt.address);

      const actualFeeReceiver = await market.feeReceiver();
      expect(actualFeeReceiver).to.equal(feeReceiver);

      const minTrade = await market.minTradeAmount();
      expect(minTrade.toString()).to.equal(MIN_TRADE_AMOUNT);

      const sRate = await market.sellerDepositRate();
      expect(sRate.toString()).to.equal(String(SELLER_DEPOSIT_RATE));

      const bRate = await market.buyerDepositRate();
      expect(bRate.toString()).to.equal(String(BUYER_DEPOSIT_RATE));

      const paused = await market.paused();
      expect(paused).to.equal(false); // 默认不暂停
    });

    it("owner can pause and unpause new orders", async () => {
      // 先暂停
      await market.pauseNewOrders(true, { from: owner });
      let paused = await market.paused();
      expect(paused).to.equal(true);

      // 再取消暂停
      await market.pauseNewOrders(false, { from: owner });
      paused = await market.paused();
      expect(paused).to.equal(false);
    });

    it("owner can update params", async () => {
      // 改下最小交易额 -> 100 USDT
      const newMin = web3.utils.toWei("100", "ether");
      await market.setParams(30, 10, newMin, { from: owner });
      const sellerR = await market.sellerDepositRate();
      const buyerR = await market.buyerDepositRate();
      const minT = await market.minTradeAmount();

      expect(sellerR.toString()).to.equal("30");
      expect(buyerR.toString()).to.equal("10");
      expect(minT.toString()).to.equal(newMin);

      // 改回原先
      await market.setParams(SELLER_DEPOSIT_RATE, BUYER_DEPOSIT_RATE, MIN_TRADE_AMOUNT, { from: owner });
    });
  });

  describe("Buy Order", () => {
    it("should revert if below minTradeAmount", async () => {
      // alice尝试挂买单: xnmAmount=10, price=1 => usdtTotal=10
      // 但我们minTradeAmount=50 => 10 < 50 => revert
      try {
        await market.createBuyOrder(
          10,
          web3.utils.toWei("1", "ether"),
          7,
          { from: alice }
        );
        expect.fail("Should revert: Below minTradeAmount");
      } catch (err) {
        expect(err.reason).to.include("Below minTradeAmount");
      }
    });

    it("should revert if not enough allowance", async () => {
      // alice想挂买单: xnm=100, price=1 => usdtTotal=100
      // totalNeed = 100 + 5 => 105
      // 还没 approve
      try {
        await market.createBuyOrder(
          100,
          web3.utils.toWei("1", "ether"),
          7,
          { from: alice }
        );
        expect.fail("Should revert: Not enough allowance");
      } catch (err) {
        expect(err.reason).to.include("Not enough allowance");
      }
    });

    it("should create buy order successfully if enough allowance", async () => {
      // 1) 先给market approve
      // xnm=100, price=1 => total=100, buyerDep=5 => totalNeed=105
      const totalNeed = web3.utils.toWei("105", "ether");
      // alice先从owner那里转一点USDT
      await usdt.transfer(alice, totalNeed, { from: owner });

      // alice approve
      await usdt.approve(market.address, totalNeed, { from: alice });

      // 2) createBuyOrder
      const tx = await market.createBuyOrder(
        100,                              // XNM
        web3.utils.toWei("1", "ether"),   // price=1 USDT
        7,                                // maxDeliveryDays
        { from: alice }
      );

      // 从事件中查到 orderId = 1 (如果是首次)
      const logs = tx.logs.filter(l => l.event === "BuyOrderCreated");
      expect(logs.length).to.equal(1);
      const orderId = logs[0].args.orderId.toString();
      expect(orderId).to.equal("1"); // 说明是第一个买单

      // check order state
      const bo = await market.buyOrders(orderId);
      expect(bo.buyer).to.equal(alice);
      expect(bo.active).to.equal(true);
      // bo.xnmAmount=100, bo.price=1e18, bo.usdtTotal=100e18, bo.buyerDeposit=5e18
    });

    it("can cancel buy order", async () => {
      // cancelBuyOrder(1)
      const orderId = 1;
      const boBefore = await market.buyOrders(orderId);
      // usdtTotal + buyerDeposit => 100 + 5 =105
      // 看看alice钱包Balance
      const aliceBalBefore = await usdt.balanceOf(alice);

      const tx = await market.cancelBuyOrder(orderId, { from: alice });
      const logs = tx.logs.filter(l => l.event === "BuyOrderCancelled");
      expect(logs.length).to.equal(1);
      expect(logs[0].args.orderId.toString()).to.equal("1");

      // buyer资金退回
      const aliceBalAfter = await usdt.balanceOf(alice);
      const diff = aliceBalAfter.sub(aliceBalBefore).toString();
      // diff应该是105e18
      expect(diff).to.equal(web3.utils.toWei("105", "ether"));

      const boAfter = await market.buyOrders(orderId);
      expect(boAfter.active).to.equal(false);
    });
  });

  describe("Sell Order", () => {
    it("should revert if minXNM>maxXNM", async () => {
      try {
        await market.createSellOrder(
          web3.utils.toWei("1", "ether"),
          200, 100, // minXNM=200, maxXNM=100
          7,
          { from: bob }
        );
        expect.fail("Should revert: min>max");
      } catch (err) {
        expect(err.reason).to.include("min>max");
      }
    });

    it("should revert if maxXNM * price < minTradeAmount", async () => {
      // maxXNM=10, price=1 => total=10 => < 50 => revert
      try {
        await market.createSellOrder(
          web3.utils.toWei("1", "ether"), // price=1
          5, 10, 7, 
          { from: bob }
        );
        expect.fail("Should revert: Below minTradeAmount");
      } catch (err) {
        expect(err.reason).to.include("Below minTradeAmount");
      }
    });

    it("should revert if not enough allowance for deposit", async () => {
      // maxXNM=100, price=1 => total=100 => deposit=21 => 21
      // bob还没approve
      try {
        await market.createSellOrder(
          web3.utils.toWei("1", "ether"),
          50, // min
          100, // max
          7,
          { from: bob }
        );
        expect.fail("Should revert: Not enough allowance for deposit");
      } catch (err) {
        expect(err.reason).to.include("Not enough allowance for deposit");
      }
    });

    it("should create sell order successfully", async () => {
      // bob先要有足够的 USDT 去做押金
      // depositRate=21 => maxXNM=100 => total=100 => deposit=21
      const depositNeed = web3.utils.toWei("21", "ether");

      // 给bob一点usdt
      await usdt.transfer(bob, depositNeed, { from: owner });

      // bob approve
      await usdt.approve(market.address, depositNeed, { from: bob });

      // create sell order
      const tx = await market.createSellOrder(
        web3.utils.toWei("1", "ether"), // price=1
        50,
        100,
        7,
        { from: bob }
      );

      const logs = tx.logs.filter(l => l.event === "SellOrderCreated");
      expect(logs.length).to.equal(1);
      const soId = logs[0].args.orderId.toString();
      expect(soId).to.equal("1"); // 第一个卖单

      // 检查sellOrders(1)
      const so = await market.sellOrders(1);
      expect(so.seller).to.equal(bob);
      expect(so.active).to.equal(true);
      // so.sellerDeposit=21e18
    });

    it("should cancel sell order", async () => {
      // cancelSellOrder(1)
      const orderId = 1;
      const soBefore = await market.sellOrders(orderId);
      const depositLocked = soBefore.sellerDeposit.toString(); // 21e18
      const bobBalBefore = await usdt.balanceOf(bob);

      const tx = await market.cancelSellOrder(orderId, { from: bob });
      const logs = tx.logs.filter(l => l.event === "SellOrderCancelled");
      expect(logs.length).to.equal(1);
      expect(logs[0].args.orderId.toString()).to.equal("1");

      const bobBalAfter = await usdt.balanceOf(bob);
      const diff = bobBalAfter.sub(bobBalBefore).toString();
      // diff 应该是21e18
      expect(diff).to.equal(depositLocked);

      const soAfter = await market.sellOrders(orderId);
      expect(soAfter.active).to.equal(false);
    });
  });

  describe("Match Orders & Complete Trade", () => {
    let buyOrderId;
    let sellOrderId;
    let tradeId;

    it("createBuyOrder -> successfully", async () => {
      // alice再挂一个buy order
      // xnm=200, price=1 => total=200 => deposit=10 => 210
      const totalNeed = web3.utils.toWei("210", "ether");
      await usdt.transfer(alice, totalNeed, { from: owner });
      await usdt.approve(market.address, totalNeed, { from: alice });

      const tx = await market.createBuyOrder(
        200,
        web3.utils.toWei("1", "ether"),
        7,
        { from: alice }
      );

      const log = tx.logs.find(l => l.event === "BuyOrderCreated");
      buyOrderId = log.args.orderId.toNumber(); // e.g. 2
      expect(buyOrderId).to.be.greaterThan(1);
    });

    it("sellOrderMatchBuy -> fail if seller not enough deposit allowance", async () => {
      // xnm=200 => total=200 => deposit=42 => bob 需approve
      try {
        await market.sellOrderMatchBuy(buyOrderId, { from: bob });
        expect.fail("Should revert: Not enough allowance for deposit");
      } catch (err) {
        expect(err.reason).to.include("Not enough allowance for deposit");
      }
    });

    it("sellOrderMatchBuy -> success", async () => {
      // bob 先转 42 USDT
      const depositNeed = web3.utils.toWei("42", "ether");
      await usdt.transfer(bob, depositNeed, { from: owner });
      await usdt.approve(market.address, depositNeed, { from: bob });

      const tx = await market.sellOrderMatchBuy(buyOrderId, { from: bob });
      const log = tx.logs.find(l => l.event === "TradeMatched");
      expect(log).to.exist;
      tradeId = log.args.tradeId.toNumber();
      expect(tradeId).to.be.greaterThan(0);

      // buyOrder( buyOrderId ) 应该变 inactive
      const bo = await market.buyOrders(buyOrderId);
      expect(bo.active).to.equal(false);

      // trade信息
      const t = await market.trades(tradeId);
      expect(t.buyer).to.equal(alice);
      expect(t.seller).to.equal(bob);
      expect(t.xnmAmount.toString()).to.equal("200");
      expect(t.usdtAmount.toString()).to.equal(web3.utils.toWei("200", "ether"));
      expect(t.status.toString()).to.equal("0"); // Active=0
    });

    it("completeTrade -> revert if not buyer", async () => {
      try {
        await market.completeTrade(tradeId, { from: bob });
        expect.fail("Should revert: Only buyer can confirm");
      } catch (err) {
        expect(err.reason).to.include("Only buyer can confirm");
      }
    });

    it("completeTrade -> success (buyer calls)", async () => {
      // alice calls completeTrade => 卖家bob拿到USDT(扣手续费)
      const bobBalBefore = await usdt.balanceOf(bob);
      const feeRecvBefore = await usdt.balanceOf(feeReceiver);

      const tx = await market.completeTrade(tradeId, { from: alice });
      const log = tx.logs.find(l => l.event === "TradeCompleted");
      expect(log).to.exist;
      expect(log.args.tradeId.toString()).to.equal(String(tradeId));

      const bobBalAfter = await usdt.balanceOf(bob);
      const feeRecvAfter = await usdt.balanceOf(feeReceiver);

      // 卖家应收 usdt=200 => fee 多少?
      // 卖家初次 => <10000 => fee=5% => 200 * 5% = 10
      // bob 实际得到=190
      // feeReceiver 得到=10
      const deltaBob = web3.utils.fromWei(bobBalAfter.sub(bobBalBefore));
      const deltaFee = web3.utils.fromWei(feeRecvAfter.sub(feeRecvBefore));
      expect(deltaBob).to.equal("190");
      expect(deltaFee).to.equal("10");

      // trade 状态 -> completed=1
      const t = await market.trades(tradeId);
      expect(t.status.toString()).to.equal("1");
    });
  });

  describe("Platform releaseTrade scenario", () => {
    let soId;
    let tId;

    it("createSellOrder => buyer eats => trade Active", async () => {
      // 先让bob创建一个卖单
      // maxXNM=100 => total=100 => deposit=21
      await usdt.transfer(bob, web3.utils.toWei("21", "ether"), { from: owner });
      await usdt.approve(market.address, web3.utils.toWei("21", "ether"), { from: bob });
      const tx1 = await market.createSellOrder(
        web3.utils.toWei("1", "ether"), // price=1
        50,
        100,
        7,
        { from: bob }
      );
      const sellLog = tx1.logs.find(l => l.event === "SellOrderCreated");
      soId = sellLog.args.orderId.toNumber();

      // buyer=alice 吃单
      // 选择 xnmAmount=100 => total=100 => buyerDep=5 => totalNeed=105
      await usdt.transfer(alice, web3.utils.toWei("105", "ether"), { from: owner });
      await usdt.approve(market.address, web3.utils.toWei("105", "ether"), { from: alice });

      const tx2 = await market.buyOrderMatchSell(soId, 100, { from: alice });
      const matchedLog = tx2.logs.find(l => l.event === "TradeMatched");
      expect(matchedLog).to.exist;
      tId = matchedLog.args.tradeId.toNumber();
    });

    it("releaseTrade => platform can forcibly release with custom distribution", async () => {
      // check trade is Active
      const t = await market.trades(tId);
      expect(t.status.toString()).to.equal("0"); // Active

      // param: (tradeId, usdtToSeller, usdtToBuyer, sellerDepositPenalty, buyerDepositPenalty)
      // Suppose admin判定: 卖家违约, 扣seller 50%押金
      // => sellerDeposit=21 => penalty=10 => feeReceiver
      // => buyer get some partial refund from locked USDT ?

      const sellerDep = t.sellerDeposit.toString(); // 21e18
      // 先看资金锁情况: buyer locked= (usdtAmount=100) + (buyerDeposit=5)=105

      // Suppose: 
      //    usdtToSeller=40, usdtToBuyer=65 => total=105
      //    sellerDepositPenalty=10 => buyerDepositPenalty=2
      //  => 这样还要别忘了,  10/2 的押金都进 feeReceiver(例子)
      // 你可以任意分配

      const bobBalBefore = await usdt.balanceOf(bob);
      const aliceBalBefore = await usdt.balanceOf(alice);
      const feeBefore = await usdt.balanceOf(feeReceiver);

      await market.releaseTrade(
        tId,
        web3.utils.toWei("40", "ether"), // usdtToSeller
        web3.utils.toWei("65", "ether"), // usdtToBuyer
        web3.utils.toWei("10", "ether"), // sellerDepositPenalty
        web3.utils.toWei("2", "ether"),  // buyerDepositPenalty
        { from: owner }
      );

      const bobBalAfter = await usdt.balanceOf(bob);
      const aliceBalAfter = await usdt.balanceOf(alice);
      const feeAfter = await usdt.balanceOf(feeReceiver);

      // bob得到 +40
      const deltaBob = bobBalAfter.sub(bobBalBefore).toString();
      expect(deltaBob).to.equal(web3.utils.toWei("40", "ether"));

      // alice得到 +65
      const deltaAlice = aliceBalAfter.sub(aliceBalBefore).toString();
      expect(deltaAlice).to.equal(web3.utils.toWei("65", "ether"));

      // feeReceiver 得到 + (10 + 2)=12
      const deltaFee = feeAfter.sub(feeBefore).toString();
      expect(deltaFee).to.equal(web3.utils.toWei("12", "ether"));

      // check trade status => Released=2
      const t2 = await market.trades(tId);
      expect(t2.status.toString()).to.equal("2");
    });
  });
});