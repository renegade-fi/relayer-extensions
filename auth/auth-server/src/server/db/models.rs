//! DB model types for the auth server
#![allow(missing_docs, clippy::missing_docs_in_private_items)]
#![allow(trivial_bounds)]

use std::time::SystemTime;

use diesel::prelude::*;
use uuid::Uuid;

use crate::server::db::schema::api_keys;

#[derive(Queryable, Selectable, Clone)]
#[diesel(table_name = api_keys)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ApiKey {
    pub id: Uuid,
    pub encrypted_key: String,
    pub description: String,
    #[allow(dead_code)]
    pub created_at: SystemTime,
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

impl From<NewApiKey> for ApiKey {
    fn from(key: NewApiKey) -> Self {
        Self {
            id: key.id,
            encrypted_key: key.encrypted_key,
            description: key.description,
            created_at: SystemTime::now(),
            is_active: true,
        }
    }
}
