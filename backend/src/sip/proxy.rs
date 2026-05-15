use anyhow::Result;
use chrono::Utc;
use sqlx::MySqlPool;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::{info, warn};

use super::handler::{
    ActiveDialogs, DialogInfo, DialogStores, PendingDialogs, SipMessage, base_response,
    extract_uri, md5_hex, uri_username,
};
use super::media::{MediaRelay, is_webrtc_sdp, make_plain_rtp_sdp, rewrite_sdp, sdp_rtp_addr};
use super::registrar::routable_contact_uri;
use super::transport::TransportRegistry;
use super::webrtc_gateway::WebRtcGateway;
use crate::config::Config;

pub const CALLER_ACCOUNT_EXISTS_SQL: &str = "\
    SELECT 1 FROM sip_accounts
    WHERE username = ? AND domain = ? AND enabled = 1";

pub const MESSAGE_SENDER_ACCOUNT_EXISTS_SQL: &str = "\
    SELECT 1 FROM sip_accounts
    WHERE username = ? AND domain = ? AND enabled = 1";

pub fn is_websocket_contact_uri(contact_uri: &str) -> bool {
    contact_uri.to_ascii_lowercase().contains("transport=ws")
}

pub fn should_preserve_webrtc_sdp_for_target(contact_uri: &str, body: &str) -> bool {
    is_websocket_contact_uri(contact_uri) && is_webrtc_sdp(body)
}

pub fn should_bridge_plain_sip_to_websocket_target(contact_uri: &str, body: &str) -> bool {
    is_websocket_contact_uri(contact_uri) && !body.is_empty() && !is_webrtc_sdp(body)
}

pub fn registered_target_uri(username: &str, contact_uri: &str, target_addr: SocketAddr) -> String {
    if is_websocket_contact_uri(contact_uri) {
        contact_uri.to_string()
    } else {
        routable_contact_uri(contact_uri, username, target_addr)
    }
}

pub fn should_refresh_registration_source(
    registered_ip: &str,
    registered_port: u16,
    src: SocketAddr,
) -> bool {
    registered_ip
        .parse::<std::net::IpAddr>()
        .map(|ip| ip == src.ip())
        .unwrap_or(false)
        && registered_port != src.port()
}

fn proxy_via_branch(call_id: &str) -> String {
    let digest = md5_hex(call_id);
    let suffix = &digest[..digest.len().min(16)];
    format!("z9hG4bKproxy{}", suffix)
}

pub fn build_forwarded_cancel_for_target(
    msg: &SipMessage,
    target_uri: &str,
    max_fwd: u32,
    sip_domain: &str,
) -> String {
    let branch = proxy_via_branch(msg.call_id().unwrap_or(""));
    let mut out = format!("CANCEL {} SIP/2.0\r\n", target_uri);
    out.push_str(&format!(
        "Via: SIP/2.0/UDP {};branch={};rport\r\n",
        sip_domain, branch
    ));
    for via in msg.via_headers() {
        out.push_str(&format!("Via: {}\r\n", via));
    }
    out.push_str(&format!("Max-Forwards: {}\r\n", max_fwd));
    for (name, vals) in &msg.headers {
        if name == "via" || name == "max-forwards" || name == "content-length" {
            continue;
        }
        for val in vals {
            out.push_str(&format!("{}: {}\r\n", capitalize_header(name), val));
        }
    }
    out.push_str("Content-Length: 0\r\n\r\n");
    out
}

pub fn build_forwarded_invite_for_target(
    msg: &SipMessage,
    target_uri: &str,
    max_fwd: u32,
    sip_domain: &str,
    rewritten_body: Option<&str>,
) -> String {
    let branch = proxy_via_branch(msg.call_id().unwrap_or(""));
    let mut out = format!("INVITE {} SIP/2.0\r\n", target_uri);

    out.push_str(&format!(
        "Via: SIP/2.0/UDP {};branch={};rport\r\n",
        sip_domain, branch
    ));
    for via in msg.via_headers() {
        out.push_str(&format!("Via: {}\r\n", via));
    }
    out.push_str(&format!("Max-Forwards: {}\r\n", max_fwd));

    // Record-Route so subsequent in-dialog requests (BYE, re-INVITE) are
    // routed through this proxy, keeping media relay effective.
    out.push_str(&format!("Record-Route: <sip:{};lr>\r\n", sip_domain));

    let body = rewritten_body.unwrap_or(&msg.body);

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
    transport_registry: TransportRegistry,
}

