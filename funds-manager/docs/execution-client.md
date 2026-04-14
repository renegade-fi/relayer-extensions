# Execution Client

This document explains the swap APIs exposed by funds-manager and the internal execution flow that services those APIs.

It is scoped to:

- `funds-manager/funds-manager-api/src/types/quoters.rs`
- `funds-manager/funds-manager-server/src/handlers/swap.rs`
- `funds-manager/funds-manager-server/src/execution_client/`

## Overview

Funds-manager owns two quoter-facing swap APIs:

- immediate swap: execute one swap now from the quoter hot wallet
- swap into target token: acquire enough of a token to reach a target balance, potentially by swapping out of multiple source assets

Both routes execute from the quoter hot wallet, not from an arbitrary user wallet. The execution client is built per chain in [`funds-manager-server/src/cli.rs`](../funds-manager-server/src/cli.rs) using:

- the chain-specific RPC provider
- the quoter hot wallet private key from custody
- the price reporter client
- optional LiFi and Bebop API keys
- `max_price_deviations` from chain config

## Public API Surface

The route constants and request/response types live in [`funds-manager-api/src/types/quoters.rs`](../funds-manager-api/src/types/quoters.rs).

### Routes

These routes are mounted under:

- `POST /custody/{chain}/quoters/swap-immediate`
- `POST /custody/{chain}/quoters/swap-into-target-token`

Both routes are HMAC-authenticated in `funds-manager-server/src/main.rs` via `with_hmac_auth(...)`.

### `QuoteParams`

`QuoteParams` is the common swap request shape:

- `from_token: String`
  The token to sell. Can be an address or symbol, though the server paths generally treat it as an on-chain token address.
- `to_token: String`
  The token to buy.
- `from_amount: U256`
  Sell amount in atomic units.
- `slippage_tolerance: Option<f64>`
  Optional slippage tolerance. If omitted, venue-specific defaults apply.
- `increase_price_deviation: bool`
  If `true`, the execution client widens the allowed price-deviation threshold before shrinking trade size.
- `venue: Option<SupportedExecutionVenue>`
  Optional venue override.

Supported venue enum values are:

- `Lifi`
- `Cowswap`
- `Bebop`
- `Okx`

Important caveats:

- `Okx` is no longer supported and is explicitly rejected.
- `Cowswap` still has an implementation, but it is excluded from the default fanout because of self-trade-prevention concerns.

### `swap-immediate`

Request body: `QuoteParams`

Response body: `SwapImmediateResponse`

- `quote: ApiExecutionQuote`
- `tx_hash: String`
- `execution_cost: f64`

This endpoint either executes one swap or returns an error / no-swap outcome if the execution client cannot find a viable path.

### `swap-into-target-token`

Request body: `SwapIntoTargetTokenRequest`

- `target_amount: f64`
  Desired final balance of the target token in whole units.
- `quote_params: QuoteParams`
  Common swap parameters. `from_token` and `from_amount` are ignored for candidate selection but must still be present in the request type.
- `exclude_tokens: Vec<String>`
  Tokens that the server must not sell while covering the target amount.

Response body: `Vec<SwapImmediateResponse>`

This endpoint may execute multiple swaps and returns one response per successful swap.

## Handler Behavior

The HTTP handlers live in [`funds-manager-server/src/handlers/swap.rs`](../funds-manager-server/src/handlers/swap.rs).

Before any swap:

1. funds-manager fetches the chain-specific `ExecutionClient`
2. it fetches the chain-specific `CustodyClient`
3. it tops up quoter hot-wallet gas via `custody_client.top_up_quoter_hot_wallet_gas()`
4. it executes the swap path
5. it records swap-cost metrics via the chain-specific `MetricsRecorder`

If cost recording fails, the swap still succeeds; the response just defaults `execution_cost` to `0.0`.

## Immediate Swap Logic

The main entrypoint is:

- `ExecutionClient::swap_immediate_decaying` in [`swap/swap_immediate.rs`](../funds-manager-server/src/execution_client/swap/swap_immediate.rs)

The control flow is:

