use anyhow::Result;
use chrono::Utc;
use sqlx::MySqlPool;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::{info, warn};

use super::handler::{SipMessage, base_response, extract_uri, uri_username};
use crate::config::Config;

#[derive(Clone)]
pub struct VoicemailMwi {
    pool: MySqlPool,
    cfg: Config,
    socket: Arc<UdpSocket>,
}

impl VoicemailMwi {
    pub fn new(pool: MySqlPool, cfg: Config, socket: Arc<UdpSocket>) -> Self {
        Self { pool, cfg, socket }
    }

    pub async fn handle_subscribe(&self, msg: &SipMessage, src: SocketAddr) -> Result<String> {
        let event = msg.header("event").unwrap_or("").to_lowercase();
        let event = event.split(';').next().unwrap_or("").trim();
        if event != "message-summary" {
            return Ok(base_response(msg, 489, "Bad Event").build());
        }

        let from = msg.from_header().unwrap_or("");
        let from_uri = extract_uri(from).unwrap_or_default();
        let subscriber = uri_username(&from_uri).unwrap_or_default();
        if subscriber.is_empty() {
            return Ok(base_response(msg, 400, "Bad Request").build());
        }

        let domain = self.cfg.server.sip_domain.clone();
        let call_id = msg.call_id().unwrap_or("").to_string();
        let subscriber_tag = extract_param(from, "tag").unwrap_or_default();
        let expires = msg.expires().unwrap_or(3600);

        if expires == 0 {
            sqlx::query(
                "DELETE FROM sip_voicemail_mwi_subscriptions WHERE subscriber = ? AND domain = ? AND call_id = ?",
            )
            .bind(&subscriber)
            .bind(&domain)
            .bind(&call_id)
            .execute(&self.pool)
            .await?;
            return Ok(base_response(msg, 200, "OK").build());
        }

        let expires_at = (Utc::now() + chrono::Duration::seconds(i64::from(expires))).naive_utc();
        sqlx::query(
            "INSERT INTO sip_voicemail_mwi_subscriptions
               (subscriber, domain, call_id, subscriber_tag, subscriber_ip, subscriber_port, expires_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON DUPLICATE KEY UPDATE
               subscriber_tag = VALUES(subscriber_tag),
               subscriber_ip = VALUES(subscriber_ip),
               subscriber_port = VALUES(subscriber_port),
               expires_at = VALUES(expires_at),
               cseq = cseq + 1",
        )
        .bind(&subscriber)
        .bind(&domain)
        .bind(&call_id)
        .bind(&subscriber_tag)
        .bind(src.ip().to_string())
        .bind(src.port())
        .bind(expires_at)
        .execute(&self.pool)
        .await?;

        self.notify_mailbox(&subscriber, &domain).await?;
        Ok(base_response(msg, 200, "OK").build())
    }

    pub async fn notify_mailbox(&self, username: &str, domain: &str) -> Result<()> {
        let (new_count, saved_count) = self.message_counts(username, domain).await?;
        let rows: Vec<(String, String, String, u16, u32)> = sqlx::query_as(
            "SELECT call_id, subscriber_tag, subscriber_ip, subscriber_port, cseq
             FROM sip_voicemail_mwi_subscriptions
             WHERE subscriber = ? AND domain = ? AND expires_at > NOW()",
        )
        .bind(username)
        .bind(domain)
        .fetch_all(&self.pool)
        .await?;

        for (call_id, tag, ip, port, cseq) in rows {
            let notify = build_notify(username, domain, &call_id, &tag, cseq + 1, new_count, saved_count);
            let addr: SocketAddr = format!("{}:{}", ip, port).parse()?;
            if let Err(e) = self.socket.send_to(notify.as_bytes(), addr).await {
                warn!("Failed to send voicemail MWI NOTIFY to {}: {}", addr, e);
            } else {
                info!("Sent voicemail MWI NOTIFY to {}", addr);
            }
        }
        Ok(())
    }

    async fn message_counts(&self, username: &str, domain: &str) -> Result<(i64, i64)> {
        let row: Option<(i64, i64)> = sqlx::query_as(
            "SELECT
               (SELECT COUNT(*) FROM sip_voicemail_messages m
                WHERE m.box_id = b.id AND m.status = 'new') AS new_count,
               (SELECT COUNT(*) FROM sip_voicemail_messages m
                WHERE m.box_id = b.id AND m.status = 'saved') AS saved_count
             FROM sip_voicemail_boxes b
             WHERE b.username = ? AND b.domain = ?",
        )
        .bind(username)
        .bind(domain)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.unwrap_or((0, 0)))
    }
}

pub fn build_message_summary_body(username: &str, domain: &str, new_count: i64, saved_count: i64) -> String {
    let waiting = if new_count > 0 { "yes" } else { "no" };
    format!(
        "Messages-Waiting: {}\r\nMessage-Account: sip:{}@{}\r\nVoice-Message: {}/{} (0/0)\r\n",
        waiting, username, domain, new_count, saved_count
    )
}

fn build_notify(
    username: &str,
    domain: &str,
    call_id: &str,
    subscriber_tag: &str,
    cseq: u32,
    new_count: i64,
    saved_count: i64,
) -> String {
    let body = build_message_summary_body(username, domain, new_count, saved_count);
    format!(
        "NOTIFY sip:{}@{} SIP/2.0\r\nVia: SIP/2.0/UDP {};branch=z9hG4bK-vm-{}\r\nFrom: <sip:{}@{}>;tag=sip3-mwi\r\nTo: <sip:{}@{}>;tag={}\r\nCall-ID: {}\r\nCSeq: {} NOTIFY\r\nEvent: message-summary\r\nSubscription-State: active\r\nContent-Type: application/simple-message-summary\r\nContent-Length: {}\r\n\r\n{}",
        username,
        domain,
        domain,
        cseq,
        username,
        domain,
        username,
        domain,
        subscriber_tag,
        call_id,
        cseq,
        body.len(),
        body
    )
}

fn extract_param(header: &str, name: &str) -> Option<String> {
    header.split(';').skip(1).find_map(|part| {
        let mut kv = part.trim().splitn(2, '=');
        let key = kv.next()?.trim();
        let value = kv.next()?.trim();
        (key.eq_ignore_ascii_case(name)).then(|| value.trim_matches('"').to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_waiting_body_with_new_and_saved_counts() {
        let body = build_message_summary_body("1001", "sip.air32.cn", 2, 1);
        assert!(body.contains("Messages-Waiting: yes"));
        assert!(body.contains("Message-Account: sip:1001@sip.air32.cn"));
        assert!(body.contains("Voice-Message: 2/1 (0/0)"));
    }

    #[test]
    fn formats_empty_body_without_waiting() {
        let body = build_message_summary_body("1001", "sip.air32.cn", 0, 0);
        assert!(body.contains("Messages-Waiting: no"));
        assert!(body.contains("Voice-Message: 0/0 (0/0)"));
    }
}
