use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct SecurityEvent {
    pub id: u64,
    pub surface: String,
    pub event_type: String,
    pub source_ip: String,
    pub username: Option<String>,
    pub detail: String,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct AutoBlockEntry {
    pub id: u32,
    pub cidr: String,
    pub description: Option<String>,
    pub priority: i32,
    pub enabled: i8,
    pub created_at: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct UnblockRequest {
    pub cidr: String,
}
