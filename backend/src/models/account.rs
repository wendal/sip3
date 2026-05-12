use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct Account {
    pub id: u64,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    #[serde(skip_serializing)]
    pub ha1_hash: Option<String>,
    pub display_name: Option<String>,
    pub domain: String,
    pub enabled: i8,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateAccount {
    pub username: String,
    pub password: String,
    pub display_name: Option<String>,
    pub domain: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateAccount {
    pub password: Option<String>,
    pub display_name: Option<String>,
    pub domain: Option<String>,
    pub enabled: Option<i8>,
}

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct Registration {
    pub id: u64,
    pub username: String,
    pub domain: String,
    pub contact_uri: String,
    pub user_agent: Option<String>,
    pub expires_at: NaiveDateTime,
    pub registered_at: NaiveDateTime,
    pub source_ip: String,
    pub source_port: u16,
}

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct Call {
    pub id: u64,
    pub call_id: String,
    pub caller: String,
    pub callee: String,
    pub status: String,
    pub started_at: NaiveDateTime,
    pub answered_at: Option<NaiveDateTime>,
    pub ended_at: Option<NaiveDateTime>,
}
