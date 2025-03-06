//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use alloy_primitives::{Address, Bytes as AlloyBytes, U256 as AlloyU256};
use alloy_sol_types::{sol, SolCall};
use auth_server_api::SponsoredMatchResponse;
use bigdecimal::{num_bigint::BigInt, BigDecimal, FromPrimitive};
use bytes::Bytes;
use ethers::contract::abigen;
use ethers::types::{transaction::eip2718::TypedTransaction, TxHash, U256};
use ethers::utils::format_ether;
use http::header::CONTENT_LENGTH;
use http::Response;
use renegade_arbitrum_client::abi::{
    processAtomicMatchSettleCall, processAtomicMatchSettleWithReceiverCall,
};
use renegade_common::types::token::Token;
use renegade_constants::NATIVE_ASSET_ADDRESS;
use tracing::{info, warn};

use renegade_api::http::external_match::{AtomicMatchApiBundle, ExternalMatchResponse};

use super::Server;
use crate::error::AuthServerError;
use crate::server::helpers::{gen_signed_sponsorship_nonce, get_selector};
use crate::telemetry::helpers::record_gas_sponsorship_metrics;

// -------
// | ABI |
// -------

// The ABI for gas sponsorship functions
sol! {
    function sponsorAtomicMatchSettle(bytes internal_party_match_payload, bytes valid_match_settle_atomic_statement, bytes match_proofs, bytes match_linking_proofs, address refund_address, uint256 nonce, bytes signature) external payable;
    function sponsorAtomicMatchSettleWithReceiver(address receiver, bytes internal_party_match_payload, bytes valid_match_settle_atomic_statement, bytes match_proofs, bytes match_linking_proofs, address refund_address, uint256 nonce, bytes signature) external payable;
    function sponsorAtomicMatchSettleWithRefundOptions(address receiver, bytes internal_party_match_payload, bytes valid_match_settle_atomic_statement, bytes match_proofs, bytes match_linking_proofs, address refund_address, uint256 nonce, bool refund_native_eth, uint256 conversion_rate, bytes signature) external payable;
}

// The ABI for gas sponsorship events
abigen!(
    GasSponsorContract,
    r#"[
        event SponsoredExternalMatch(uint256 indexed amount, uint256 indexed nonce)
    ]"#
);

impl sponsorAtomicMatchSettleWithRefundOptionsCall {
    /// Create a `sponsorAtomicMatchSettleWithRefundOptions` call from
    /// `processAtomicMatchSettle` calldata
    pub fn from_process_atomic_match_settle_calldata(
        calldata: &[u8],
        refund_address: Address,
        nonce: AlloyU256,
        refund_native_eth: bool,
        conversion_rate: AlloyU256,
        signature: AlloyBytes,
    ) -> Result<Self, AuthServerError> {
        let processAtomicMatchSettleCall {
            internal_party_match_payload,
            valid_match_settle_atomic_statement,
            match_proofs,
            match_linking_proofs,
        } = processAtomicMatchSettleCall::abi_decode(
            calldata, true, // validate
        )
        .map_err(AuthServerError::gas_sponsorship)?;

        Ok(sponsorAtomicMatchSettleWithRefundOptionsCall {
            receiver: Address::ZERO,
            internal_party_match_payload,
            valid_match_settle_atomic_statement,
            match_proofs,
            match_linking_proofs,
            refund_address,
            nonce,
            refund_native_eth,
            conversion_rate,
            signature,
        })
    }

    /// Create a `sponsorAtomicMatchSettleWithRefundOptions` call from
    /// `processAtomicMatchSettleWithReceiver` calldata
    pub fn from_process_atomic_match_settle_with_receiver_calldata(
        calldata: &[u8],
        refund_address: Address,
        nonce: AlloyU256,
        refund_native_eth: bool,
        conversion_rate: AlloyU256,
        signature: AlloyBytes,
    ) -> Result<Self, AuthServerError> {
        let processAtomicMatchSettleWithReceiverCall {
            receiver,
            internal_party_match_payload,
            valid_match_settle_atomic_statement,
            match_proofs,
            match_linking_proofs,
        } = processAtomicMatchSettleWithReceiverCall::abi_decode(
            calldata, true, // validate
        )
        .map_err(AuthServerError::gas_sponsorship)?;

        Ok(sponsorAtomicMatchSettleWithRefundOptionsCall {
            receiver,
            internal_party_match_payload,
            valid_match_settle_atomic_statement,
            match_proofs,
            match_linking_proofs,
            refund_address,
            nonce,
            refund_native_eth,
            conversion_rate,
            signature,
        })
    }
}

