use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct AclEntry {
    pub id: u32,
    pub action: String,
    pub cidr: String,
    pub description: Option<String>,
    pub priority: i32,
    pub enabled: i8,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateAclEntry {
    pub action: String,
    pub cidr: String,
    pub description: Option<String>,
    /// Rule priority — lower value is matched first (default: 100).
    pub priority: Option<i32>,
    pub enabled: Option<i8>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateAclEntry {
    pub action: Option<String>,
    pub cidr: Option<String>,
    pub description: Option<String>,
    pub priority: Option<i32>,
    pub enabled: Option<i8>,
}
