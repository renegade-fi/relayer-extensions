// @generated automatically by Diesel CLI.

pub mod sql_types {
    #[derive(diesel::query_builder::QueryId, Clone, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "object_type"))]
    pub struct ObjectType;
}

diesel::table! {
    balances (identifier_seed) {
        identifier_seed -> Numeric,
        active -> Bool,
        mint -> Text,
        owner_address -> Text,
        one_time_key -> Text,
        protocol_fee -> Numeric,
        relayer_fee -> Numeric,
        amount -> Numeric,
        allow_public_fills -> Bool,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::ObjectType;

    generic_state_objects (identifier_seed) {
        identifier_seed -> Numeric,
        active -> Bool,
        object_type -> ObjectType,
        nullifier -> Numeric,
        version -> Numeric,
        encryption_seed -> Numeric,
        owner_address -> Text,
        public_shares -> Array<Numeric>,
        private_shares -> Array<Numeric>,
    }
}

diesel::table! {
    intents (identifier_seed) {
        identifier_seed -> Numeric,
        active -> Bool,
        input_mint -> Text,
        output_mint -> Text,
        owner_address -> Text,
        min_price -> Numeric,
        input_amount -> Numeric,
        matching_pool -> Text,
        allow_external_matches -> Bool,
        min_fill_size -> Numeric,
        precompute_cancellation_proof -> Bool,
    }
}

diesel::table! {
    master_view_seeds (owner_address) {
        owner_address -> Text,
        seed -> Numeric,
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
    generic_state_objects,
    intents,
    master_view_seeds,
    processed_nullifiers,
);
