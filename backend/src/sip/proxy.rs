use anyhow::Result;
use chrono::Utc;
use sqlx::MySqlPool;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::{info, warn};

use super::handler::{base_response, extract_uri, uri_username, PendingDialogs, SipMessage};
use crate::config::Config;

#[derive(Clone)]
pub struct Proxy {
    pool: MySqlPool,
    cfg: Config,
    socket: Arc<UdpSocket>,
    /// Shared map of call-id → caller's SocketAddr for response relay.
    pending_dialogs: PendingDialogs,
}

impl Proxy {
    pub fn new(
        pool: MySqlPool,
        cfg: Config,
        socket: Arc<UdpSocket>,
        pending_dialogs: PendingDialogs,
    ) -> Self {
        Self {
            pool,
            cfg,
            socket,
            pending_dialogs,
        }
    }

    pub async fn handle_invite(&self, msg: &SipMessage, src: SocketAddr) -> Result<String> {
        let request_uri = msg.request_uri.as_deref().unwrap_or("");
        let callee = uri_username(request_uri).unwrap_or_default();
        let caller = msg
            .from_header()
            .and_then(extract_uri)
            .and_then(|u| uri_username(&u))
            .unwrap_or_else(|| "unknown".to_string());
        let call_id = msg.call_id().unwrap_or("").to_string();
        let domain = self.cfg.server.sip_domain.clone();

        if callee.is_empty() {
            return Ok(base_response(msg, 400, "Bad Request").build());
        }

        let max_fwd = msg
            .header("max-forwards")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(70);
        if max_fwd == 0 {
            return Ok(base_response(msg, 483, "Too Many Hops").build());
        }

        // Look up callee's registration
        let row: Option<(String, String, u16)> = sqlx::query_as(
            "SELECT contact_uri, source_ip, source_port FROM sip_registrations
             WHERE username = ? AND domain = ? AND expires_at > NOW()",
        )
        .bind(&callee)
        .bind(&domain)
        .fetch_optional(&self.pool)
        .await?;

        let (contact_uri, source_ip, source_port) = match row {
            Some(r) => r,
            None => {
                warn!("INVITE to unregistered user: {}", callee);
                return Ok(base_response(msg, 404, "Not Found").build());
            }
        };

        // Record call attempt
        let now = Utc::now().naive_utc();
        let _ = sqlx::query(
            r#"INSERT INTO sip_calls (call_id, caller, callee, status, started_at)
               VALUES (?, ?, ?, 'trying', ?)
               ON DUPLICATE KEY UPDATE status = 'trying', started_at = ?"#,
        )
        .bind(&call_id)
        .bind(format!("{}@{}", caller, domain))
        .bind(format!("{}@{}", callee, domain))
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await;

        // Store the caller's address so we can relay callee responses back.
        {
            let mut dialogs = self.pending_dialogs.lock().await;
            dialogs.insert(call_id.clone(), src);
        }

        // Forward INVITE to callee
        let target_addr: SocketAddr = format!("{}:{}", source_ip, source_port).parse()?;
        let forwarded = self.build_forwarded_invite(msg, &contact_uri, max_fwd - 1);
        self.socket
            .send_to(forwarded.as_bytes(), target_addr)
            .await?;

        info!(
            "Proxied INVITE from {} to {} at {}",
            caller, callee, target_addr
        );

        Ok(base_response(msg, 100, "Trying").build())
    }

    fn build_forwarded_invite(&self, msg: &SipMessage, contact_uri: &str, max_fwd: u32) -> String {
        let branch = format!("z9hG4bKproxy{}", rand_token());
        let mut out = format!("INVITE {} SIP/2.0\r\n", contact_uri);

        out.push_str(&format!(
            "Via: SIP/2.0/UDP {};branch={}\r\n",
            self.cfg.server.sip_domain, branch
        ));
        for via in msg.via_headers() {
            out.push_str(&format!("Via: {}\r\n", via));
        }
        out.push_str(&format!("Max-Forwards: {}\r\n", max_fwd));

        for (name, vals) in &msg.headers {
            if name == "via" || name == "max-forwards" {
                continue;
            }
            for val in vals {
                out.push_str(&format!("{}: {}\r\n", capitalize_header(name), val));
            }
        }

        out.push_str("\r\n");
        out.push_str(&msg.body);
        out
    }

