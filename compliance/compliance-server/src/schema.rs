// @generated automatically by Diesel CLI.

diesel::table! {
    wallet_compliance (address) {
        address -> Text,
        is_compliant -> Bool,
        risk_level -> Text,
        reason -> Text,
        created_at -> Timestamp,
        expires_at -> Timestamp,
    }
}
