//! Local SIP conference endpoint.
//!
//! Conferences are a B2BUA-style local endpoint. When [`SipHandler`] sees an
//! INVITE whose request-URI matches an enabled conference room extension, it
//! routes the call here instead of the generic proxy. Responses are built
//! locally with an SDP answer pointing at a per-participant UDP RTP socket
//! managed by [`ConferenceMedia`]. Subsequent ACK/BYE/CANCEL/INFO that share
//! the same Call-ID are also routed here.

use anyhow::Result;
use sqlx::MySqlPool;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use super::conference_media::ConferenceMedia;
use super::conference_sdp::{ConferenceCodec, build_answer, negotiate_offer};
use super::handler::{SipMessage, base_response, extract_uri, uri_username};
use super::proxy::CALLER_ACCOUNT_EXISTS_SQL;
use crate::config::Config;
use crate::models::conference::validate_conference_extension;

#[derive(Clone)]
#[allow(dead_code)]
struct ConferenceCall {
    room_id: u64,
    extension: String,
    domain: String,
    caller_account: String,
    to_tag: String,
    participant_db_id: Option<u64>,
}

#[derive(Clone)]
pub struct Conference {
    pool: MySqlPool,
    cfg: Config,
    media: ConferenceMedia,
    active: Arc<Mutex<HashMap<String, ConferenceCall>>>,
}

