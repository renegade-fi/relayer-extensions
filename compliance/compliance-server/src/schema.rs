// @generated automatically by Diesel CLI.

diesel::table! {
    wallet_compliance (address) {
        address -> Text,
        is_compliant -> Bool,
        reason -> Text,
        created_at -> Timestamp,
        expires_at -> Timestamp,
    }
}
