//! DB model types for the auth server
#![allow(missing_docs, clippy::missing_docs_in_private_items)]
#![allow(trivial_bounds)]

use std::time::SystemTime;

use crate::schema::api_keys;
use diesel::prelude::*;
use uuid::Uuid;

#[derive(Queryable, Selectable, Clone)]
#[diesel(table_name = api_keys)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ApiKey {
    pub id: Uuid,
    pub encrypted_key: String,
    pub description: String,
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
