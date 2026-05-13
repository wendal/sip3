use anyhow::{anyhow, Result};
use sqlx::MySqlPool;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

use super::proxy::Proxy;
use super::registrar::Registrar;
use crate::config::Config;

/// Shared map from SIP Call-ID to the caller's address, used to relay
/// provisional/final responses from the callee back to the caller.
pub type PendingDialogs = Arc<tokio::sync::Mutex<HashMap<String, SocketAddr>>>;

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
                    if let Some(key) = &last_key {
                        if let Some(vals) = headers.get_mut(key) {
                            if let Some(last) = vals.last_mut() {
                                last.push(' ');
                                last.push_str(line.trim());
                            }
                        }
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
    if let Some(start) = addr.find('<') {
        if let Some(end_rel) = addr[start..].find('>') {
            return Some(addr[start + 1..start + end_rel].trim().to_string());
        }
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

#[derive(Clone)]
pub struct SipHandler {
    cfg: Config,
    socket: Arc<UdpSocket>,
    registrar: Registrar,
    proxy: Proxy,
    pending_dialogs: PendingDialogs,
}

impl SipHandler {
    pub fn with_socket(cfg: Config, pool: MySqlPool, socket: Arc<UdpSocket>) -> Self {
        let pending_dialogs: PendingDialogs = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let registrar = Registrar::new(pool.clone(), cfg.clone());
        let proxy = Proxy::new(pool, cfg.clone(), socket.clone(), pending_dialogs.clone());
        Self {
            cfg,
            socket,
            registrar,
            proxy,
            pending_dialogs,
        }
    }

    pub async fn handle_datagram(&self, data: Vec<u8>, src: SocketAddr) -> Result<()> {
        let raw = String::from_utf8_lossy(&data).to_string();
        debug!("Received {} bytes from {}", data.len(), src);

        let msg = match SipMessage::parse(&raw) {
            Ok(m) => m,
            Err(e) => {
                warn!("Failed to parse SIP message from {}: {}", src, e);
                return Ok(());
            }
        };

        let method = match &msg.method {
            Some(m) => m.clone(),
            None => {
                // SIP response from callee — relay back to the original caller.
                self.relay_response(&msg).await;
                return Ok(());
            }
        };

        info!("SIP {} from {}", method, src);

        let response = match method.as_str() {
            "REGISTER" => self.registrar.handle_register(&msg, src).await,
            "INVITE" => self.proxy.handle_invite(&msg, src).await,
            "OPTIONS" => self.handle_options(&msg),
            "ACK" => {
                self.proxy.handle_ack(&msg, src).await?;
                return Ok(());
            }
            "BYE" => self.proxy.handle_bye(&msg, src).await,
            "CANCEL" => self.proxy.handle_cancel(&msg, src).await,
            _ => {
                warn!("Unsupported SIP method: {}", method);
                Ok(base_response(&msg, 405, "Method Not Allowed")
                    .header("Allow", "REGISTER, INVITE, ACK, BYE, CANCEL, OPTIONS")
                    .build())
            }
        };

        match response {
            Ok(resp) => {
                self.socket.send_to(resp.as_bytes(), src).await?;
                debug!("Sent {} response to {}", method, src);
            }
            Err(e) => {
                warn!("Error handling {}: {}", method, e);
                let err_resp = base_response(&msg, 500, "Internal Server Error").build();
                let _ = self.socket.send_to(err_resp.as_bytes(), src).await;
            }
        }

        Ok(())
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

        let caller_addr = {
            let dialogs = self.pending_dialogs.lock().await;
            dialogs.get(&call_id).copied()
        };

        if let Some(addr) = caller_addr {
            // Strip the Via we added when forwarding the INVITE.
            let relayed = strip_proxy_via(&msg.raw, &self.cfg.server.sip_domain);
            if let Err(e) = self.socket.send_to(relayed.as_bytes(), addr).await {
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
        } else {
            debug!(
                "No pending dialog for call-id {}, dropping response",
                call_id
            );
        }
    }

    fn handle_options(&self, msg: &SipMessage) -> Result<String> {
        Ok(base_response(msg, 200, "OK")
            .header("Allow", "REGISTER, INVITE, ACK, BYE, CANCEL, OPTIONS")
            .header("Accept", "application/sdp")
            .build())
    }
}
