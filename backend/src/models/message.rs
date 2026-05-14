use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct SipMessageRecord {
    pub id: u64,
    pub message_id: Option<String>,
    pub call_id: Option<String>,
    pub sender: String,
    pub receiver: String,
    pub content_type: String,
    pub body: String,
    pub status: String,
    pub source_ip: String,
    pub created_at: NaiveDateTime,
    pub delivered_at: Option<NaiveDateTime>,
}
