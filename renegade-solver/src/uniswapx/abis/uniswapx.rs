//! ABI types for UniswapX

use alloy::sol;

// Copied from [UniswapX PriorityOrderReactor.sol](https://github.com/Uniswap/uniswapx/blob/main/src/lib/PriorityOrderLib.sol)
sol! {
    #[derive(Debug)]
    /// @dev UniswapX Priority Order Reactor interface
    contract PriorityOrderReactor {
        struct OrderInfo {
            address reactor;
            address swapper;
            uint256 nonce;
            uint256 deadline;
            address additionalValidationContract;
            bytes additionalValidationData;
        }

        struct PriorityCosignerData {
            // the block at which the order can be executed (overrides auctionStartBlock)
            uint256 auctionTargetBlock;
        }

        struct PriorityInput {
            address token;
            uint256 amount;
            // the less amount of input to be received per wei of priority fee
            uint256 mpsPerPriorityFeeWei;
        }

        struct PriorityOutput {

            address token;
            uint256 amount;
            // the extra amount of output to be paid per wei of priority fee
            uint256 mpsPerPriorityFeeWei;
            address recipient;
        }

        /// @dev External struct used to specify priority orders
        struct PriorityOrder {
            // generic order information
            OrderInfo info;
            // The address which may cosign the order
            address cosigner;
            // the block at which the order can be executed
            uint256 auctionStartBlock;
            // the baseline priority fee for the order, above which additional taxes are applied
            uint256 baselinePriorityFeeWei;
            // The tokens that the swapper will provide when settling the order
            PriorityInput input;
            // The tokens that must be received to satisfy the order
            PriorityOutput[] outputs;
            // signed over by the cosigner
            PriorityCosignerData cosignerData;
            // signature from the cosigner over (orderHash || cosignerData)
            bytes cosignature;
        }

        struct SignedOrder {
            bytes order;
            bytes signature;
        }
    }
}