impl Proxy {
    pub fn new(
        pool: MySqlPool,
        cfg: Config,
        socket: Arc<UdpSocket>,
        dialog_stores: DialogStores,
        media_relay: MediaRelay,
        webrtc_gateway: Arc<WebRtcGateway>,
        transport_registry: TransportRegistry,
    ) -> Self {
        Self {
            pool,
            cfg,
            socket,
            pending_dialogs: dialog_stores.pending,
            media_relay,
            active_dialogs: dialog_stores.active,
            webrtc_gateway,
            transport_registry,
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
            let caller_ok: Option<(i32,)> = sqlx::query_as(CALLER_ACCOUNT_EXISTS_SQL)
                .bind(&caller)
                .bind(&domain)
                .fetch_optional(&self.pool)
                .await?;

            if caller_ok.is_none() {
                warn!("INVITE from unrecognised caller: {}@{}", caller, domain);
                return Ok(base_response(msg, 403, "Forbidden").build());
            }

            self.refresh_sender_registration_source(&caller, &domain, src)
                .await?;
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
        let caller_is_stream = self.transport_registry.contains(src);
        let callee_is_stream = self.transport_registry.contains(target_addr);

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
                    caller_is_stream,
                    callee_is_stream,
                },
            );
        }

        // Allocate media for this call: WebRTC INVITE or plain SIP.
        let rewritten_body = if should_preserve_webrtc_sdp_for_target(&contact_uri, &msg.body) {
            None
        } else if callee_is_stream
            && should_bridge_plain_sip_to_websocket_target(&contact_uri, &msg.body)
        {
            // SIP-phone-originated INVITE to browser callee:
            // create reverse WebRTC bridge and send browser-compatible offer.
            match self
                .webrtc_gateway
                .create_session_for_sip_caller(call_id.clone())
                .await
            {
                Ok((webrtc_offer_sdp, sip_port)) => {
                    if let Some(sip_addr) = sdp_rtp_addr(&msg.body) {
                        self.webrtc_gateway.set_sip_peer(&call_id, sip_addr).await;
                    }
                    info!(
                        "Reverse WebRTC session for {}: forwarding browser offer (sip port {})",
                        call_id, sip_port
                    );
                    Some(webrtc_offer_sdp)
                }
                Err(e) => {
                    warn!(
                        "Reverse WebRTC session creation failed for {}: {}",
                        call_id, e
                    );
                    None
                }
            }
        } else if !msg.body.is_empty() && is_webrtc_sdp(&msg.body) {
            // Browser-originated WebRTC INVITE: create a WebRTC session and
            // replace the WebRTC SDP with a plain RTP offer for the SIP phone.
            match self
                .webrtc_gateway
                .create_session(call_id.clone(), &msg.body)
                .await
            {
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
        let target_uri = registered_target_uri(&callee, &contact_uri, target_addr);
        let forwarded = self.build_forwarded_invite(msg, &target_uri, max_fwd - 1, rewritten_body);
        self.send_sip(forwarded, target_addr).await?;

        info!(
            "Proxied INVITE from {} to {} at {}",
            caller, callee, target_addr
        );

        Ok(base_response(msg, 100, "Trying").build())
    }

    async fn send_sip(&self, message: String, target: SocketAddr) -> Result<()> {
        if !self.transport_registry.send(target, message.clone()) {
            self.socket.send_to(message.as_bytes(), target).await?;
        }
        Ok(())
    }

    async fn refresh_sender_registration_source(
        &self,
        username: &str,
        domain: &str,
        src: SocketAddr,
    ) -> Result<()> {
        let row: Option<(String, u16)> = sqlx::query_as(
            "SELECT source_ip, source_port FROM sip_registrations
             WHERE username = ? AND domain = ? AND expires_at > NOW()",
        )
        .bind(username)
        .bind(domain)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((registered_ip, registered_port)) = row
            && should_refresh_registration_source(&registered_ip, registered_port, src)
        {
            sqlx::query(
                "UPDATE sip_registrations
                 SET source_port = ?
                 WHERE username = ? AND domain = ? AND source_ip = ? AND expires_at > NOW()",
            )
            .bind(src.port())
            .bind(username)
            .bind(domain)
            .bind(&registered_ip)
            .execute(&self.pool)
            .await?;

            info!(
                "Refreshed registration source port for {}@{}: {}:{} -> {}:{}",
                username,
                domain,
                registered_ip,
                registered_port,
                registered_ip,
                src.port()
            );
        }

        Ok(())
    }

    async fn lookup_registered_target(&self, username: &str, domain: &str) -> Option<SocketAddr> {
        let row: Option<(String, u16)> = sqlx::query_as(
            "SELECT source_ip, source_port FROM sip_registrations
             WHERE username = ? AND domain = ? AND expires_at > NOW()",
        )
        .bind(username)
        .bind(domain)
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten();

        row.and_then(|(ip, port)| format!("{}:{}", ip, port).parse::<SocketAddr>().ok())
    }

    async fn persist_message(
        &self,
        msg: &SipMessage,
        sender: &str,
        receiver: &str,
        status: &str,
        source_ip: &str,
    ) {
        let message_id = msg.header("message-id");
        let call_id = msg.call_id();
        let content_type = msg.header("content-type").unwrap_or("text/plain");
        let delivered_at = if status == "delivered" {
            Some(Utc::now().naive_utc())
        } else {
            None
        };

        if let Err(e) = sqlx::query(
            "INSERT INTO sip_messages
                (message_id, call_id, sender, receiver, content_type, body, status, source_ip, created_at, delivered_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, NOW(), ?)",
        )
        .bind(message_id)
        .bind(call_id)
        .bind(sender)
        .bind(receiver)
        .bind(content_type)
        .bind(&msg.body)
        .bind(status)
        .bind(source_ip)
        .bind(delivered_at)
        .execute(&self.pool)
        .await
        {
            warn!("Failed to persist MESSAGE {} -> {}: {}", sender, receiver, e);
        }
    }

    fn build_forwarded_invite(
        &self,
        msg: &SipMessage,
        target_uri: &str,
        max_fwd: u32,
        rewritten_body: Option<String>,
    ) -> String {
        build_forwarded_invite_for_target(
            msg,
            target_uri,
            max_fwd,
            &self.cfg.server.sip_domain,
            rewritten_body.as_deref(),
        )
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
            let _ = self.send_sip(msg.raw.clone(), target).await;
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

                if let Some((ip, port)) = row
                    && let Ok(target) = format!("{}:{}", ip, port).parse::<SocketAddr>()
                {
                    let _ = self.send_sip(msg.raw.clone(), target).await;
                    info!("Forwarded ACK (fallback) to {} at {}", callee, target);
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
            let _ = self.send_sip(msg.raw.clone(), target).await;
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

                if let Some((ip, port)) = row
                    && let Ok(fallback) = format!("{}:{}", ip, port).parse::<SocketAddr>()
                {
                    let _ = self.send_sip(msg.raw.clone(), fallback).await;
                    info!("Forwarded BYE (fallback) to {} at {}", callee, fallback);
                }
            }
        }

        info!("Call ended: {}", call_id);
        Ok(base_response(msg, 200, "OK").build())
    }

    pub async fn handle_cancel(&self, msg: &SipMessage, _src: SocketAddr) -> Result<String> {
        let call_id = msg.call_id().unwrap_or("").to_string();
        let domain = self.cfg.server.sip_domain.clone();
        let max_fwd = msg
            .header("max-forwards")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(70);

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
            let target_ip = target.ip().to_string();
            let target_port = target.port();
            let target_uri = sqlx::query_as::<_, (String,)>(
                "SELECT contact_uri FROM sip_registrations
                 WHERE source_ip = ? AND source_port = ? AND expires_at > NOW()
                 LIMIT 1",
            )
            .bind(&target_ip)
            .bind(target_port)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten()
            .map(|(uri,)| uri)
            .or_else(|| msg.request_uri.clone())
            .unwrap_or_else(|| {
                format!(
                    "sip:{}@{}",
                    uri_username(msg.request_uri.as_deref().unwrap_or("")).unwrap_or_default(),
                    domain
                )
            });

            let forwarded = build_forwarded_cancel_for_target(
                msg,
                &target_uri,
                max_fwd.saturating_sub(1),
                &domain,
            );
            let _ = self.send_sip(forwarded, target).await;
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
                let _ = self.send_sip(msg.raw.clone(), target).await;
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
            let _ = self.send_sip(msg.raw.clone(), target).await;
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
            let _ = self.send_sip(msg.raw.clone(), target).await;
            info!("Forwarded INFO for call {} to {}", call_id, target);
        } else {
            warn!("No active dialog for INFO call_id: {}", call_id);
        }

        Ok(base_response(msg, 200, "OK").build())
    }

    pub async fn handle_message(&self, msg: &SipMessage, src: SocketAddr) -> Result<String> {
        let domain = self.cfg.server.sip_domain.clone();
        let sender = msg
            .from_header()
            .and_then(extract_uri)
            .and_then(|u| uri_username(&u))
            .unwrap_or_default();
        if sender.is_empty() {
            warn!("MESSAGE with no sender from {}", src);
            return Ok(base_response(msg, 400, "Bad Request").build());
        }
        let sender_ok: Option<(i32,)> = sqlx::query_as(MESSAGE_SENDER_ACCOUNT_EXISTS_SQL)
            .bind(&sender)
            .bind(&domain)
            .fetch_optional(&self.pool)
            .await?;
        if sender_ok.is_none() {
            warn!("MESSAGE from unrecognised sender: {}@{}", sender, domain);
            return Ok(base_response(msg, 403, "Forbidden").build());
        }
        self.refresh_sender_registration_source(&sender, &domain, src)
            .await?;

        let fallback_callee = msg
            .to_header()
            .and_then(extract_uri)
            .and_then(|u| uri_username(&u))
            .unwrap_or_default();
        let callee = msg
            .request_uri
            .as_deref()
            .and_then(uri_username)
            .filter(|u| !u.is_empty())
            .unwrap_or(fallback_callee);
        if callee.is_empty() {
            warn!("MESSAGE with no callee from {}", src);
            return Ok(base_response(msg, 400, "Bad Request").build());
        }

        let sender_aor = format!("{}@{}", sender, domain);
        let callee_aor = format!("{}@{}", callee, domain);
        let call_id = msg.call_id().unwrap_or("").to_string();

        let dialog_target = if call_id.is_empty() {
            None
        } else {
            let active = self.active_dialogs.lock().await;
            active.get(&call_id).map(|d| {
                if src == d.caller_addr {
                    d.callee_addr
                } else {
                    d.caller_addr
                }
            })
        };

        let target = match dialog_target {
            Some(addr) => Some(addr),
            None => self.lookup_registered_target(&callee, &domain).await,
        };

        let source_ip = src.ip().to_string();
        if let Some(target_addr) = target {
            if let Err(e) = self.send_sip(msg.raw.clone(), target_addr).await {
                self.persist_message(msg, &sender_aor, &callee_aor, "failed", &source_ip)
                    .await;
                warn!(
                    "Failed to forward MESSAGE {} -> {}: {}",
                    sender_aor, callee_aor, e
                );
                return Ok(base_response(msg, 500, "Internal Server Error").build());
            }
            self.persist_message(msg, &sender_aor, &callee_aor, "delivered", &source_ip)
                .await;
            info!(
                "Forwarded MESSAGE {} -> {} at {}",
                sender_aor, callee_aor, target_addr
            );
            Ok(base_response(msg, 200, "OK").build())
        } else {
            self.persist_message(msg, &sender_aor, &callee_aor, "failed", &source_ip)
                .await;
            warn!("MESSAGE target offline: {} -> {}", sender_aor, callee_aor);
            Ok(base_response(msg, 404, "Not Found").build())
        }
    }
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
