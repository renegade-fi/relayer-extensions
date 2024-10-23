//! DB model types for the auth server
#![allow(missing_docs, clippy::missing_docs_in_private_items)]

use crate::schema::api_keys;
use diesel::prelude::*;
use diesel::sql_types::Timestamp;
use uuid::Uuid;

#[derive(Queryable)]
pub struct ApiKey {
    pub id: Uuid,
    pub encrypted_key: String,
    pub description: String,
    pub created_at: Timestamp,
    pub is_active: bool,
}

#[derive(Insertable)]
#[diesel(table_name = api_keys)]
pub struct NewApiKey {
    pub id: Uuid,
    pub encrypted_key: String,
    pub description: String,
}

impl NewApiKey {
    /// Create a new API key
    pub fn new(id: Uuid, encrypted_key: String, description: String) -> Self {
        Self { id, encrypted_key, description }
    }
}
