// @generated automatically by Diesel CLI.

diesel::table! {
    balances (recovery_stream_seed) {
        recovery_stream_seed -> Numeric,
        version -> BigInt,
        share_stream_seed -> Numeric,
        share_stream_index -> BigInt,
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
        recovery_stream_seed -> Numeric,
        share_stream_seed -> Numeric,
    }
}

diesel::table! {
    intents (recovery_stream_seed) {
        recovery_stream_seed -> Numeric,
        version -> BigInt,
        share_stream_seed -> Numeric,
        share_stream_index -> BigInt,
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
    public_intents (intent_hash) {
        intent_hash -> Text,
        order_id -> Uuid,
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
        intent_signature_nonce -> Numeric,
        intent_signature_bytes -> Text,
        permit -> Text,
    }
}

diesel::table! {
    master_view_seeds (account_id) {
        account_id -> Uuid,
        owner_address -> Text,
        seed -> Numeric,
        recovery_seed_csprng_index -> BigInt,
        share_seed_csprng_index -> BigInt,
    }
}

diesel::table! {
    processed_nullifiers (nullifier) {
        nullifier -> Numeric,
    }
}

diesel::table! {
    processed_recovery_ids (recovery_id) {
        recovery_id -> Numeric,
    }
}

diesel::table! {
    processed_public_intent_updates (intent_hash, tx_hash) {
        intent_hash -> Text,
        tx_hash -> Text,
    }
}

diesel::table! {
    last_indexed_nullifier_block (id) {
        id -> Integer,
        block_number -> BigInt,
    }
}

diesel::table! {
    last_indexed_recovery_id_block (id) {
        id -> Integer,
        block_number -> BigInt,
    }
}

diesel::table! {
    last_indexed_public_intent_update_block (id) {
        id -> Integer,
        block_number -> BigInt,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    balances,
    expected_state_objects,
    intents,
    public_intents,
    master_view_seeds,
    processed_nullifiers,
    processed_recovery_ids,
    processed_public_intent_updates,
    last_indexed_nullifier_block,
    last_indexed_recovery_id_block,
    last_indexed_public_intent_update_block,
);