// ---------------
// | Server Impl |
// ---------------

/// Handle a proxied request
impl Server {
    /// Mutate a quote assembly response to invoke gas sponsorship
    pub(crate) async fn mutate_response_for_gas_sponsorship(
        &self,
        resp: &mut Response<Bytes>,
        is_sponsored: bool,
        refund_address: Address,
        refund_native_eth: bool,
    ) -> Result<(), AuthServerError> {
        let mut relayer_external_match_resp: ExternalMatchResponse =
            serde_json::from_slice(resp.body()).map_err(AuthServerError::serde)?;

        relayer_external_match_resp.match_bundle.settlement_tx.set_to(self.gas_sponsor_address);

        if is_sponsored {
            info!("Sponsoring match bundle via gas sponsor");

            let conversion_rate = self
                .maybe_fetch_conversion_rate(&relayer_external_match_resp, refund_native_eth)
                .await?;

            let gas_sponsor_calldata = self
                .generate_gas_sponsor_calldata(
                    &relayer_external_match_resp,
                    refund_address,
                    refund_native_eth,
                    conversion_rate,
                )?
                .into();

            relayer_external_match_resp.match_bundle.settlement_tx.set_data(gas_sponsor_calldata);
        }

        let external_match_resp = SponsoredMatchResponse {
            match_bundle: relayer_external_match_resp.match_bundle,
            is_sponsored,
        };

        let body =
            Bytes::from(serde_json::to_vec(&external_match_resp).map_err(AuthServerError::serde)?);

        resp.headers_mut().insert(CONTENT_LENGTH, body.len().into());
        *resp.body_mut() = body;

        Ok(())
    }

    /// Fetch the conversion rate from ETH to the buy-side token in the trade
    /// from the price reporter, if necessary.
    /// The conversion rate is in units of
    /// `token/wei * 10^CONVERSION_RATE_SCALE`
    #[allow(clippy::unused_async)]
    async fn maybe_fetch_conversion_rate(
        &self,
        external_match_resp: &ExternalMatchResponse,
        refund_native_eth: bool,
    ) -> Result<Option<AlloyU256>, AuthServerError> {
        let buy_mint = &external_match_resp.match_bundle.receive.mint;
        let native_eth_buy = buy_mint.to_lowercase() == NATIVE_ASSET_ADDRESS.to_lowercase();

        // If we're deliberately refunding via native ETH, or the buy-side token
        // is native ETH, we don't need to get a conversion rate
        if refund_native_eth || native_eth_buy {
            return Ok(None);
        }

        // Get ETH price
        let eth_price_f64 = self.price_reporter_client.get_eth_price().await?;

        // Get TOKEN price
        let buy_token_price_f64 = self.price_reporter_client.get_binance_price(buy_mint).await?;

        // Get the number of decimals for the buy-side token
        let buy_token_decimals: u32 = Token::from_addr(buy_mint)
            .get_decimals()
            .ok_or(AuthServerError::gas_sponsorship("buy-side token does not have known decimals"))?
            .into();

        // Compute the conversion rate
        let conversion_rate =
            Self::compute_conversion_rate(eth_price_f64, buy_token_price_f64, buy_token_decimals)?;

        Ok(Some(conversion_rate))
    }

    /// Given the price of ETH and the buy-side token,
    /// compute the conversion rate in units of
    /// `token/wei * 10^CONVERSION_RATE_SCALE`
    fn compute_conversion_rate(
        eth_price: f64,
        buy_token_price: f64,
        buy_token_decimals: u32,
    ) -> Result<AlloyU256, AuthServerError> {
        // USDT per ETH
        let eth_price = BigDecimal::from_f64(eth_price)
            .ok_or(AuthServerError::gas_sponsorship("failed to convert ETH price to BigDecimal"))?;

        // USDT per TOKEN
        let buy_token_price =
            BigDecimal::from_f64(buy_token_price).ok_or(AuthServerError::gas_sponsorship(
                "failed to convert buy-side token price to BigDecimal",
            ))?;

        // Compute conversion rate of TOKEN per ETH
        let conversion_rate = eth_price / buy_token_price;

        // Decimal-adjust the rate to represent (smallest-denomination) *units* of TOKEN
        // per ETH
        let adjustment: BigDecimal = BigInt::from(10).pow(buy_token_decimals).into();
        let conversion_rate_adjusted = conversion_rate * adjustment;

        // Convert the scaled rate to a U256. We can use the `BigInt` component of the
        // `BigDecimal` directly because we round to 0 digits after the decimal.
        let (conversion_rate_bigint, _) =
            conversion_rate_adjusted.round(0 /* round_digits */).into_bigint_and_scale();

        AlloyU256::try_from(conversion_rate_bigint).map_err(AuthServerError::gas_sponsorship)
    }

