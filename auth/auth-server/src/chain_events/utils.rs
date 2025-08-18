//! Utility functions for the on-chain event listener
use alloy::sol;

sol! {
    /// The CoW Protocol GPv2Settlement contract
    contract GPv2Settlement {
        event Trade(
            address indexed owner,
            address sellToken,
            address buyToken,
            uint256 sellAmount,
            uint256 buyAmount,
            uint256 feeAmount,
            bytes   orderUid
        );
    }
}

sol! {
    contract IERC20 {
        event Transfer(address indexed from, address indexed to, uint256 value);
    }
}
