// @generated automatically by Diesel CLI.

diesel::table! {
    api_keys (id) {
        id -> Uuid,
        encrypted_key -> Varchar,
        username -> Varchar,
        created_at -> Timestamp,
        is_active -> Bool,
    }
}
