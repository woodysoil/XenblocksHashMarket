const XenblocksHashMarket = artifacts.require("XenblocksHashMarket");

module.exports = async function (deployer) {
  // 这里是合约构造函数需要的参数
  // constructor(
  //   address _usdtAddress,
  //   address _feeReceiver,
  //   uint256 _minTradeAmount,
  //   uint256 _sellerDepositRate,
  //   uint256 _buyerDepositRate
  // )

  // 因为现在是在本地测试，所以随便传一些地址 & 参数即可：
  // (真要和USDT交互需要Mock合约或者换到测试网再做更真实测试)

  const mockUSDTAddress = "0x1111111111111111111111111111111111111112"; 
  const feeReceiver = "0x1111111111111111111111111111111111111111"; 
  // minTradeAmount = 50 USDT => 用 50*10^18 表示(如果按18位)
  const minTradeAmount = web3.utils.toWei("50", "ether"); 
  const sellerDepositRate = 21; // 21%
  const buyerDepositRate = 5;   // 5%

  await deployer.deploy(
    XenblocksHashMarket,
    mockUSDTAddress,
    feeReceiver,
    minTradeAmount,
    sellerDepositRate,
    buyerDepositRate
  );
};