    /// Generate the calldata for sponsoring the given match via the gas sponsor
    fn generate_gas_sponsor_calldata(
        &self,
        external_match_resp: &ExternalMatchResponse,
        refund_address: Address,
        refund_native_eth: bool,
        conversion_rate: Option<AlloyU256>,
    ) -> Result<Bytes, AuthServerError> {
        let (nonce, signature) = gen_signed_sponsorship_nonce(
            refund_address,
            conversion_rate,
            &self.gas_sponsor_auth_key,
        )?;

        let conversion_rate = conversion_rate.unwrap_or_default();

        let calldata = external_match_resp
            .match_bundle
            .settlement_tx
            .data()
            .ok_or(AuthServerError::gas_sponsorship("expected calldata"))?;

        let selector = get_selector(calldata)?;

        let gas_sponsor_call = match selector {
            processAtomicMatchSettleCall::SELECTOR => {
                sponsorAtomicMatchSettleWithRefundOptionsCall::from_process_atomic_match_settle_calldata(
                    calldata,
                    refund_address,
                    nonce,
                    refund_native_eth,
                    conversion_rate,
                    signature,
                )
            },
            processAtomicMatchSettleWithReceiverCall::SELECTOR => {
                sponsorAtomicMatchSettleWithRefundOptionsCall::from_process_atomic_match_settle_with_receiver_calldata(
                    calldata,
                    refund_address,
                    nonce,
                    refund_native_eth,
                    conversion_rate,
                    signature,
                )
            },
            _ => {
                return Err(AuthServerError::gas_sponsorship("invalid selector"));
            },
        }?;

        let calldata = gas_sponsor_call.abi_encode().into();

        Ok(calldata)
    }

    /// Get the amount of Ether spent to sponsor the given settlement
    /// transaction, and the associated transaction hash
    async fn get_sponsorship_amount_and_tx(
        &self,
        settlement_tx: &TypedTransaction,
    ) -> Result<Option<(U256, TxHash)>, AuthServerError> {
        // Parse the nonce from the TX calldata
        let calldata =
            settlement_tx.data().ok_or(AuthServerError::gas_sponsorship("expected calldata"))?;

        let selector = get_selector(calldata)?;

        let nonce = match selector {
            sponsorAtomicMatchSettleCall::SELECTOR => {
                Self::get_nonce_from_sponsor_atomic_match_calldata(calldata)?
            },
            sponsorAtomicMatchSettleWithReceiverCall::SELECTOR => {
                Self::get_nonce_from_sponsor_atomic_match_with_receiver_calldata(calldata)?
            },
            _ => {
                return Err(AuthServerError::gas_sponsorship("invalid selector"));
            },
        };

        // Search for the `AmountSponsored` event for the given nonce
        let filter =
            GasSponsorContract::new(self.gas_sponsor_address, self.arbitrum_client.client())
                .event::<SponsoredExternalMatchFilter>()
                .address(self.gas_sponsor_address.into())
                .topic2(nonce)
                .from_block(self.start_block_num);

        let events = filter.query_with_meta().await.map_err(AuthServerError::gas_sponsorship)?;

        // If no event was found, we assume that gas was not sponsored for this nonce.
        // This could be the case if the gas sponsor was underfunded or paused.
        let amount_sponsored_with_tx =
            events.last().map(|(event, meta)| (event.amount, meta.transaction_hash));

        if amount_sponsored_with_tx.is_none() {
            warn!("No gas sponsorship event found for nonce: {}", nonce);
        }

        Ok(amount_sponsored_with_tx)
    }

