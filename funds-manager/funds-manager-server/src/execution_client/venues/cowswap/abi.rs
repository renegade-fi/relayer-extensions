//! Solidity type bindings for Cowswap contract types

#![allow(missing_docs)]

use alloy_sol_types::sol;

sol! {
    // Taken from <https://github.com/cowprotocol/contracts/blob/main/src/contracts/libraries/GPv2Order.sol#L11>
    struct Order {
        address sellToken;
        address buyToken;
        address receiver;
        uint256 sellAmount;
        uint256 buyAmount;
        uint32 validTo;
        bytes32 appData;
        uint256 feeAmount;
        string kind;
        bool partiallyFillable;
        string sellTokenBalance;
        string buyTokenBalance;
    }
}
