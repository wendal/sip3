use anyhow::{Result, anyhow};
use sqlx::MySqlPool;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

use super::conference::Conference;
use super::conference_media::ConferenceMedia;
use super::media::{
    MediaRelay, is_invite_200_ok_with_sdp, rewrite_content_length, rewrite_sdp_media, sdp_rtp_addr,
};
use super::presence::Presence;
use super::proxy::Proxy;
use super::registrar::Registrar;
use super::transport::TransportRegistry;
use super::voicemail::{NoAnswerTimerCancel, Voicemail, is_message_summary_event};
use super::voicemail_media::VoicemailMedia;
use super::voicemail_mwi::VoicemailMwi;
use super::webrtc_gateway::WebRtcGateway;
use crate::config::Config;
use crate::security_guard::{GuardLimits, SecurityGuard};

/// Shared map from SIP Call-ID to the caller's address, used to relay
/// provisional/final responses from the callee back to the caller.
pub type PendingDialogs = Arc<tokio::sync::Mutex<HashMap<String, SocketAddr>>>;

/// State for an established SIP dialog (post-ACK), used to route in-dialog
/// requests (BYE, INFO) in both directions.
#[derive(Debug, Clone, Copy)]
pub struct DialogInfo {
    pub caller_addr: SocketAddr,
    pub callee_addr: SocketAddr,
    pub caller_is_stream: bool,
    pub callee_is_stream: bool,
}

/// Shared map from SIP Call-ID to established dialog info.
pub type ActiveDialogs = Arc<tokio::sync::Mutex<HashMap<String, DialogInfo>>>;

pub const SIP_ALLOW_METHODS: &str =
    "REGISTER, INVITE, ACK, BYE, CANCEL, OPTIONS, INFO, REFER, NOTIFY, SUBSCRIBE, MESSAGE";

#[derive(Clone)]
pub struct DialogStores {
    pub pending: PendingDialogs,
    pub active: ActiveDialogs,
}

/// Parsed SIP request or response
#[derive(Debug, Clone)]
pub struct SipMessage {
    pub method: Option<String>,
    pub request_uri: Option<String>,
    pub status_code: Option<u16>,
    pub headers: HashMap<String, Vec<String>>,
    pub body: String,
    pub raw: String,
}

impl SipMessage {
    pub fn parse(raw: &str) -> Result<Self> {
        let mut lines = raw.lines();
        let first_line = lines.next().ok_or_else(|| anyhow!("Empty SIP message"))?;

        let mut method = None;
        let mut request_uri = None;
        let mut status_code = None;

        let parts: Vec<&str> = first_line.splitn(3, ' ').collect();
        if parts.len() < 2 {
            return Err(anyhow!("Invalid SIP first line: {}", first_line));
        }

        if first_line.starts_with("SIP/2.0") {
            status_code = parts.get(1).and_then(|s| s.parse::<u16>().ok());
        } else {
            method = Some(parts[0].to_string());
            request_uri = parts.get(1).map(|s| s.to_string());
        }

        let mut headers: HashMap<String, Vec<String>> = HashMap::new();
        let mut in_headers = true;
        let mut body_lines: Vec<&str> = Vec::new();
        let mut last_key: Option<String> = None;

        for line in lines {
            if in_headers {
                if line.is_empty() {
                    in_headers = false;
                    continue;
                }
                if line.starts_with(' ') || line.starts_with('\t') {
                    if let Some(key) = &last_key
                        && let Some(vals) = headers.get_mut(key)
                        && let Some(last) = vals.last_mut()
                    {
                        last.push(' ');
                        last.push_str(line.trim());
                    }
                } else if let Some(colon_pos) = line.find(':') {
                    let name = normalize_header_name(&line[..colon_pos]);
                    let value = line[colon_pos + 1..].trim().to_string();
                    headers.entry(name.clone()).or_default().push(value);
                    last_key = Some(name);
                }
            } else {
                body_lines.push(line);
            }
        }

        Ok(SipMessage {
            method,
            request_uri,
            status_code,
            headers,
            body: body_lines.join("\r\n"),
            raw: raw.to_string(),
        })
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        let key = normalize_header_name(name);
        self.headers
            .get(&key)
            .and_then(|v| v.first())
            .map(|s| s.as_str())
    }

