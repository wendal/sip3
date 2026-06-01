use anyhow::Result;
use sqlx::MySqlPool;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

use super::conference::Conference;
use super::conference_media::ConferenceMedia;
use super::media::{
    MediaRelay, is_invite_200_ok_with_sdp, rewrite_content_length, rewrite_sdp_media,
    sdp_connection_ip, sdp_rtp_addr, sdp_video_port,
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

pub use super::message::SIP_ALLOW_METHODS;
pub use super::message::{extract_uri, make_www_authenticate, md5_hex, normalize_header_name, parse_auth_params, strip_proxy_via, uri_username, uri_host, SipMessage};
pub use super::response::{base_response, finalize_response, SipResponseBuilder};

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

#[derive(Clone)]
pub struct DialogStores {
    pub pending: PendingDialogs,
    pub active: ActiveDialogs,
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
        let invite_guard = Arc::new(tokio::sync::Mutex::new(SecurityGuard::new(GuardLimits {
            window_secs: cfg.security.window_secs,
            ip_fail_threshold: cfg.security.sip_invite_ip_fail_threshold as usize,
            user_ip_fail_threshold: cfg.security.sip_invite_user_ip_fail_threshold as usize,
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
            invite_guard,
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

    pub(crate) fn transport_registry(&self) -> TransportRegistry {
        self.transport_registry.clone()
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
                info!(
                    "SIP response {} {} from {}",
                    msg.status_code.unwrap_or(0),
                    msg.cseq().unwrap_or(""),
                    src
                );
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
            if msg.status_code == Some(200) && msg.cseq().is_some_and(|c| c.ends_with("INVITE")) {
                info!(
                    "Handling 200 INVITE response for call {}: content-type={:?}, body_len={}",
                    call_id,
                    msg.header("content-type"),
                    msg.body.len()
                );
            }
            // Strip the Via we added when forwarding the INVITE.
            let relayed = strip_proxy_via(&msg.raw, &self.cfg.server.sip_domain);

            // On a 200 OK to an INVITE that carries SDP, substitute the appropriate SDP.
            // If this is a WebRTC call, use the stored WebRTC answer SDP.
            // Otherwise rewrite the body so the caller sends RTP to our relay_b port.
            let relayed = if is_invite_200_ok_with_sdp(msg) {
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
                        if let (Some(ip), Some(video_port)) =
                            (sdp_connection_ip(&msg.body), sdp_video_port(&msg.body))
                            && let Ok(video_addr) = format!("{}:{}", ip, video_port).parse()
                        {
                            self.webrtc_gateway
                                .set_sip_video_peer(&call_id, video_addr)
                                .await;
                        }
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
            if msg.status_code == Some(200) && msg.cseq().is_some_and(|c| c.ends_with("INVITE")) {
                warn!(
                    "Dropping 200 INVITE response for call {} because pending dialog is missing",
                    call_id
                );
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::MySqlPool;

    #[tokio::test]
    async fn relay_response_rewrites_200ok_sdp_for_stream_to_stream_plain_sip_calls() {
        let cfg = Config::load().expect("config should load");
        let pool =
            MySqlPool::connect_lazy("mysql://root:root@127.0.0.1:3306/sip3").expect("lazy pool");
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("bind udp"));
        let handler = SipHandler::with_socket(cfg.clone(), pool, socket);

        let call_id = format!("stream-to-stream-sdp-{}", std::process::id());
        let caller_addr: SocketAddr = "127.0.0.1:55060".parse().expect("caller addr");
        let callee_addr: SocketAddr = "127.0.0.1:55061".parse().expect("callee addr");
        let mut rx = handler.register_stream(caller_addr);

        let (_relay_a, relay_b) = handler
            .media_relay
            .allocate_session(call_id.clone())
            .await
            .expect("allocate media relay");

        handler
            .pending_dialogs
            .lock()
            .await
            .insert(call_id.clone(), caller_addr);
        handler.active_dialogs.lock().await.insert(
            call_id.clone(),
            DialogInfo {
                caller_addr,
                callee_addr,
                caller_is_stream: true,
                callee_is_stream: true,
            },
        );

        let raw_response = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP sip.air32.cn;branch=z9hG4bKproxy123\r\n\
             Via: SIP/2.0/TLS 192.0.2.10:5061;branch=z9hG4bKorig\r\n\
             From: <sip:1001@sip.air32.cn>;tag=caller\r\n\
             To: <sip:1003@sip.air32.cn>;tag=callee\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Contact: <sip:1003@192.168.31.27:56044;transport=tls>\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: 117\r\n\
             \r\n\
             v=0\r\n\
             o=- 0 0 IN IP4 192.168.31.27\r\n\
             s=-\r\n\
             c=IN IP4 192.168.31.27\r\n\
             t=0 0\r\n\
             m=audio 56044 RTP/AVP 0 8\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n"
        );
        let msg = SipMessage::parse(&raw_response).expect("parse 200 OK");

        handler.relay_response(&msg).await;

        let relayed = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("wait for relayed response")
            .expect("relayed response");

        assert!(
            relayed.contains(&format!("c=IN IP4 {}\r\n", cfg.server.public_ip)),
            "200 OK SDP should advertise relay public IP"
        );
        assert!(
            relayed.contains(&format!("m=audio {} RTP/AVP 0 8\r\n", relay_b)),
            "200 OK SDP should direct caller to relay_b"
        );

        handler.media_relay.remove_session(&call_id).await;
        handler.unregister_stream(caller_addr);
    }
}