1. Validate the swap can be attempted.
2. Fetch quotes across venues.
3. Select the best valid quote.
4. Reject quotes that deviate too far from the Renegade price.
5. Execute the quote.
6. If the execution fails, retry by excluding the failing quote source.
7. After too many retries with exclusions, reduce swap size and try again.

### Preconditions

`can_execute_swap` enforces:

- the hot wallet has enough `from_token` balance to cover `from_amount`
- the expected notional size is at least `MIN_SWAP_QUOTE_AMOUNT` (`10 USDC`)

The expected quote amount is computed from the Renegade price reporter. Stablecoins are treated as face value; non-stables use `price_reporter.get_price(...)`.

### Quote fanout and selection

If `params.venue` is set, only that venue is queried, except:

- `Okx` is immediately rejected

If no venue is set, `fetch_all_quotes` fans out to `AllExecutionVenues::get_all_venues()`, which currently returns:

- LiFi
- Bebop

CowSwap is intentionally disabled in default fanout.

Each venue may return multiple quotes:

- LiFi typically returns one route, whose `tool` is tracked as a `CrossVenueQuoteSource::LifiExchange(...)`
- Bebop may return both `BebopJAMv2` and `BebopPMMv3`
- CowSwap returns a single `CrossVenueQuoteSource::Cowswap`

Before ranking, quotes are filtered through `ExecutableQuote::is_valid()`, which currently rejects quotes whose calldata appears to contain the Renegade darkpool address. This is a self-trade guardrail.

Best-quote selection is simple:

- for sells, higher price is better
- for buys, lower price is better

`ExecutionQuote::is_sell()` is defined in Renegade terms: a quote is considered a sell when the bought token is USDC.

### Price-deviation gate

The best quote is compared against the Renegade price reporter in `exceeds_price_deviation`.

Deviation logic:

- sell path: `(renegade_price - quote_price) / renegade_price`
- buy path: `(quote_price - renegade_price) / renegade_price`

Default allowed deviation is:

- `1%` globally (`DEFAULT_MAX_PRICE_DEVIATION`)
- optionally overridden per base-token ticker by `max_price_deviations` in chain config

If `increase_price_deviation` is `true`, the client increases the allowed threshold in `20%` relative increments, up to `4x` the configured maximum, before giving up.

If `increase_price_deviation` is `false`, the client instead decays trade size immediately.

Price-deviation metrics are emitted regardless of pass/fail so operators can tune these thresholds later.

### Retry strategy

If a quote executes unsuccessfully:

- first retry path: exclude the failing quote source and refetch
- after `MAX_RETRIES_WITH_EXCLUSION` (`5`) retries, shrink `from_amount` by `SWAP_DECAY_FACTOR` (`2x`) and try again

When the client shrinks trade size, it also resets the excluded-source list because the quote landscape has changed materially.

The successful result is returned as `DecayingSwapOutcome`:

- executed `ExecutionQuote`
- `buy_amount_actual`
- `tx_hash`
- `cumulative_gas_cost` across all attempts

## Swap-Into-Target Logic

The main entrypoint is:

- `ExecutionClient::try_swap_into_target_token` in [`swap/swap_to_target.rs`](../funds-manager-server/src/execution_client/swap/swap_to_target.rs)

This path is balance-oriented rather than order-oriented.

### High-level flow

1. Parse the target token from `quote_params.to_token`.
2. Exclude:
   - the target token itself
   - all `exclude_tokens`
3. Check the current hot-wallet balance of the target token.
4. If the current balance already exceeds `target_amount`, do nothing.
5. Convert the deficit into USDC notional using the price reporter.
6. If the deficit is smaller than `MIN_SWAP_QUOTE_AMOUNT`, do nothing.
7. Gather swap candidates from the hot wallet.
8. Sell candidates one by one until the target deficit is covered or the candidate set is exhausted.

### Candidate selection

`get_swap_candidates` considers all known tokens and filters them with:

- token is on the same chain
- token is not excluded
- token is not the dummy `USD` ticker

Each candidate includes:

- token
- current hot-wallet balance
- current price

Candidates are sorted by descending notional value (`balance * price`), so the client spends the largest holdings first.