    pub fn all_headers_vec(&self, name: &str) -> Vec<&str> {
        let key = normalize_header_name(name);
        self.headers
            .get(&key)
            .map(|v| v.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    pub fn call_id(&self) -> Option<&str> {
        self.header("call-id")
    }
    #[allow(clippy::wrong_self_convention)]
    pub fn from_header(&self) -> Option<&str> {
        self.header("from")
    }
    pub fn to_header(&self) -> Option<&str> {
        self.header("to")
    }
    pub fn contact(&self) -> Option<&str> {
        self.header("contact")
    }
    pub fn via_headers(&self) -> Vec<&str> {
        self.all_headers_vec("via")
    }
    pub fn cseq(&self) -> Option<&str> {
        self.header("cseq")
    }
    pub fn expires(&self) -> Option<u32> {
        self.header("expires").and_then(|s| s.parse().ok())
    }
    pub fn user_agent(&self) -> Option<&str> {
        self.header("user-agent")
    }
    pub fn authorization(&self) -> Option<&str> {
        self.header("authorization")
    }
}

pub fn normalize_header_name(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    match lower.as_str() {
        "f" => "from".to_string(),
        "t" => "to".to_string(),
        "v" => "via".to_string(),
        "c" => "content-type".to_string(),
        "l" => "content-length".to_string(),
        "i" => "call-id".to_string(),
        "m" => "contact".to_string(),
        "e" => "content-encoding".to_string(),
        other => other.to_string(),
    }
}

/// Build a SIP response string
pub struct SipResponseBuilder {
    status_code: u16,
    reason: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl SipResponseBuilder {
    pub fn new(status_code: u16, reason: &str) -> Self {
        Self {
            status_code,
            reason: reason.to_string(),
            headers: Vec::new(),
            body: String::new(),
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn body(mut self, body: &str) -> Self {
        self.body = body.to_string();
        self
    }

    pub fn build(self) -> String {
        let content_len = self.body.len();
        let mut msg = format!("SIP/2.0 {} {}\r\n", self.status_code, self.reason);
        for (name, value) in &self.headers {
            msg.push_str(&format!("{}: {}\r\n", name, value));
        }
        msg.push_str(&format!("Content-Length: {}\r\n", content_len));
        msg.push_str("\r\n");
        msg.push_str(&self.body);
        msg
    }
}

/// Copy standard headers from request into a response builder
pub fn base_response(req: &SipMessage, status_code: u16, reason: &str) -> SipResponseBuilder {
    let mut builder = SipResponseBuilder::new(status_code, reason);

    for via in req.via_headers() {
        builder = builder.header("Via", via);
    }
    if let Some(from) = req.from_header() {
        builder = builder.header("From", from);
    }
    if let Some(to) = req.to_header() {
        builder = builder.header("To", to);
    }
    if let Some(call_id) = req.call_id() {
        builder = builder.header("Call-ID", call_id);
    }
    if let Some(cseq) = req.cseq() {
        builder = builder.header("CSeq", cseq);
    }
    builder.header("Server", "SIP3/0.1.0")
}

/// Extract URI from a SIP address like "Name" <sip:user@host> or sip:user@host
pub fn extract_uri(addr: &str) -> Option<String> {
    if let Some(start) = addr.find('<')
        && let Some(end_rel) = addr[start..].find('>')
    {
        return Some(addr[start + 1..start + end_rel].trim().to_string());
    }
    let uri = addr.split(';').next().unwrap_or(addr).trim();
    if uri.starts_with("sip:") || uri.starts_with("sips:") || uri.starts_with("tel:") {
        Some(uri.to_string())
    } else {
        None
    }
}

/// Extract username from a SIP URI (sip:user@host)
pub fn uri_username(uri: &str) -> Option<String> {
    let without_scheme = uri.trim_start_matches("sip:").trim_start_matches("sips:");
    if without_scheme.contains('@') {
        Some(without_scheme.split('@').next()?.to_string())
    } else {
        None
    }
}

/// Extract host from a SIP URI
#[allow(dead_code)]
pub fn uri_host(uri: &str) -> Option<String> {
    let without_scheme = uri.trim_start_matches("sip:").trim_start_matches("sips:");
    let part = if without_scheme.contains('@') {
        without_scheme.split('@').nth(1).unwrap_or(without_scheme)
    } else {
        without_scheme
    };
    let host = part.split(';').next().unwrap_or(part);
    let host = host.split('?').next().unwrap_or(host);
    Some(host.to_string())
}

/// Parse auth parameters from an Authorization or WWW-Authenticate header value
/// e.g. Digest username="alice", realm="example.com", nonce="abc", ...
pub fn parse_auth_params(header: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    let rest = match header.find(' ') {
        Some(pos) => header[pos + 1..].trim(),
        None => return params,
    };

    // Simple state-machine parser to handle quoted values
    let mut pos = 0;
    let bytes = rest.as_bytes();
    let len = bytes.len();

    while pos < len {
        // Skip leading whitespace/commas
        while pos < len && (bytes[pos] == b',' || bytes[pos] == b' ' || bytes[pos] == b'\t') {
            pos += 1;
        }
        if pos >= len {
            break;
        }

        // Read key
        let key_start = pos;
        while pos < len && bytes[pos] != b'=' {
            pos += 1;
        }
        if pos >= len {
            break;
        }
        let key = rest[key_start..pos].trim().to_lowercase();
        pos += 1; // skip '='

        // Read value
        if pos < len && bytes[pos] == b'"' {
            pos += 1; // skip opening quote
            let val_start = pos;
            while pos < len && bytes[pos] != b'"' {
                if bytes[pos] == b'\\' {
                    pos += 1;
                } // escaped char
                pos += 1;
            }
            let val = rest[val_start..pos].to_string();
            if pos < len {
                pos += 1;
            } // skip closing quote
            params.insert(key, val);
        } else {
            let val_start = pos;
            while pos < len && bytes[pos] != b',' {
                pos += 1;
            }
            let val = rest[val_start..pos].trim().to_string();
            params.insert(key, val);
        }
    }

    params
}

/// Build a WWW-Authenticate challenge header value
pub fn make_www_authenticate(realm: &str, nonce: &str) -> String {
    format!(
        r#"Digest realm="{}", nonce="{}", algorithm=MD5, qop="auth""#,
        realm, nonce
    )
}

/// Compute MD5 hex digest
pub fn md5_hex(input: &str) -> String {
    format!("{:x}", md5::compute(input.as_bytes()))
}

/// Strip the first Via header that contains `sip_domain` (i.e., the one we added
/// when proxying an INVITE), so the relayed response looks correct to the caller.
pub fn strip_proxy_via(raw: &str, sip_domain: &str) -> String {
    let mut removed = false;
    let mut lines: Vec<&str> = Vec::new();
    for line in raw.split("\r\n") {
        if !removed {
            let lower = line.trim_start().to_lowercase();
            if (lower.starts_with("via:") || lower.starts_with("v:")) && line.contains(sip_domain) {
                removed = true;
                continue;
            }
        }
        lines.push(line);
    }
    lines.join("\r\n")
}

fn finalize_response(
    msg: &SipMessage,
    response: Result<String>,
    method: &str,
) -> Result<Option<String>> {
    match response {
        Ok(resp) => Ok(Some(resp)),
        Err(e) => {
            warn!("Error handling {}: {}", method, e);
            Ok(Some(
                base_response(msg, 500, "Internal Server Error").build(),
            ))
        }
    }
}

#[derive(Clone)]
pub struct SipHandler {
    cfg: Config,
    socket: Arc<UdpSocket>,
    registrar: Registrar,
    proxy: Proxy,
    pending_dialogs: PendingDialogs,
    active_dialogs: ActiveDialogs,
    media_relay: MediaRelay,
    presence: Presence,
    webrtc_gateway: Arc<WebRtcGateway>,
    transport_registry: TransportRegistry,
    conference: Conference,
    voicemail: Voicemail,
}

impl SipHandler {
    pub fn with_socket(cfg: Config, pool: MySqlPool, socket: Arc<UdpSocket>) -> Self {
        let pending_dialogs: PendingDialogs = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let active_dialogs: ActiveDialogs = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let dialog_stores = DialogStores {
            pending: pending_dialogs.clone(),
            active: active_dialogs.clone(),
        };
        let transport_registry = TransportRegistry::default();
        let media_relay = MediaRelay::new(
            cfg.server.public_ip.clone(),
            cfg.server.rtp_port_min,
            cfg.server.rtp_port_max,
        );
        let webrtc_gateway = Arc::new(WebRtcGateway::new(
            cfg.server.public_ip.clone(),
            cfg.server.rtp_port_min,
            cfg.server.rtp_port_max,
        ));
        let security_guard = Arc::new(tokio::sync::Mutex::new(SecurityGuard::new(GuardLimits {
            window_secs: cfg.security.window_secs,
            ip_fail_threshold: cfg.security.sip_ip_fail_threshold as usize,
            user_ip_fail_threshold: cfg.security.sip_user_ip_fail_threshold as usize,
            block_secs: cfg.security.block_secs,
        })));
        let presence = Presence::new(pool.clone(), cfg.clone(), socket.clone());
        let voicemail_media = VoicemailMedia::new(
            cfg.server.public_ip.clone(),
            cfg.server.voicemail_rtp_port_min,
            cfg.server.voicemail_rtp_port_max,
        );
        let voicemail_mwi = VoicemailMwi::new(pool.clone(), cfg.clone(), socket.clone());
        let voicemail = Voicemail::new(pool.clone(), cfg.clone(), voicemail_media, voicemail_mwi);
        let registrar = Registrar::new(
            pool.clone(),
            cfg.clone(),
            presence.clone(),
            security_guard.clone(),
        );
        let proxy = Proxy::new(
            pool.clone(),
            cfg.clone(),
            socket.clone(),
            dialog_stores,
            media_relay.clone(),
            webrtc_gateway.clone(),
            transport_registry.clone(),
            voicemail.clone(),
        );
        let conference_media = ConferenceMedia::new(
            cfg.server.public_ip.clone(),
            cfg.server.conference_rtp_port_min,
            cfg.server.conference_rtp_port_max,
        );
        let conference = Conference::new(pool, cfg.clone(), conference_media);
        Self {
            cfg,
            socket,
            registrar,
            proxy,
            pending_dialogs,
            active_dialogs,
            media_relay,
            presence,
            webrtc_gateway,
            transport_registry,
            conference,
            voicemail,
        }
    }

    /// Expose the media relay so callers (e.g. the server loop) can schedule
    /// background cleanup without requiring a separate reference.
    pub fn media_relay(&self) -> &MediaRelay {
        &self.media_relay
    }

    /// Expose the WebRTC gateway for background cleanup.
    pub fn webrtc_gateway(&self) -> &Arc<WebRtcGateway> {
        &self.webrtc_gateway
    }

    /// Expose the conference service for startup reconciliation tasks.
    pub fn conference(&self) -> &Conference {
        &self.conference
    }

    pub fn voicemail(&self) -> &Voicemail {
        &self.voicemail
    }

    pub fn register_stream(&self, src: SocketAddr) -> tokio::sync::mpsc::UnboundedReceiver<String> {
        self.transport_registry.register(src)
    }

    pub fn unregister_stream(&self, src: SocketAddr) {
        self.transport_registry.unregister(src);
    }

    async fn send_to_addr(&self, message: String, addr: SocketAddr) -> Result<()> {
        if !self.transport_registry.send(addr, message.clone()) {
            self.socket.send_to(message.as_bytes(), addr).await?;
        }
        Ok(())
    }

    pub async fn handle_datagram(&self, data: Vec<u8>, src: SocketAddr) -> Result<()> {
        let raw = String::from_utf8_lossy(&data).to_string();
        debug!("Received {} bytes from {}", data.len(), src);
        if let Some(resp) = self.process_sip_msg(&raw, src).await? {
            self.socket.send_to(resp.as_bytes(), src).await?;
        }
        Ok(())
    }

    /// Process a raw SIP message (from any transport) and return a response string
    /// if one should be sent, or None for ACKs, relayed responses, and parse errors.
    pub async fn process_sip_msg(&self, raw: &str, src: SocketAddr) -> Result<Option<String>> {
        let msg = match SipMessage::parse(raw) {
            Ok(m) => m,
            Err(e) => {
                warn!("Failed to parse SIP message from {}: {}", src, e);
                return Ok(None);
            }
        };

        let method = match &msg.method {
            Some(m) => m.clone(),
            None => {
                // SIP response from callee — relay back to the original caller.
                self.relay_response(&msg).await;
                return Ok(None);
            }
        };

        info!("SIP {} from {}", method, src);

        // Route local voicemail traffic before conference and generic proxy handling.
        let call_id_str = msg.call_id().unwrap_or("").to_string();
        let is_vm = !call_id_str.is_empty() && self.voicemail.is_voicemail_call(&call_id_str).await;

        if method == "SUBSCRIBE" && is_message_summary_event(msg.header("event")) {
            let resp = self.voicemail.mwi().handle_subscribe(&msg, src).await;
            return finalize_response(&msg, resp, &method);
        }

        if method == "INVITE" && self.voicemail.is_access_invite(&msg) {
            let resp = self.voicemail.handle_access_invite(&msg, src).await;
            return finalize_response(&msg, resp, &method);
        }

        if is_vm {
            match method.as_str() {
                "ACK" => {
                    self.voicemail.handle_ack(&msg).await;
                    return Ok(None);
                }
                "BYE" => {
                    let resp = self.voicemail.handle_bye(&msg).await;
                    return finalize_response(&msg, resp, &method);
                }
                "CANCEL" => {
                    let resp = self.voicemail.handle_cancel(&msg).await;
                    return finalize_response(&msg, resp, &method);
                }
                "INFO" => {
                    let resp = self.voicemail.handle_info(&msg).await;
                    return finalize_response(&msg, resp, &method);
                }
                _ => {}
            }
        }

        // Route conference traffic before generic proxy handling.
        let is_conf =
            !call_id_str.is_empty() && self.conference.is_conference_call(&call_id_str).await;

        if method == "INVITE" {
            let req_uri = msg.request_uri.as_deref().unwrap_or("");
            let target = uri_username(req_uri).unwrap_or_default();
            let domain = self.cfg.server.sip_domain.clone();
            if let Some((room_id, max_p)) = self.conference.lookup_room(&target, &domain).await {
                let resp = self
                    .conference
                    .handle_invite(&msg, src, room_id, max_p)
                    .await;
                return finalize_response(&msg, resp, &method);
            }
        } else if is_conf {
            match method.as_str() {
                "ACK" => {
                    self.conference.handle_ack(&msg).await;
                    return Ok(None);
                }
                "BYE" => {
                    let resp = self.conference.handle_bye(&msg).await;
                    return finalize_response(&msg, resp, &method);
                }
                "CANCEL" => {
                    let resp = self.conference.handle_cancel(&msg).await;
                    return finalize_response(&msg, resp, &method);
                }
                "INFO" => {
                    let resp = self.conference.handle_info(&msg).await;
                    return finalize_response(&msg, resp, &method);
                }
                _ => {}
            }
        }

        let response = match method.as_str() {
            "REGISTER" => self.registrar.handle_register(&msg, src).await,
            "INVITE" => self.proxy.handle_invite(&msg, src).await,
            "OPTIONS" => self.handle_options(&msg),
            "INFO" => self.proxy.handle_info(&msg, src).await,
            "MESSAGE" => self.proxy.handle_message(&msg, src).await,
            "REFER" => self.proxy.handle_refer(&msg, src).await,
            "NOTIFY" => self.proxy.handle_notify(&msg, src).await,
            "SUBSCRIBE" => self.presence.handle_subscribe(&msg, src).await,
            "ACK" => {
                self.proxy.handle_ack(&msg, src).await?;
                return Ok(None);
            }
            "BYE" => self.proxy.handle_bye(&msg, src).await,
            "CANCEL" => self.proxy.handle_cancel(&msg, src).await,
            _ => {
                warn!("Unsupported SIP method: {}", method);
                Ok(base_response(&msg, 405, "Method Not Allowed")
                    .header("Allow", SIP_ALLOW_METHODS)
                    .build())
            }
        };

        match response {
            Ok(resp) => Ok(Some(resp)),
            Err(e) => {
                warn!("Error handling {}: {}", method, e);
                let err_resp = base_response(&msg, 500, "Internal Server Error").build();
                Ok(Some(err_resp))
            }
        }
    }

    /// Handle a SIP message received over TCP/TLS. Returns the response to send
    /// back on the same connection, or None if no reply is needed.
    pub async fn handle_tcp_msg(&self, raw: &str, src: SocketAddr) -> Result<Option<String>> {
        self.process_sip_msg(raw, src).await
    }

    /// Relay a SIP response (e.g. 180, 200) from the callee back to the original caller.
    async fn relay_response(&self, msg: &SipMessage) {
        let call_id = match msg.call_id() {
            Some(id) => id.to_string(),
            None => {
                debug!("Dropping SIP response with no Call-ID");
                return;
            }
        };

        if msg.status_code.is_some_and(|c| c >= 200)
            && self.voicemail.cancel_no_answer_timer(&call_id).await
                == NoAnswerTimerCancel::AlreadyFired
        {
            self.pending_dialogs.lock().await.remove(&call_id);
            self.active_dialogs.lock().await.remove(&call_id);
            self.media_relay.remove_session(&call_id).await;
            self.webrtc_gateway.remove_session(&call_id).await;
            debug!(
                "Dropping final response for {} because no-answer voicemail already won",
                call_id
            );
            return;
        }

        let caller_addr = {
            let dialogs = self.pending_dialogs.lock().await;
            dialogs.get(&call_id).copied()
        };

        if let Some(addr) = caller_addr {
            let stream_to_stream = {
                let active = self.active_dialogs.lock().await;
                active
                    .get(&call_id)
                    .map(|d| d.caller_is_stream && d.callee_is_stream)
                    .unwrap_or(false)
            };
            // Strip the Via we added when forwarding the INVITE.
            let relayed = strip_proxy_via(&msg.raw, &self.cfg.server.sip_domain);

            // On a 200 OK to an INVITE that carries SDP, substitute the appropriate SDP.
            // If this is a WebRTC call, use the stored WebRTC answer SDP.
            // Otherwise rewrite the body so the caller sends RTP to our relay_b port.
            let relayed = if is_invite_200_ok_with_sdp(msg) && !stream_to_stream {
                if let Some(answer_sdp) = self.webrtc_gateway.get_answer_sdp(&call_id).await {
                    if self.webrtc_gateway.is_sip_caller_session(&call_id).await {
                        if let Err(e) = self
                            .webrtc_gateway
                            .apply_callee_answer(&call_id, &msg.body)
                            .await
                        {
                            warn!(
                                "WebRTC gw: failed to apply callee answer for {}: {}",
                                call_id, e
                            );
                        }
                    } else if let Some(sip_rtp_addr) = sdp_rtp_addr(&msg.body) {
                        // Browser-originated call: learn SIP RTP peer from 200 OK.
                        self.webrtc_gateway
                            .set_sip_peer(&call_id, sip_rtp_addr)
                            .await;
                    }
                    rewrite_content_length(&relayed, &answer_sdp)
                } else if let Some(new_sdp) = self.rewrite_200ok_sdp(&call_id, &msg.body).await {
                    rewrite_content_length(&relayed, &new_sdp)
                } else {
                    relayed
                }
            } else {
                relayed
            };

            if let Err(e) = self.send_to_addr(relayed, addr).await {
                warn!("Failed to relay response to {}: {}", addr, e);
            } else {
                debug!(
                    "Relayed {} response for call {} to caller at {}",
                    msg.status_code.unwrap_or(0),
                    call_id,
                    addr
                );
            }
            // Clean up the dialog entry for final responses (>= 200).
            if msg.status_code.is_some_and(|c| c >= 200) {
                self.pending_dialogs.lock().await.remove(&call_id);
            }
            // On non-2xx final responses, also remove the media session and WebRTC session.
            if msg.status_code.is_some_and(|c| c >= 300) {
                self.media_relay.remove_session(&call_id).await;
                self.webrtc_gateway.remove_session(&call_id).await;
            }
        } else {
            debug!(
                "No pending dialog for call-id {}, dropping response",
                call_id
            );
        }
    }

    /// Rewrite the body of a 200 OK response to an INVITE: replace the SDP
    /// `c=` and `m=audio` fields with the server's relay_b address so the caller
    /// directs its RTP to our relay port instead of the callee's private IP.
    async fn rewrite_200ok_sdp(&self, call_id: &str, sdp: &str) -> Option<String> {
        let sessions = self.media_relay.sessions.lock().await;
        let session = sessions.get(call_id)?;
        let caller_ports = session.caller_sdp_ports();
        let public_ip = self.media_relay.public_ip.as_str();
        let new_sdp = rewrite_sdp_media(sdp, public_ip, &caller_ports);
        info!(
            "Rewrote 200 OK SDP for {} with {} relayed media streams",
            call_id,
            caller_ports.len()
        );
        Some(new_sdp)
    }

    fn handle_options(&self, msg: &SipMessage) -> Result<String> {
        Ok(base_response(msg, 200, "OK")
            .header("Allow", SIP_ALLOW_METHODS)
            .header("Accept", "application/sdp, text/plain")
            .build())
    }
}