    pub async fn handle_ack(&self, msg: &SipMessage, _src: SocketAddr) -> Result<()> {
        let call_id = msg.call_id().unwrap_or("").to_string();
        let request_uri = msg.request_uri.as_deref().unwrap_or("");
        let callee = uri_username(request_uri).unwrap_or_default();
        let domain = self.cfg.server.sip_domain.clone();

        let _ = sqlx::query(
            "UPDATE sip_calls SET status = 'answered', answered_at = NOW() WHERE call_id = ?",
        )
        .bind(&call_id)
        .execute(&self.pool)
        .await;

        if !callee.is_empty() {
            let row: Option<(String, u16)> = sqlx::query_as(
                "SELECT source_ip, source_port FROM sip_registrations
                 WHERE username = ? AND domain = ? AND expires_at > NOW()",
            )
            .bind(&callee)
            .bind(&domain)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten();

            if let Some((ip, port)) = row {
                if let Ok(target) = format!("{}:{}", ip, port).parse::<SocketAddr>() {
                    let _ = self.socket.send_to(msg.raw.as_bytes(), target).await;
                    info!("Forwarded ACK to {} at {}", callee, target);
                }
            }
        }

        Ok(())
    }

    pub async fn handle_bye(&self, msg: &SipMessage, _src: SocketAddr) -> Result<String> {
        let call_id = msg.call_id().unwrap_or("").to_string();
        let request_uri = msg.request_uri.as_deref().unwrap_or("");
        let callee = uri_username(request_uri).unwrap_or_default();
        let domain = self.cfg.server.sip_domain.clone();

        let _ = sqlx::query(
            "UPDATE sip_calls SET status = 'ended', ended_at = NOW() WHERE call_id = ?",
        )
        .bind(&call_id)
        .execute(&self.pool)
        .await;

        // Clean up pending dialog entry
        self.pending_dialogs.lock().await.remove(&call_id);

        if !callee.is_empty() {
            let row: Option<(String, u16)> = sqlx::query_as(
                "SELECT source_ip, source_port FROM sip_registrations
                 WHERE username = ? AND domain = ? AND expires_at > NOW()",
            )
            .bind(&callee)
            .bind(&domain)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten();

            if let Some((ip, port)) = row {
                if let Ok(target) = format!("{}:{}", ip, port).parse::<SocketAddr>() {
                    let _ = self.socket.send_to(msg.raw.as_bytes(), target).await;
                    info!("Forwarded BYE to {} at {}", callee, target);
                }
            }
        }

        info!("Call ended: {}", call_id);
        Ok(base_response(msg, 200, "OK").build())
    }

    pub async fn handle_cancel(&self, msg: &SipMessage, _src: SocketAddr) -> Result<String> {
        let call_id = msg.call_id().unwrap_or("").to_string();

        let _ = sqlx::query(
            "UPDATE sip_calls SET status = 'cancelled', ended_at = NOW() WHERE call_id = ?",
        )
        .bind(&call_id)
        .execute(&self.pool)
        .await;

        // Clean up pending dialog entry
        self.pending_dialogs.lock().await.remove(&call_id);

        info!("Call cancelled: {}", call_id);
        Ok(base_response(msg, 200, "OK").build())
    }
}

fn rand_token() -> String {
    use rand::Rng;
    let n: u64 = rand::thread_rng().gen();
    format!("{:x}", n)
}

fn capitalize_header(name: &str) -> String {
    match name {
        "call-id" => "Call-ID".to_string(),
        "cseq" => "CSeq".to_string(),
        "www-authenticate" => "WWW-Authenticate".to_string(),
        "content-type" => "Content-Type".to_string(),
        "content-length" => "Content-Length".to_string(),
        "user-agent" => "User-Agent".to_string(),
        other => other
            .split('-')
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join("-"),
    }
}
