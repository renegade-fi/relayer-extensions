// @generated automatically by Diesel CLI.

pub mod sql_types {
    #[derive(diesel::query_builder::QueryId, Clone, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "rate_limit_method"))]
    pub struct RateLimitMethod;
}

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
    use diesel::sql_types::*;
    use super::sql_types::RateLimitMethod;

    rate_limits (api_key_id, method) {
        api_key_id -> Uuid,
        method -> RateLimitMethod,
        requests_per_minute -> Int4,
    }
}

diesel::table! {
    user_fees (id, asset) {
        id -> Uuid,
        asset -> Varchar,
        fee -> Float4,
    }
}

diesel::joinable!(rate_limits -> api_keys (api_key_id));
diesel::joinable!(user_fees -> api_keys (id));

diesel::allow_tables_to_appear_in_same_query!(api_keys, asset_default_fees, rate_limits, user_fees,);
