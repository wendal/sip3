use anyhow::Result;
use chrono::Utc;
use rand::Rng;
use sqlx::MySqlPool;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::{info, warn};

use super::handler::{base_response, extract_uri, uri_username, SipMessage};
use crate::config::Config;

#[derive(Debug, Clone, Copy)]
pub enum PresenceStatus {
    Open,   // registered and idle
    Closed, // not registered
}

#[derive(Clone)]
pub struct Presence {
    pool: MySqlPool,
    cfg: Config,
    socket: Arc<UdpSocket>,
}

impl Presence {
    pub fn new(pool: MySqlPool, cfg: Config, socket: Arc<UdpSocket>) -> Self {
        Self { pool, cfg, socket }
    }

    /// Handle a SIP SUBSCRIBE request for presence (BLF).
    pub async fn handle_subscribe(&self, msg: &SipMessage, src: SocketAddr) -> Result<String> {
        let event = msg.header("event").unwrap_or("presence").to_lowercase();
        let event = event.split(';').next().unwrap_or("presence").trim();
        if event != "presence" && event != "dialog" {
            return Ok(base_response(msg, 489, "Bad Event").build());
        }

        let from = msg.from_header().unwrap_or("");
        let from_uri = extract_uri(from).unwrap_or_default();
        let subscriber = uri_username(&from_uri).unwrap_or_default();
        let subscriber_tag = extract_param(from, "tag").unwrap_or_default();

        let req_uri = msg.request_uri.as_deref().unwrap_or("");
        let target = uri_username(req_uri).unwrap_or_default();

        if subscriber.is_empty() || target.is_empty() {
            return Ok(base_response(msg, 400, "Bad Request").build());
        }

        let domain = self.cfg.server.sip_domain.clone();
        let call_id = msg.call_id().unwrap_or("").to_string();
        let expires = msg.expires().unwrap_or(300);

        if expires == 0 {
            // Unsubscribe
            sqlx::query(
                "DELETE FROM sip_presence_subscriptions \
                 WHERE subscriber = ? AND target = ? AND domain = ?",
            )
            .bind(&subscriber)
            .bind(&target)
            .bind(&domain)
            .execute(&self.pool)
            .await?;

            let notify = build_notify(
                &subscriber,
                &subscriber_tag,
                &target,
                &domain,
                &self.cfg.server.sip_domain,
                &call_id,
                1,
                "terminated;reason=timeout",
                PresenceStatus::Closed,
            );
            let _ = self.socket.send_to(notify.as_bytes(), src).await;
            return Ok(base_response(msg, 200, "OK").build());
        }

        let expires_at = (Utc::now() + chrono::Duration::seconds(i64::from(expires))).naive_utc();
        sqlx::query(
            "INSERT INTO sip_presence_subscriptions \
               (subscriber, target, domain, call_id, subscriber_tag, \
                subscriber_ip, subscriber_port, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON DUPLICATE KEY UPDATE \
               call_id = VALUES(call_id), \
               subscriber_tag = VALUES(subscriber_tag), \
               subscriber_ip = VALUES(subscriber_ip), \
               subscriber_port = VALUES(subscriber_port), \
               expires_at = VALUES(expires_at), \
               cseq = cseq + 1",
        )
        .bind(&subscriber)
        .bind(&target)
        .bind(&domain)
        .bind(&call_id)
        .bind(&subscriber_tag)
        .bind(src.ip().to_string())
        .bind(src.port())
        .bind(expires_at)
        .execute(&self.pool)
        .await?;

        info!(
            "Presence subscription: {} watching {}@{} (expires {}s)",
            subscriber, target, domain, expires
        );

        let cseq: u32 = sqlx::query_scalar(
            "SELECT cseq FROM sip_presence_subscriptions \
             WHERE subscriber = ? AND target = ? AND domain = ?",
        )
        .bind(&subscriber)
        .bind(&target)
        .bind(&domain)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(1);

        let status = self.get_status(&target, &domain).await;
        let sub_state = format!("active;expires={}", expires);
        let notify = build_notify(
            &subscriber,
            &subscriber_tag,
            &target,
            &domain,
            &self.cfg.server.sip_domain,
            &call_id,
            cseq,
            &sub_state,
            status,
        );
        let _ = self.socket.send_to(notify.as_bytes(), src).await;

        Ok(base_response(msg, 200, "OK").build())
    }

