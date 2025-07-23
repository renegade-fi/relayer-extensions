// @generated automatically by Diesel CLI.

diesel::table! {
    api_keys (id) {
        id -> Uuid,
        encrypted_key -> Varchar,
        description -> Varchar,
        created_at -> Timestamp,
        is_active -> Bool,
        rate_limit_whitelisted -> Bool,
    }
}

diesel::table! {
    asset_default_fees (asset) {
        asset -> Varchar,
        fee -> Float4,
    }
}

diesel::table! {
    user_fees (id, asset) {
        id -> Uuid,
        asset -> Varchar,
        fee -> Float4,
    }
}

diesel::joinable!(user_fees -> api_keys (id));

diesel::allow_tables_to_appear_in_same_query!(api_keys, asset_default_fees, user_fees,);
