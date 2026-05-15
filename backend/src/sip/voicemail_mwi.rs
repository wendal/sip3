use anyhow::Result;
use chrono::Utc;
use sqlx::MySqlPool;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use tokio::net::UdpSocket;
use tracing::{info, warn};

use super::handler::{SipMessage, SipResponseBuilder, base_response, extract_uri, uri_username};
use crate::config::Config;

static MWI_NOTIFIER: OnceLock<VoicemailMwi> = OnceLock::new();

#[derive(Clone)]
pub struct VoicemailMwi {
    pool: MySqlPool,
    cfg: Config,
    socket: Arc<UdpSocket>,
}

impl VoicemailMwi {
    pub fn new(pool: MySqlPool, cfg: Config, socket: Arc<UdpSocket>) -> Self {
        let mwi = Self { pool, cfg, socket };
        let _ = MWI_NOTIFIER.set(mwi.clone());
        mwi
    }

    pub async fn notify_mailbox_if_available(username: &str, domain: &str) -> Result<bool> {
        if let Some(mwi) = MWI_NOTIFIER.get() {
            mwi.notify_mailbox(username, domain).await?;
            Ok(true)
        } else {
            Ok(false)
        }
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
        if !self
            .subscriber_source_is_registered(&subscriber, &domain, src)
            .await?
        {
            warn!(
                "Rejecting voicemail MWI SUBSCRIBE from unregistered source {} for {}@{}",
                src, subscriber, domain
            );
            return Ok(base_response(msg, 403, "Forbidden").build());
        }

        let contact = format!(
            "<sip:{}@{}:{}>",
            subscriber, self.cfg.server.public_ip, self.cfg.server.sip_port
        );

        if expires == 0 {
            sqlx::query(
                "DELETE FROM sip_voicemail_mwi_subscriptions WHERE subscriber = ? AND domain = ? AND call_id = ?",
            )
            .bind(&subscriber)
            .bind(&domain)
            .bind(&call_id)
            .execute(&self.pool)
            .await?;
            return Ok(base_response_with_to(
                msg,
                200,
                "OK",
                &mwi_subscribe_to_header(msg.to_header().unwrap_or(""), &call_id),
            )
            .header("Contact", &contact)
            .header("Expires", "0")
            .build());
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

        let mwi = self.clone();
        let sub = subscriber.to_string();
        let dom = domain.clone();
        tokio::spawn(async move {
            if let Err(e) = mwi.notify_mailbox(&sub, &dom).await {
                warn!("Failed to send initial MWI NOTIFY to {}: {}", sub, e);
            }
        });

        Ok(base_response_with_to(
            msg,
            200,
            "OK",
            &mwi_subscribe_to_header(msg.to_header().unwrap_or(""), &call_id),
        )
        .header("Contact", &contact)
        .header("Expires", &expires.to_string())
        .build())
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
            let next_cseq = cseq + 1;
            let notify = build_notify(
                username,
                domain,
                &call_id,
                &tag,
                next_cseq,
                new_count,
                saved_count,
            );
            let addr: SocketAddr = format!("{}:{}", ip, port).parse()?;

            // Send NOTIFY. Response processing (200 OK acknowledgment, 481 removal) is
            // handled when MWI is wired into SipHandler transaction routing (Task 8).
            if let Err(e) = self.socket.send_to(notify.as_bytes(), addr).await {
                warn!("Failed to send voicemail MWI NOTIFY to {}: {}", addr, e);
                continue;
            }

            info!("Sent voicemail MWI NOTIFY to {} (CSeq {})", addr, next_cseq);

            // Persist the CSeq increment after successful send
            sqlx::query(
                "UPDATE sip_voicemail_mwi_subscriptions SET cseq = ? 
                 WHERE subscriber = ? AND domain = ? AND call_id = ?",
            )
            .bind(next_cseq)
            .bind(username)
            .bind(domain)
            .bind(&call_id)
            .execute(&self.pool)
            .await?;
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

    async fn subscriber_source_is_registered(
        &self,
        subscriber: &str,
        domain: &str,
        src: SocketAddr,
    ) -> Result<bool> {
        let rows: Vec<(String, u16)> = sqlx::query_as(
            "SELECT source_ip, source_port FROM sip_registrations
             WHERE username = ? AND domain = ? AND expires_at > NOW()",
        )
        .bind(subscriber)
        .bind(domain)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .any(|(ip, port)| registered_source_matches(&ip, port, src)))
    }
}

pub fn build_message_summary_body(
    username: &str,
    domain: &str,
    new_count: i64,
    saved_count: i64,
) -> String {
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
    let from_tag = mwi_server_tag(call_id);
    let branch = format!("z9hG4bK-vm-{}-{}", from_tag, cseq);

    format!(
        "NOTIFY sip:{}@{} SIP/2.0\r\nVia: SIP/2.0/UDP {};branch={}\r\nMax-Forwards: 70\r\nFrom: <sip:{}@{}>;tag={}\r\nTo: <sip:{}@{}>;tag={}\r\nCall-ID: {}\r\nCSeq: {} NOTIFY\r\nEvent: message-summary\r\nSubscription-State: active\r\nContent-Type: application/simple-message-summary\r\nContent-Length: {}\r\n\r\n{}",
        username,
        domain,
        domain,
        branch,
        username,
        domain,
        from_tag,
        username,
        domain,
        subscriber_tag,
        call_id,
        cseq,
        body.len(),
        body
    )
}

fn base_response_with_to(
    req: &SipMessage,
    status_code: u16,
    reason: &str,
    to: &str,
) -> SipResponseBuilder {
    let mut builder = SipResponseBuilder::new(status_code, reason);
    for via in req.via_headers() {
        builder = builder.header("Via", via);
    }
    if let Some(from) = req.from_header() {
        builder = builder.header("From", from);
    }
    builder = builder.header("To", to);
    if let Some(call_id) = req.call_id() {
        builder = builder.header("Call-ID", call_id);
    }
    if let Some(cseq) = req.cseq() {
        builder = builder.header("CSeq", cseq);
    }
    builder.header("Server", "SIP3/0.1.0")
}

fn mwi_server_tag(call_id: &str) -> String {
    use sha1::{Digest, Sha1};

    let digest = Sha1::digest(call_id.as_bytes());
    let hex = format!("{digest:x}");
    format!("sip3-mwi-{}", &hex[..16])
}

fn mwi_subscribe_to_header(to: &str, call_id: &str) -> String {
    let tag = mwi_server_tag(call_id);
    if to.to_ascii_lowercase().contains(";tag=") {
        to.to_string()
    } else {
        format!("{};tag={}", to, tag)
    }
}

pub fn registered_source_matches(
    registered_ip: &str,
    registered_port: u16,
    src: SocketAddr,
) -> bool {
    registered_ip
        .parse::<std::net::IpAddr>()
        .map(|ip| ip == src.ip())
        .unwrap_or(false)
        && registered_port == src.port()
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

    #[test]
    fn notify_includes_required_headers_and_correct_content_length() {
        let notify = build_notify("1001", "sip.air32.cn", "test-call-123", "user-tag", 5, 2, 1);

        assert!(notify.contains("Max-Forwards: 70"));
        assert!(notify.contains("Event: message-summary"));
        assert!(notify.contains("Subscription-State: active"));
        assert!(notify.contains("Content-Type: application/simple-message-summary"));

        let body = build_message_summary_body("1001", "sip.air32.cn", 2, 1);
        let expected_length = format!("Content-Length: {}", body.len());
        assert!(notify.contains(&expected_length));

        assert!(notify.contains("branch=z9hG4bK-vm-"));
        assert!(notify.contains("-5"));
    }

    #[test]
    fn notify_produces_unique_branches_for_different_call_ids() {
        let notify1 = build_notify("1001", "sip.air32.cn", "call-aaa", "tag1", 3, 0, 0);
        let notify2 = build_notify("1001", "sip.air32.cn", "call-bbb", "tag2", 3, 0, 0);

        let branch1 = extract_header_value(&notify1, "Via").unwrap();
        let branch2 = extract_header_value(&notify2, "Via").unwrap();

        assert_ne!(
            branch1, branch2,
            "Branches should differ for different call-IDs"
        );
        assert!(branch1.contains(&mwi_server_tag("call-aaa")));
        assert!(branch2.contains(&mwi_server_tag("call-bbb")));
    }

    #[test]
    fn notify_produces_unique_from_tags_for_different_call_ids() {
        let notify1 = build_notify("1001", "sip.air32.cn", "call-aaa", "tag1", 1, 0, 0);
        let notify2 = build_notify("1001", "sip.air32.cn", "call-bbb", "tag2", 1, 0, 0);

        let from1 = extract_header_value(&notify1, "From").unwrap();
        let from2 = extract_header_value(&notify2, "From").unwrap();

        assert_ne!(
            from1, from2,
            "From tags should differ for different call-IDs"
        );
        assert!(from1.contains(&mwi_server_tag("call-aaa")));
        assert!(from2.contains(&mwi_server_tag("call-bbb")));
    }

    #[test]
    fn subscribe_ok_to_tag_matches_notify_from_tag() {
        let call_id = "call-aaa";
        let tag = mwi_server_tag(call_id);
        let response_to = mwi_subscribe_to_header("<sip:1001@sip.air32.cn>", call_id);
        let notify = build_notify("1001", "sip.air32.cn", call_id, "subscriber-tag", 1, 0, 0);
        let notify_from = extract_header_value(&notify, "From").unwrap();

        assert!(response_to.contains(&format!("tag={tag}")));
        assert!(notify_from.contains(&format!("tag={tag}")));
    }

    #[test]
    fn mwi_server_tag_is_token_safe_for_common_call_ids() {
        let call_id = "abc@client.example";
        let tag = mwi_server_tag(call_id);
        let response_to = mwi_subscribe_to_header("<sip:1001@sip.air32.cn>", call_id);
        let notify = build_notify("1001", "sip.air32.cn", call_id, "subscriber-tag", 1, 0, 0);
        let notify_from = extract_header_value(&notify, "From").unwrap();

        assert!(tag.starts_with("sip3-mwi-"));
        assert!(tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        assert!(response_to.contains(&format!("tag={tag}")));
        assert!(notify_from.contains(&format!("tag={tag}")));
    }

    #[test]
    fn notify_branch_is_token_safe_for_common_call_ids() {
        let notify = build_notify(
            "1001",
            "sip.air32.cn",
            "abc@client.example",
            "subscriber-tag",
            1,
            0,
            0,
        );
        let via = extract_header_value(&notify, "Via").unwrap();
        let branch = via.split("branch=").nth(1).unwrap();

        assert!(
            branch
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        );
    }

    #[test]
    fn registered_source_match_requires_same_ip_and_port() {
        let src: SocketAddr = "192.0.2.10:5062".parse().unwrap();

        assert!(registered_source_matches("192.0.2.10", 5062, src));
        assert!(!registered_source_matches("192.0.2.10", 5063, src));
        assert!(!registered_source_matches("192.0.2.11", 5062, src));
        assert!(!registered_source_matches("not-an-ip", 5062, src));
    }

    fn extract_header_value(msg: &str, header: &str) -> Option<String> {
        for line in msg.lines() {
            if line.to_lowercase().starts_with(&header.to_lowercase()) {
                return Some(line.to_string());
            }
        }
        None
    }
}
