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
    indexing_metadata (key) {
        key -> Text,
        value -> Text,
    }
}

diesel::table! {
    wallets (id) {
        id -> Uuid,
        mints -> Nullable<Array<Nullable<Text>>>,
        secret_id -> Nullable<Text>,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    fees,
    indexing_metadata,
    wallets,
);
