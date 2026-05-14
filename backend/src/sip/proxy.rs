use anyhow::Result;
use chrono::Utc;
use sqlx::MySqlPool;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::{info, warn};

use super::handler::{
    base_response, extract_uri, uri_username, ActiveDialogs, DialogInfo, PendingDialogs,
    SipMessage,
};
use super::media::{is_webrtc_sdp, make_plain_rtp_sdp, rewrite_sdp, MediaRelay};
use super::webrtc_gateway::WebRtcGateway;
use crate::config::Config;

#[derive(Clone)]
pub struct Proxy {
    pool: MySqlPool,
    cfg: Config,
    socket: Arc<UdpSocket>,
    /// Shared map of call-id → caller's SocketAddr for response relay.
    pending_dialogs: PendingDialogs,
    media_relay: MediaRelay,
    /// Established dialogs (post-ACK): call-id → (caller_addr, callee_addr).
    /// Used for bidirectional BYE/INFO routing.
    active_dialogs: ActiveDialogs,
    webrtc_gateway: Arc<WebRtcGateway>,
}

impl Proxy {
    pub fn new(
        pool: MySqlPool,
        cfg: Config,
        socket: Arc<UdpSocket>,
        pending_dialogs: PendingDialogs,
        media_relay: MediaRelay,
        webrtc_gateway: Arc<WebRtcGateway>,
    ) -> Self {
        Self {
            pool,
            cfg,
            socket,
            pending_dialogs,
            media_relay,
            active_dialogs: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            webrtc_gateway,
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

        // Verify the caller has an enabled account in our domain before
        // proxying any calls — prevents unauthenticated call injection.
        if caller != "unknown" {
            let caller_ok: Option<(i64,)> = sqlx::query_as(
                "SELECT id FROM sip_accounts WHERE username = ? AND domain = ? AND enabled = 1",
            )
            .bind(&caller)
            .bind(&domain)
            .fetch_optional(&self.pool)
            .await?;

            if caller_ok.is_none() {
                warn!("INVITE from unrecognised caller: {}@{}", caller, domain);
                return Ok(base_response(msg, 403, "Forbidden").build());
            }
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

        let target_addr: SocketAddr = format!("{}:{}", source_ip, source_port).parse()?;

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

        // Store dialog endpoints for bidirectional routing of subsequent requests.
        {
            let mut active = self.active_dialogs.lock().await;
            active.insert(
                call_id.clone(),
                DialogInfo {
                    caller_addr: src,
                    callee_addr: target_addr,
                },
            );
        }

        // Allocate media for this call: WebRTC INVITE or plain SIP.
        let rewritten_body = if !msg.body.is_empty() && is_webrtc_sdp(&msg.body) {
            // Browser-originated WebRTC INVITE: create a WebRTC session and
            // replace the WebRTC SDP with a plain RTP offer for the SIP phone.
            match self.webrtc_gateway.create_session(call_id.clone(), &msg.body).await {
                Ok((_answer_sdp, sip_port)) => {
                    info!(
                        "WebRTC session for {}: forwarding with plain RTP port {}",
                        call_id, sip_port
                    );
                    let public_ip = &self.cfg.server.public_ip;
                    Some(make_plain_rtp_sdp(public_ip, sip_port))
                }
                Err(e) => {
                    warn!("WebRTC session creation failed for {}: {}", call_id, e);
                    None
                }
            }
        } else {
            // Legacy plain-RTP call: allocate a symmetric relay session.
            match self.media_relay.allocate_session(call_id.clone()).await {
                Ok((relay_port_a, _relay_port_b)) => {
                    let public_ip = self.media_relay.public_ip.as_str();
                    if msg.body.is_empty() {
                        None
                    } else {
                        Some(rewrite_sdp(&msg.body, public_ip, relay_port_a))
                    }
                }
                Err(e) => {
                    warn!("Failed to allocate media relay for {}: {}", call_id, e);
                    None
                }
            }
        };

        // Forward INVITE to callee (with rewritten SDP and Record-Route).
        let forwarded = self.build_forwarded_invite(msg, &contact_uri, max_fwd - 1, rewritten_body);
        self.socket
            .send_to(forwarded.as_bytes(), target_addr)
            .await?;

        info!(
            "Proxied INVITE from {} to {} at {}",
            caller, callee, target_addr
        );

        Ok(base_response(msg, 100, "Trying").build())
    }

    fn build_forwarded_invite(
        &self,
        msg: &SipMessage,
        contact_uri: &str,
        max_fwd: u32,
        rewritten_body: Option<String>,
    ) -> String {
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

        // Record-Route so subsequent in-dialog requests (BYE, re-INVITE) are
        // routed through this proxy, keeping media relay effective.
        out.push_str(&format!(
            "Record-Route: <sip:{};lr>\r\n",
            self.cfg.server.sip_domain
        ));

        let body = rewritten_body.as_deref().unwrap_or(&msg.body);

        for (name, vals) in &msg.headers {
            if name == "via"
                || name == "max-forwards"
                || name == "content-length"
                || name == "record-route"
            {
                continue;
            }
            for val in vals {
                out.push_str(&format!("{}: {}\r\n", capitalize_header(name), val));
            }
        }

        out.push_str(&format!("Content-Length: {}\r\n", body.len()));
        out.push_str("\r\n");
        out.push_str(body);
        out
    }

    pub async fn handle_ack(&self, msg: &SipMessage, _src: SocketAddr) -> Result<()> {
        let call_id = msg.call_id().unwrap_or("").to_string();

        let _ = sqlx::query(
            "UPDATE sip_calls SET status = 'answered', answered_at = NOW() WHERE call_id = ?",
        )
        .bind(&call_id)
        .execute(&self.pool)
        .await;

        // Forward ACK to callee using the dialog state (avoids a DB lookup).
        let callee_addr = {
            let active = self.active_dialogs.lock().await;
            active.get(&call_id).map(|d| d.callee_addr)
        };

        if let Some(target) = callee_addr {
            let _ = self.socket.send_to(msg.raw.as_bytes(), target).await;
            info!("Forwarded ACK for call {} to {}", call_id, target);
        } else {
            // Fallback: look up callee registration from DB.
            let request_uri = msg.request_uri.as_deref().unwrap_or("");
            let callee = uri_username(request_uri).unwrap_or_default();
            let domain = self.cfg.server.sip_domain.clone();
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
                        info!("Forwarded ACK (fallback) to {} at {}", callee, target);
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn handle_bye(&self, msg: &SipMessage, src: SocketAddr) -> Result<String> {
        let call_id = msg.call_id().unwrap_or("").to_string();

        let _ = sqlx::query(
            "UPDATE sip_calls SET status = 'ended', ended_at = NOW() WHERE call_id = ?",
        )
        .bind(&call_id)
        .execute(&self.pool)
        .await;

        // Use active_dialogs to route BYE in both directions.
        let forward_addr = {
            let mut active = self.active_dialogs.lock().await;
            if let Some(dialog) = active.remove(&call_id) {
                // Determine the other party based on who sent the BYE.
                if src == dialog.caller_addr {
                    Some(dialog.callee_addr)
                } else {
                    Some(dialog.caller_addr)
                }
            } else {
                None
            }
        };

        self.pending_dialogs.lock().await.remove(&call_id);
        self.media_relay.remove_session(&call_id).await;
        self.webrtc_gateway.remove_session(&call_id).await;

        if let Some(target) = forward_addr {
            let _ = self.socket.send_to(msg.raw.as_bytes(), target).await;
            info!("Forwarded BYE to {}", target);
        } else {
            // Fallback: route by request-URI (no active dialog found).
            let request_uri = msg.request_uri.as_deref().unwrap_or("");
            let callee = uri_username(request_uri).unwrap_or_default();
            let domain = self.cfg.server.sip_domain.clone();
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
                    if let Ok(fallback) = format!("{}:{}", ip, port).parse::<SocketAddr>() {
                        let _ = self.socket.send_to(msg.raw.as_bytes(), fallback).await;
                        info!("Forwarded BYE (fallback) to {} at {}", callee, fallback);
                    }
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

        // Forward CANCEL to callee so it can stop ringing.
        let callee_addr = {
            let active = self.active_dialogs.lock().await;
            active.get(&call_id).map(|d| d.callee_addr)
        };

        if let Some(target) = callee_addr {
            let _ = self.socket.send_to(msg.raw.as_bytes(), target).await;
            info!("Forwarded CANCEL for call {} to {}", call_id, target);
        } else {
            warn!("No dialog found to forward CANCEL for call_id: {}", call_id);
        }

        // Clean up all state for this call.
        self.pending_dialogs.lock().await.remove(&call_id);
        self.active_dialogs.lock().await.remove(&call_id);
        self.media_relay.remove_session(&call_id).await;
        self.webrtc_gateway.remove_session(&call_id).await;

        info!("Call cancelled: {}", call_id);
        Ok(base_response(msg, 200, "OK").build())
    }

    /// Forward a REFER request to the other party in the active dialog.
    ///
    /// The proxy responds 202 Accepted immediately; NOTIFY updates from the
    /// transfer target will be forwarded back to the REFER sender via
    /// `handle_notify`.
    pub async fn handle_refer(&self, msg: &SipMessage, src: SocketAddr) -> Result<String> {
        let call_id = msg.call_id().unwrap_or("").to_string();

        let forward_addr = {
            let active = self.active_dialogs.lock().await;
            active.get(&call_id).map(|d| {
                if src == d.caller_addr {
                    d.callee_addr
                } else {
                    d.caller_addr
                }
            })
        };

        match forward_addr {
            Some(target) => {
                let _ = self.socket.send_to(msg.raw.as_bytes(), target).await;
                info!("Forwarded REFER for call {} to {}", call_id, target);
                Ok(base_response(msg, 202, "Accepted").build())
            }
            None => {
                warn!("No active dialog for REFER call_id: {}", call_id);
                Ok(base_response(msg, 481, "Call/Transaction Does Not Exist").build())
            }
        }
    }

    /// Transparently proxy an in-dialog NOTIFY (e.g. REFER progress) to the other party.
    pub async fn handle_notify(&self, msg: &SipMessage, src: SocketAddr) -> Result<String> {
        let call_id = msg.call_id().unwrap_or("").to_string();

        let forward_addr = {
            let active = self.active_dialogs.lock().await;
            active.get(&call_id).map(|d| {
                if src == d.caller_addr {
                    d.callee_addr
                } else {
                    d.caller_addr
                }
            })
        };

        if let Some(target) = forward_addr {
            let _ = self.socket.send_to(msg.raw.as_bytes(), target).await;
            info!("Forwarded NOTIFY for call {} to {}", call_id, target);
        } else {
            warn!("No active dialog for NOTIFY call_id: {}", call_id);
        }

        Ok(base_response(msg, 200, "OK").build())
    }

    /// Transparently proxy an in-dialog INFO request (e.g. DTMF) to the other party.
    pub async fn handle_info(&self, msg: &SipMessage, src: SocketAddr) -> Result<String> {
        let call_id = msg.call_id().unwrap_or("").to_string();

        let forward_addr = {
            let active = self.active_dialogs.lock().await;
            if let Some(dialog) = active.get(&call_id) {
                if src == dialog.caller_addr {
                    Some(dialog.callee_addr)
                } else {
                    Some(dialog.caller_addr)
                }
            } else {
                None
            }
        };

        if let Some(target) = forward_addr {
            let _ = self.socket.send_to(msg.raw.as_bytes(), target).await;
            info!("Forwarded INFO for call {} to {}", call_id, target);
        } else {
            warn!("No active dialog for INFO call_id: {}", call_id);
        }

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
