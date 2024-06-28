//! Database helpers for the server

use std::time::SystemTime;

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

// ----------
// | Models |
// ----------

/// A compliance entry for a wallet
#[derive(Debug, Clone, Queryable, Insertable)]
#[table_name = "wallet_compliance"]
#[allow(missing_docs)]
pub struct ComplianceEntry {
    pub address: String,
    pub is_compliant: bool,
    pub reason: String,
    pub created_at: SystemTime,
    pub expires_at: SystemTime,
}

impl ComplianceEntry {
    /// Get the compliance status for an entry
    pub fn compliance_status(&self) -> ComplianceStatus {
        if self.is_compliant {
            ComplianceStatus::Compliant
        } else {
            ComplianceStatus::NotCompliant { reason: self.reason.clone() }
        }
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

    Ok(query.first().cloned())
}
