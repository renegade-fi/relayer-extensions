//! Database helpers for the server

use std::time::{Duration, SystemTime};

use compliance_api::ComplianceStatus;
use diesel::{ExpressionMethods, Insertable, PgConnection, QueryDsl, Queryable, RunQueryDsl};
use renegade_util::err_str;

use crate::{
    error::ComplianceServerError,
    schema::{
        wallet_compliance,
        wallet_compliance::dsl::{address as address_col, wallet_compliance as compliance_table},
    },
};

/// The default expiration duration for a compliance entry
const DEFAULT_EXPIRATION_DURATION: Duration = Duration::from_days(7);

// ----------
// | Models |
// ----------

/// A compliance entry for a wallet
#[derive(Debug, Clone, Queryable, Insertable)]
#[diesel(table_name = wallet_compliance)]
#[allow(missing_docs)]
pub struct ComplianceEntry {
    pub address: String,
    pub is_compliant: bool,
    pub risk_level: String,
    pub reason: String,
    pub created_at: SystemTime,
    pub expires_at: SystemTime,
}

impl ComplianceEntry {
    /// Create a new entry from a risk assessment
    pub fn new(address: String, is_compliant: bool, risk_level: String, reason: String) -> Self {
        let created_at = SystemTime::now();
        let expires_at = created_at + DEFAULT_EXPIRATION_DURATION;
        ComplianceEntry { address, is_compliant, risk_level, reason, created_at, expires_at }
    }

    /// Get the compliance status for an entry
    pub fn compliance_status(&self) -> ComplianceStatus {
        if self.is_compliant {
            ComplianceStatus::Compliant
        } else {
            ComplianceStatus::NotCompliant { reason: self.reason.clone() }
        }
    }

    /// Check if the entry is expired
    pub fn is_expired(&self) -> bool {
        self.expires_at < SystemTime::now()
    }
}

// -----------
// | Queries |
// -----------

/// Get a compliance entry by address
pub fn get_compliance_entry(
    address: &str,
    conn: &mut PgConnection,
) -> Result<Option<ComplianceEntry>, ComplianceServerError> {
    let query = compliance_table
        .filter(address_col.eq(address))
        .load::<ComplianceEntry>(conn)
        .map_err(err_str!(ComplianceServerError::Db))?;

    // Return the first entry if it is not expired
    match query.first().cloned() {
        Some(entry) => {
            if entry.is_expired() {
                tracing::info!("Compliance entry expired for {address}, re-screening");
                Ok(None)
            } else {
                Ok(Some(entry))
            }
        },
        None => Ok(None),
    }
}

/// Upsert a compliance entry into the database
pub fn upsert_compliance_entry(
    entry: &ComplianceEntry,
    conn: &mut PgConnection,
) -> Result<(), ComplianceServerError> {
    diesel::insert_into(compliance_table)
        .values(entry)
        .on_conflict(address_col)
        .do_update()
        .set((
            wallet_compliance::is_compliant.eq(&entry.is_compliant),
            wallet_compliance::risk_level.eq(&entry.risk_level),
            wallet_compliance::reason.eq(&entry.reason),
            wallet_compliance::created_at.eq(&entry.created_at),
            wallet_compliance::expires_at.eq(&entry.expires_at),
        ))
        .execute(conn)
        .map_err(err_str!(ComplianceServerError::Db))?;

    Ok(())
}
