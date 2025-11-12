// @generated automatically by Diesel CLI.

diesel::table! {
    balances (recovery_stream_seed) {
        recovery_stream_seed -> Numeric,
        version -> Numeric,

        share_stream_seed -> Numeric,
        share_stream_index -> Numeric,
        nullifier -> Numeric,
        public_shares -> Array<Numeric>,
        mint -> Text,
        owner_address -> Text,
        relayer_fee_recipient -> Text,
        one_time_authority -> Text,
        protocol_fee -> Numeric,
        relayer_fee -> Numeric,
        amount -> Numeric,
        account_id -> Uuid,
        active -> Bool,
    }
}

diesel::table! {
    expected_state_objects (recovery_id) {
        recovery_id -> Numeric,
        account_id -> Uuid,
        owner_address -> Text,
        recovery_stream_seed -> Numeric,
        share_stream_seed -> Numeric,
    }
}

diesel::table! {
    intents (recovery_stream_seed) {
        recovery_stream_seed -> Numeric,
        version -> Numeric,
        share_stream_seed -> Numeric,
        share_stream_index -> Numeric,
        nullifier -> Numeric,
        public_shares -> Array<Numeric>,
        input_mint -> Text,
        output_mint -> Text,
        owner_address -> Text,
        min_price -> Numeric,
        input_amount -> Numeric,
        account_id -> Uuid,
        active -> Bool,
        matching_pool -> Text,
        allow_external_matches -> Bool,
        min_fill_size -> Numeric,
        precompute_cancellation_proof -> Bool,
    }
}

diesel::table! {
    master_view_seeds (account_id) {
        account_id -> Uuid,
        owner_address -> Text,
        seed -> Numeric,
        recovery_seed_csprng_index -> Numeric,
        share_seed_csprng_index -> Numeric,
    }
}

diesel::table! {
    processed_nullifiers (nullifier) {
        nullifier -> Numeric,
        block_number -> Numeric,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    balances,
    expected_state_objects,
    intents,
    master_view_seeds,
    processed_nullifiers,
);
