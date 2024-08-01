// @generated automatically by Diesel CLI.

diesel::table! {
    fees (id) {
        id -> Int4,
        tx_hash -> Text,
        mint -> Text,
        amount -> Numeric,
        blinder -> Numeric,
        receiver -> Text,
        redeemed -> Bool,
    }
}

diesel::table! {
    gas_wallets (id) {
        id -> Uuid,
        address -> Text,
        peer_id -> Nullable<Text>,
        active -> Bool,
        created_at -> Timestamp,
    }
}

diesel::table! {
    hot_wallets (id) {
        id -> Uuid,
        secret_id -> Text,
        vault -> Text,
        address -> Text,
        internal_wallet_id -> Uuid,
    }
}

diesel::table! {
    indexing_metadata (key) {
        key -> Text,
        value -> Text,
    }
}

diesel::table! {
    renegade_wallets (id) {
        id -> Uuid,
        mints -> Array<Nullable<Text>>,
        secret_id -> Text,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    fees,
    gas_wallets,
    hot_wallets,
    indexing_metadata,
    renegade_wallets,
);
