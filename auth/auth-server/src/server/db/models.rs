//! DB model types for the auth server
#![allow(missing_docs, clippy::missing_docs_in_private_items)]
#![allow(trivial_bounds)]

use std::time::SystemTime;

use auth_server_api::key_management::ApiKey as UserFacingApiKey;
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
    pub rate_limit_whitelisted: bool,
}

impl From<ApiKey> for UserFacingApiKey {
    fn from(key: ApiKey) -> Self {
        let created_at = key.created_at.duration_since(SystemTime::UNIX_EPOCH).unwrap();
        Self {
            id: key.id,
            description: key.description,
            is_active: key.is_active,
            rate_limit_whitelisted: key.rate_limit_whitelisted,
            created_at: created_at.as_secs(),
        }
    }
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
            rate_limit_whitelisted: false,
        }
    }
}
