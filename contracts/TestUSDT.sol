// contracts/TestUSDT.sol
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/**
 * @title TestUSDT
 * @dev 一个简化的ERC20，用来在测试环境里充当 USDT
 */
contract TestUSDT {
    string public name = "Test USDT";
    string public symbol = "tUSDT";
    uint8 public decimals = 18;

    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    constructor(uint256 _initSupply) {
        totalSupply = _initSupply;
        balanceOf[msg.sender] = _initSupply;
        emit Transfer(address(0), msg.sender, _initSupply);
    }

    function transfer(address recipient, uint256 amount) external returns(bool) {
        require(balanceOf[msg.sender] >= amount, "Not enough balance");
        balanceOf[msg.sender] -= amount;
        balanceOf[recipient] += amount;
        emit Transfer(msg.sender, recipient, amount);
        return true;
    }

    function approve(address spender, uint256 amount) external returns(bool) {
        allowance[msg.sender][spender] = amount;
        emit Approval(msg.sender, spender, amount);
        return true;
    }

    function transferFrom(
        address sender,
        address recipient,
        uint256 amount
    ) external returns(bool) {
        require(balanceOf[sender] >= amount, "Not enough balance");
        require(allowance[sender][msg.sender] >= amount, "Not enough allowance");
        allowance[sender][msg.sender] -= amount;
        balanceOf[sender] -= amount;
        balanceOf[recipient] += amount;
        emit Transfer(sender, recipient, amount);
        return true;
    }
}