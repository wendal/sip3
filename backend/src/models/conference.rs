use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Serialize, Deserialize, FromRow, Clone)]
pub struct ConferenceRoom {
    pub id: u64,
    pub extension: String,
    pub domain: String,
    pub name: String,
    pub enabled: i8,
    pub max_participants: u32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Deserialize)]
pub struct CreateConferenceRoom {
    pub extension: String,
    pub name: String,
    pub domain: Option<String>,
    pub max_participants: Option<u32>,
    pub enabled: Option<i8>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateConferenceRoom {
    pub name: Option<String>,
    pub domain: Option<String>,
    pub max_participants: Option<u32>,
    pub enabled: Option<i8>,
}

#[derive(Debug, Serialize, FromRow, Clone)]
pub struct ConferenceParticipant {
    pub id: u64,
    pub room_id: u64,
    pub call_id: String,
    pub account: String,
    pub source_ip: String,
    pub source_port: u16,
    pub rtp_ip: Option<String>,
    pub rtp_port: Option<u16>,
    pub relay_port: u16,
    pub codec: String,
    pub muted: i8,
    pub joined_at: NaiveDateTime,
    pub left_at: Option<NaiveDateTime>,
}

pub const CONFERENCE_EXTENSION_RULE_MESSAGE: &str = "Conference extension must be exactly 9 digits";

/// Conference room extensions are restricted to 9 ASCII digits.
/// SIP user accounts are 3-6 digits, so this range cannot collide.
pub fn validate_conference_extension(extension: &str) -> Result<(), &'static str> {
    if extension.len() == 9 && extension.chars().all(|c| c.is_ascii_digit()) {
        Ok(())
    } else {
        Err(CONFERENCE_EXTENSION_RULE_MESSAGE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_nine_digit_extension() {
        assert!(validate_conference_extension("900000000").is_ok());
        assert!(validate_conference_extension("123456789").is_ok());
    }

    #[test]
    fn rejects_wrong_length_or_non_digits() {
        assert!(validate_conference_extension("").is_err());
        assert!(validate_conference_extension("12345678").is_err());
        assert!(validate_conference_extension("1234567890").is_err());
        assert!(validate_conference_extension("90000000a").is_err());
        assert!(validate_conference_extension("900 000 00").is_err());
    }
}