    /// Record the gas sponsorship rate limit & metrics for a given settled
    /// match
    pub async fn record_settled_match_sponsorship(
        &self,
        match_bundle: &AtomicMatchApiBundle,
        is_sponsored: bool,
        key: String,
        request_id: String,
    ) -> Result<(), AuthServerError> {
        if is_sponsored
            && let Some((gas_cost, tx_hash)) =
                self.get_sponsorship_amount_and_tx(&match_bundle.settlement_tx).await?
        {
            // Convert wei to ether using format_ether, then parse to f64
            let gas_cost_eth: f64 =
                format_ether(gas_cost).parse().map_err(AuthServerError::custom)?;

            let eth_price: f64 = self
                .price_reporter_client
                .get_eth_price()
                .await
                .map_err(AuthServerError::custom)?;

            let gas_sponsorship_value = eth_price * gas_cost_eth;

            self.rate_limiter.record_gas_sponsorship(key, gas_sponsorship_value).await;

            record_gas_sponsorship_metrics(gas_sponsorship_value, tx_hash, request_id);
        }

        Ok(())
    }

    /// Get the nonce from `sponsorAtomicMatchSettle` calldata
    fn get_nonce_from_sponsor_atomic_match_calldata(
        calldata: &[u8],
    ) -> Result<U256, AuthServerError> {
        let call = sponsorAtomicMatchSettleCall::abi_decode(
            calldata, true, // validate
        )
        .map_err(AuthServerError::gas_sponsorship)?;

        Ok(U256::from_big_endian(&call.nonce.to_be_bytes_vec()))
    }

    /// Get the nonce from `sponsorAtomicMatchSettleWithReceiver` calldata
    fn get_nonce_from_sponsor_atomic_match_with_receiver_calldata(
        calldata: &[u8],
    ) -> Result<U256, AuthServerError> {
        let call = sponsorAtomicMatchSettleWithReceiverCall::abi_decode(
            calldata, true, // validate
        )
        .map_err(AuthServerError::gas_sponsorship)?;

        Ok(U256::from_big_endian(&call.nonce.to_be_bytes_vec()))
    }
}

#[cfg(test)]
mod tests {
    use rand::{thread_rng, Rng};

    use super::*;

    #[test]
    fn test_conversion_rate_simple() {
        let eth_price = 1000.0;
        let token_price = 1.0;
        let token_decimals: u32 = 18;

        let expected_conversion_rate = AlloyU256::from(1000 * 10_u128.pow(18));
        let conversion_rate =
            Server::compute_conversion_rate(eth_price, token_price, token_decimals).unwrap();

        assert_eq!(conversion_rate, expected_conversion_rate);
    }

    #[test]
    fn test_conversion_rate_diff_decimals() {
        let eth_price = 2_500.0;
        let token_price = 100_000.0;
        let token_decimals: u32 = 8;

        let expected_conversion_rate = AlloyU256::from(2_500_000);
        let conversion_rate =
            Server::compute_conversion_rate(eth_price, token_price, token_decimals).unwrap();

        assert_eq!(conversion_rate, expected_conversion_rate);
    }

    #[test]
    fn test_conversion_rate_random() {
        let mut rng = thread_rng();
        let eth_price: f64 = rng.gen();
        let token_price: f64 = rng.gen();
        let token_decimals: u32 = rng.gen_range(1..=18);

        let conversion_rate =
            Server::compute_conversion_rate(eth_price, token_price, token_decimals).unwrap();

        // Simulate converting 1 ETH to TOKEN, and check that the resulting USD value is
        // the same as 1 ETH worth of USD.

        // This is the amount of nominal units of TOKEN for 1 whole ETH
        let nominal_token_per_eth: f64 = conversion_rate.into();

        // The token price is the amount of USD for 1 whole TOKEN, not for 1 nominal
        // unit. As such, this is effectively the derived amount of USD for 1
        // whole ETH, scaled by a factor of 10^`token_decimals`. We truncate
        // this result to compare to the original ETH price up to
        // `token_decimals` digits of precision.
        // Even this is not guaranteed to be exact, as the conversion from U256 -> f64
        // above has unspecified precision, so we scale down by one additional decimal
        // point.
        let usd_per_eth_scaled = (nominal_token_per_eth * token_price / 10.0).trunc();
        let eth_price_scaled = (eth_price * 10_f64.powi(token_decimals as i32 - 1)).trunc();

        assert_eq!(usd_per_eth_scaled, eth_price_scaled);
    }
}