### Cover buffer

The client multiplies the target deficit by `SWAP_TO_COVER_BUFFER` (`1.1`) before starting swaps.

This buffer exists because candidate selection and pricing happen against sampled balances and prices that may drift before swaps complete.

### Per-candidate execution

For each candidate:

- if the candidate's total notional value is less than the remaining deficit, sell the full balance
- otherwise sell only enough to cover the remaining deficit
- if the resulting swap value is below `10 USDC`, skip it
- construct a new `QuoteParams`
- invoke `swap_immediate_decaying`

Errors while swapping one candidate do not abort the whole target-token flow. The client logs and continues to the next candidate.

The returned vector contains only successful swap outcomes.

## Multi-target Helper

There is an additional internal helper:

- `ExecutionClient::multi_swap_into_target_tokens`

This is used outside the quoter swap endpoints, for example by gas-refill paths.

Its strategy is:

1. Compute required purchase value for each target token.
2. First buy enough USDC to cover all purchases.
3. Then buy each target token out of USDC.

This preserves the existing telemetry semantics around “buy” vs “sell” and base-token accounting.

## Venue-Specific Behavior

### LiFi

Implementation:

- [`venues/lifi/mod.rs`](../funds-manager-server/src/execution_client/venues/lifi/mod.rs)

Behavior:

- quote request is built from `QuoteParams` plus defaults
- excluded LiFi route sources become `deny_exchanges`
- default slippage falls back to `DEFAULT_SLIPPAGE_TOLERANCE` (`10 bps`)
- approvals are made to the LiFi diamond
- transaction is sent onchain directly from the hot wallet
- reverted or failed-to-send transactions return `tx_hash: None`, which feeds back into the retry logic

### Bebop

Implementation:

- [`venues/bebop/mod.rs`](../funds-manager-server/src/execution_client/venues/bebop/mod.rs)

Behavior:

- one API response may yield two executable quote variants: `JAMv2` and `PMMv3`
- excluded quote sources are filtered before conversion
- approvals are made to the venue-provided `approval_target`
- quotes are requested with taker checks skipped, because funds-manager wants checks enforced at execution time, not quote time
- transactions are sent onchain directly from the hot wallet

### CowSwap

Implementation:

- [`venues/cowswap/mod.rs`](../funds-manager-server/src/execution_client/venues/cowswap/mod.rs)

Behavior:

- implemented, but not queried by default
- available only on chains the adapter supports
- execution is asynchronous from funds-manager's perspective: place order, then wait for trade execution
- gas cost is effectively zero on settlement because the solver settles the trade

The default fanout excludes CowSwap until there is a better self-trade-prevention mechanism.

## Operational Caveats

- All swaps execute from the quoter hot wallet, so custody and gas top-up are prerequisites.
- Price validation is based on Renegade's price reporter, not venue-native pricing.
- `swap-into-target-token` is best-effort across candidates; partial success is possible and expected.
- `increase_price_deviation` trades off execution certainty against price discipline.
- `Okx` is still present in the public enum for compatibility, but runtime behavior rejects it.

## Key Files

- API types: [`funds-manager-api/src/types/quoters.rs`](../funds-manager-api/src/types/quoters.rs)
- HTTP handlers: [`funds-manager-server/src/handlers/swap.rs`](../funds-manager-server/src/handlers/swap.rs)
- Execution client root: [`funds-manager-server/src/execution_client/mod.rs`](../funds-manager-server/src/execution_client/mod.rs)
- Immediate swaps: [`funds-manager-server/src/execution_client/swap/swap_immediate.rs`](../funds-manager-server/src/execution_client/swap/swap_immediate.rs)
- Target-token swaps: [`funds-manager-server/src/execution_client/swap/swap_to_target.rs`](../funds-manager-server/src/execution_client/swap/swap_to_target.rs)
- Venue abstraction: [`funds-manager-server/src/execution_client/venues/mod.rs`](../funds-manager-server/src/execution_client/venues/mod.rs)
- Quote model: [`funds-manager-server/src/execution_client/venues/quote.rs`](../funds-manager-server/src/execution_client/venues/quote.rs)