impl Conference {
    pub fn new(pool: MySqlPool, cfg: Config, media: ConferenceMedia) -> Self {
        Self {
            pool,
            cfg,
            media,
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn media(&self) -> &ConferenceMedia {
        &self.media
    }

    pub async fn is_conference_call(&self, call_id: &str) -> bool {
        self.active.lock().await.contains_key(call_id)
    }

    /// Look up an enabled conference room by extension+domain. Returns the
    /// row id and `max_participants` when found.
    pub async fn lookup_room(&self, extension: &str, domain: &str) -> Option<(u64, u32)> {
        if validate_conference_extension(extension).is_err() {
            return None;
        }
        let row: Option<(u64, u32)> = sqlx::query_as(
            "SELECT id, max_participants FROM sip_conference_rooms \
             WHERE extension = ? AND domain = ? AND enabled = 1",
        )
        .bind(extension)
        .bind(domain)
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten();
        row
    }

    pub async fn handle_invite(
        &self,
        msg: &SipMessage,
        src: SocketAddr,
        room_id: u64,
        max_participants: u32,
    ) -> Result<String> {
        let request_uri = msg.request_uri.as_deref().unwrap_or("");
        let extension = uri_username(request_uri).unwrap_or_default();
        let caller = msg
            .from_header()
            .and_then(extract_uri)
            .and_then(|u| uri_username(&u))
            .unwrap_or_else(|| "unknown".to_string());
        let call_id = msg.call_id().unwrap_or("").to_string();
        let domain = self.cfg.server.sip_domain.clone();

        if call_id.is_empty() {
            return Ok(base_response(msg, 400, "Bad Request").build());
        }

        // Re-INVITE on an existing dialog is treated as an idempotent confirmation.
        if self.is_conference_call(&call_id).await {
            debug!("Re-INVITE for active conference call {}", call_id);
        }

        // Caller must be a known SIP account in our domain (matches proxy behaviour).
        if caller == "unknown" {
            return Ok(base_response(msg, 403, "Forbidden").build());
        }
        let caller_ok: Option<(i32,)> = sqlx::query_as(CALLER_ACCOUNT_EXISTS_SQL)
            .bind(&caller)
            .bind(&domain)
            .fetch_optional(&self.pool)
            .await?;
        if caller_ok.is_none() {
            warn!(
                "Conference INVITE from unknown caller {}@{} for room {}",
                caller, domain, extension
            );
            return Ok(base_response(msg, 403, "Forbidden").build());
        }

        let active_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sip_conference_participants \
             WHERE room_id = ? AND left_at IS NULL",
        )
        .bind(room_id)
        .fetch_one(&self.pool)
        .await?;
        if active_count.0 as u64 >= max_participants as u64 {
            warn!("Conference room {} is full ({})", room_id, max_participants);
            return Ok(base_response(msg, 486, "Busy Here").build());
        }

        let negotiation = match negotiate_offer(&msg.body) {
            Ok(n) => n,
            Err(e) => {
                warn!(
                    "Conference {} rejecting incompatible SDP from {}: {}",
                    extension, caller, e
                );
                return Ok(base_response(msg, 488, "Not Acceptable Here").build());
            }
        };

        let joined = match self
            .media
            .join(
                room_id,
                call_id.clone(),
                negotiation.codec,
                negotiation.audio_pt,
                negotiation.telephone_event_pt,
            )
            .await
        {
            Ok(j) => j,
            Err(e) => {
                warn!("Conference media allocation failed: {}", e);
                return Ok(base_response(msg, 500, "Internal Server Error").build());
            }
        };

        let session_id = epoch_id();
        let answer = build_answer(
            self.media.public_ip(),
            joined.relay_port,
            &negotiation,
            session_id,
        );

        let codec_label = match negotiation.codec {
            ConferenceCodec::Pcmu => "PCMU",
            ConferenceCodec::Pcma => "PCMA",
        };

        let participant_db_id: Option<u64> = match sqlx::query(
            "INSERT INTO sip_conference_participants
                (room_id, call_id, account, source_ip, source_port, relay_port, codec, muted, joined_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, 0, NOW())
             ON DUPLICATE KEY UPDATE
                account = VALUES(account),
                source_ip = VALUES(source_ip),
                source_port = VALUES(source_port),
                relay_port = VALUES(relay_port),
                codec = VALUES(codec),
                muted = 0,
                left_at = NULL,
                joined_at = NOW()",
        )
        .bind(room_id)
        .bind(&call_id)
        .bind(&caller)
        .bind(src.ip().to_string())
        .bind(src.port())
        .bind(joined.relay_port)
        .bind(codec_label)
        .execute(&self.pool)
        .await
        {
            Ok(r) if r.last_insert_id() > 0 => Some(r.last_insert_id()),
            Ok(_) => None,
            Err(e) => {
                warn!("Failed to record conference participant {}: {}", call_id, e);
                None
            }
        };

        let to_tag = format!("conf-{:x}", session_id);
        let to_with_tag = with_to_tag(msg.to_header().unwrap_or(""), &to_tag);
        let contact = format!(
            "<sip:{}@{}:{}>",
            extension, self.cfg.server.public_ip, self.cfg.server.sip_port
        );

        self.active.lock().await.insert(
            call_id.clone(),
            ConferenceCall {
                room_id,
                extension: extension.clone(),
                domain: domain.clone(),
                caller_account: caller.clone(),
                to_tag: to_tag.clone(),
                participant_db_id,
            },
        );

        info!(
            "Conference {} joined: {} (call_id={}, relay_port={}, codec={})",
            extension, caller, call_id, joined.relay_port, codec_label
        );

        let response = base_response_with_to(msg, 200, "OK", &to_with_tag)
            .header("Contact", &contact)
            .header("Content-Type", "application/sdp")
            .header("Allow", "INVITE, ACK, CANCEL, BYE, INFO")
            .body(&answer)
            .build();
        Ok(response)
    }

    pub async fn handle_ack(&self, msg: &SipMessage) {
        let call_id = msg.call_id().unwrap_or("");
        debug!("Conference ACK received for {}", call_id);
    }

    pub async fn handle_bye(&self, msg: &SipMessage) -> Result<String> {
        let call_id = msg.call_id().unwrap_or("").to_string();
        let removed = self.active.lock().await.remove(&call_id);
        if let Some(call) = removed {
            self.media.leave(call.room_id, &call_id).await;
            let _ = sqlx::query(
                "UPDATE sip_conference_participants SET left_at = NOW() \
                 WHERE call_id = ? AND left_at IS NULL",
            )
            .bind(&call_id)
            .execute(&self.pool)
            .await;
            info!(
                "Conference {} left: {} (call_id={})",
                call.extension, call.caller_account, call_id
            );
        }
        Ok(base_response(msg, 200, "OK").build())
    }

    pub async fn handle_cancel(&self, msg: &SipMessage) -> Result<String> {
        // Symmetric with BYE for our purposes — treat as participant departure.
        self.handle_bye(msg).await
    }

    pub async fn handle_info(&self, msg: &SipMessage) -> Result<String> {
        let call_id = msg.call_id().unwrap_or("").to_string();
        let ctype = msg.header("content-type").unwrap_or("").to_lowercase();
        if !ctype.contains("application/dtmf-relay") && !ctype.contains("application/dtmf") {
            return Ok(base_response(msg, 200, "OK").build());
        }
        let active = self.active.lock().await;
        let Some(call) = active.get(&call_id).cloned() else {
            drop(active);
            return Ok(base_response(msg, 481, "Call/Transaction Does Not Exist").build());
        };
        drop(active);

        if let Some(toggled) =
            parse_dtmf_relay_for_star6(&msg.body, &self.media, call.room_id, &call_id).await
        {
            let now_muted = toggled;
            let _ =
                sqlx::query("UPDATE sip_conference_participants SET muted = ? WHERE call_id = ?")
                    .bind(if now_muted { 1i8 } else { 0i8 })
                    .bind(&call_id)
                    .execute(&self.pool)
                    .await;
            info!(
                "Conference {} {} mute toggled to {} via INFO",
                call.extension, call.caller_account, now_muted
            );
        }
        Ok(base_response(msg, 200, "OK").build())
    }

    /// Mark all in-memory conference participants as ended on shutdown.
    /// Used at startup to reconcile DB state with lost media sessions.
    pub async fn reconcile_active_on_startup(&self) -> Result<()> {
        let _ = sqlx::query(
            "UPDATE sip_conference_participants SET left_at = NOW() WHERE left_at IS NULL",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn epoch_id() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Append/replace `;tag=<tag>` on a `To` header value.
fn with_to_tag(to: &str, tag: &str) -> String {
    if to.is_empty() {
        return format!(";tag={}", tag);
    }
    // If a tag already exists, leave the header alone (don't double-tag).
    let lower = to.to_lowercase();
    if lower.contains(";tag=") {
        return to.to_string();
    }
    format!("{};tag={}", to, tag)
}

/// Variant of [`base_response`] that overrides the `To` header value (so we
/// can attach our generated server tag).
fn base_response_with_to(
    req: &SipMessage,
    status_code: u16,
    reason: &str,
    to_value: &str,
) -> super::handler::SipResponseBuilder {
    let mut builder = super::handler::SipResponseBuilder::new(status_code, reason);
    for via in req.via_headers() {
        builder = builder.header("Via", via);
    }
    if let Some(from) = req.from_header() {
        builder = builder.header("From", from);
    }
    builder = builder.header("To", to_value);
    if let Some(call_id) = req.call_id() {
        builder = builder.header("Call-ID", call_id);
    }
    if let Some(cseq) = req.cseq() {
        builder = builder.header("CSeq", cseq);
    }
    builder.header("Server", "SIP3/0.1.0")
}

/// Parse a `Signal=<digit>` style DTMF-relay body. Linphone sends bodies like
/// `Signal=*\r\nDuration=160\r\n`. Toggles mute on `*` followed by `6` within
/// 3 seconds. Returns `Some(now_muted)` when a toggle occurred.
async fn parse_dtmf_relay_for_star6(
    body: &str,
    media: &ConferenceMedia,
    room_id: u64,
    call_id: &str,
) -> Option<bool> {
    let signal = body
        .lines()
        .find_map(|l| {
            let l = l.trim();
            l.strip_prefix("Signal=")
                .or_else(|| l.strip_prefix("signal="))
        })
        .map(|s| s.trim().to_string())?;

    // Map textual signal to RFC 4733 event code; `record_dtmf` does the rest.
    let event = match signal.chars().next()? {
        '*' => 10u8,
        '#' => 11u8,
        c if c.is_ascii_digit() => c.to_digit(10).unwrap() as u8,
        _ => return None,
    };
    media.record_dtmf_for(room_id, call_id, event).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_to_tag_appends_when_missing() {
        let to = "<sip:900000000@sip.air32.cn>";
        assert_eq!(
            with_to_tag(to, "abc123"),
            "<sip:900000000@sip.air32.cn>;tag=abc123"
        );
    }

    #[test]
    fn with_to_tag_preserves_existing() {
        let to = "<sip:900000000@sip.air32.cn>;tag=existing";
        assert_eq!(with_to_tag(to, "abc123"), to);
    }
}
