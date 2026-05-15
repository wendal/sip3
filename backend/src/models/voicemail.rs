use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

pub const VOICEMAIL_STATUS_NEW: &str = "new";
pub const VOICEMAIL_STATUS_SAVED: &str = "saved";
pub const VOICEMAIL_STATUS_DELETED: &str = "deleted";
pub const VOICEMAIL_MAX_MESSAGE_SECS: u32 = 1250;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct VoicemailBox {
    pub id: u64,
    pub username: String,
    pub domain: String,
    pub enabled: i8,
    pub no_answer_secs: u32,
    pub max_message_secs: u32,
    pub max_messages: u32,
    pub greeting_storage_key: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Summary of a voicemail box with aggregate message counts.
/// Used for queries that JOIN and COUNT messages grouped by status,
/// not plain `SELECT *` from sip_voicemail_boxes.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct VoicemailBoxSummary {
    pub id: u64,
    pub username: String,
    pub domain: String,
    pub enabled: i8,
    pub no_answer_secs: u32,
    pub max_message_secs: u32,
    pub max_messages: u32,
    pub new_count: i64,
    pub saved_count: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct VoicemailMessage {
    pub id: u64,
    pub box_id: u64,
    pub caller: String,
    pub callee: String,
    pub call_id: String,
    pub duration_secs: u32,
    pub storage_key: String,
    pub content_type: String,
    pub status: String,
    pub created_at: NaiveDateTime,
    pub heard_at: Option<NaiveDateTime>,
}

#[derive(Debug, Deserialize)]
pub struct CreateVoicemailBox {
    pub username: String,
    pub domain: Option<String>,
    pub enabled: Option<i8>,
    pub no_answer_secs: Option<u32>,
    pub max_message_secs: Option<u32>,
    pub max_messages: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateVoicemailBox {
    pub enabled: Option<i8>,
    pub no_answer_secs: Option<u32>,
    pub max_message_secs: Option<u32>,
    pub max_messages: Option<u32>,
    pub greeting_storage_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateVoicemailMessage {
    pub status: String,
}

pub fn validate_voicemail_status(status: &str) -> Result<(), &'static str> {
    match status {
        VOICEMAIL_STATUS_NEW | VOICEMAIL_STATUS_SAVED | VOICEMAIL_STATUS_DELETED => Ok(()),
        _ => Err("voicemail status must be one of: new, saved, deleted"),
    }
}

pub fn validate_enabled_flag(enabled: Option<i8>) -> Result<(), &'static str> {
    match enabled {
        Some(0 | 1) | None => Ok(()),
        _ => Err("enabled must be 0 or 1"),
    }
}

pub fn validate_box_limits(
    no_answer_secs: u32,
    max_message_secs: u32,
    max_messages: u32,
) -> Result<(), &'static str> {
    if !(1..=600).contains(&no_answer_secs) {
        return Err("no_answer_secs must be between 1 and 600");
    }
    if !(1..=VOICEMAIL_MAX_MESSAGE_SECS).contains(&max_message_secs) {
        return Err("max_message_secs must be between 1 and 1250");
    }
    if !(1..=10_000).contains(&max_messages) {
        return Err("max_messages must be between 1 and 10000");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_message_statuses() {
        assert!(validate_voicemail_status("new").is_ok());
        assert!(validate_voicemail_status("saved").is_ok());
        assert!(validate_voicemail_status("deleted").is_ok());
        assert!(validate_voicemail_status("heard").is_err());
    }

    #[test]
    fn validates_box_limits() {
        assert!(validate_box_limits(25, 120, 100).is_ok());
        assert!(validate_box_limits(1, 1, 1).is_ok());
        assert!(validate_box_limits(600, 1250, 10_000).is_ok());
        assert!(validate_box_limits(25, 1251, 100).is_err());
        assert!(validate_box_limits(0, 120, 100).is_err());
        assert!(validate_box_limits(25, 0, 100).is_err());
        assert!(validate_box_limits(25, 120, 0).is_err());
        assert!(validate_box_limits(601, 120, 100).is_err());
    }

    #[test]
    fn validates_enabled_flag_values() {
        assert!(validate_enabled_flag(Some(1)).is_ok());
        assert!(validate_enabled_flag(Some(0)).is_ok());
        assert!(validate_enabled_flag(None).is_ok());
        assert!(validate_enabled_flag(Some(2)).is_err());
        assert!(validate_enabled_flag(Some(-1)).is_err());
    }
}