    /// Send NOTIFY to all active subscribers watching `username@domain`.
    pub async fn notify_status_change(&self, username: &str, domain: &str, status: PresenceStatus) {
        let rows: Vec<(
            String,
            String,
            String,
            u16,
            String,
            u32,
            chrono::NaiveDateTime,
        )> = match sqlx::query_as(
            "SELECT subscriber, subscriber_tag, subscriber_ip, subscriber_port, \
                        call_id, cseq, expires_at \
                 FROM sip_presence_subscriptions \
                 WHERE target = ? AND domain = ? AND expires_at > NOW()",
        )
        .bind(username)
        .bind(domain)
        .fetch_all(&self.pool)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    "Failed to fetch presence subscriptions for {}@{}: {}",
                    username, domain, e
                );
                return;
            }
        };

        for (subscriber, sub_tag, ip, port, call_id, cseq, expires_at) in rows {
            let new_cseq = cseq.saturating_add(1);
            let _ = sqlx::query(
                "UPDATE sip_presence_subscriptions SET cseq = ? \
                 WHERE subscriber = ? AND target = ? AND domain = ?",
            )
            .bind(new_cseq)
            .bind(&subscriber)
            .bind(username)
            .bind(domain)
            .execute(&self.pool)
            .await;

            let addr: SocketAddr = match format!("{}:{}", ip, port).parse() {
                Ok(a) => a,
                Err(_) => continue,
            };

            let remaining = (expires_at - Utc::now().naive_utc()).num_seconds().max(0) as u32;
            let sub_state = format!("active;expires={}", remaining);
            let notify = build_notify(
                &subscriber,
                &sub_tag,
                username,
                domain,
                &self.cfg.server.sip_domain,
                &call_id,
                new_cseq,
                &sub_state,
                status,
            );
            let _ = self.socket.send_to(notify.as_bytes(), addr).await;
            info!(
                "Presence NOTIFY → {} about {}@{} ({:?})",
                subscriber, username, domain, status
            );
        }
    }

    async fn get_status(&self, username: &str, domain: &str) -> PresenceStatus {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sip_registrations \
             WHERE username = ? AND domain = ? AND expires_at > NOW()",
        )
        .bind(username)
        .bind(domain)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0);

        if count > 0 {
            PresenceStatus::Open
        } else {
            PresenceStatus::Closed
        }
    }
}

/// Extract a named parameter value from a SIP header value.
/// e.g. extract_param("sip:alice@example.com>;tag=abc", "tag") → Some("abc")
fn extract_param(header_val: &str, param: &str) -> Option<String> {
    for part in header_val.split(';') {
        let part = part.trim();
        if let Some(_rest) = part
            .to_lowercase()
            .strip_prefix(&format!("{}=", param.to_lowercase()))
        {
            // 'rest' is from the lowercased copy; get the actual value from 'part'
            let val = &part[param.len() + 1..];
            return Some(val.trim_matches('"').to_string());
        }
    }
    None
}

fn build_pidf(target: &str, domain: &str, status: PresenceStatus) -> String {
    let basic = match status {
        PresenceStatus::Open => "open",
        PresenceStatus::Closed => "closed",
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <presence xmlns=\"urn:ietf:params:xml:ns:pidf\"\n\
                   entity=\"sip:{}@{}\">\n\
           <tuple id=\"1\">\n\
             <status><basic>{}</basic></status>\n\
           </tuple>\n\
         </presence>",
        target, domain, basic
    )
}

#[allow(clippy::too_many_arguments)]
fn build_notify(
    subscriber: &str,
    subscriber_tag: &str,
    target: &str,
    domain: &str,
    server_domain: &str,
    call_id: &str,
    cseq: u32,
    sub_state: &str,
    status: PresenceStatus,
) -> String {
    let pidf = build_pidf(target, domain, status);
    let branch = format!("z9hG4bKpres{}", rand_hex(4));
    let to_tag = if subscriber_tag.is_empty() {
        String::new()
    } else {
        format!(";tag={}", subscriber_tag)
    };
    format!(
        "NOTIFY sip:{}@{} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {};branch={}\r\n\
         From: <sip:{}@{}>;tag=pres-srv-1\r\n\
         To: <sip:{}@{}>{}\r\n\
         Call-ID: {}\r\n\
         CSeq: {} NOTIFY\r\n\
         Event: presence\r\n\
         Subscription-State: {}\r\n\
         Content-Type: application/pidf+xml\r\n\
         Content-Length: {}\r\n\
         \r\n\
         {}",
        subscriber,
        domain,
        server_domain,
        branch,
        target,
        domain,
        subscriber,
        domain,
        to_tag,
        call_id,
        cseq,
        sub_state,
        pidf.len(),
        pidf
    )
}

fn rand_hex(bytes: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..bytes)
        .map(|_| format!("{:02x}", rng.gen::<u8>()))
        .collect()
}
