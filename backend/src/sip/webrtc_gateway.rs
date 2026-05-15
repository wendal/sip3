//! WebRTC media gateway — bridges browser WebRTC (ICE + DTLS-SRTP) with
//! legacy SIP phones (plain UDP RTP).
//!
//! # Media flow
//! ```text
//! Browser ─(ICE/DTLS-SRTP)─► RTCPeerConnection ─[on_track]─► sip_socket ──► SIP phone
//! SIP phone ─► sip_socket ──► local_track ──► RTCPeerConnection ─(DTLS-SRTP)─► Browser
//! ```

use super::media::make_plain_rtp_sdp;
use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MIME_TYPE_PCMA, MIME_TYPE_PCMU, MediaEngine};
use webrtc::api::setting_engine::SettingEngine;
use webrtc::ice_transport::ice_candidate_type::RTCIceCandidateType;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::{
    RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType,
};
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::{TrackLocal, TrackLocalWriter};
use webrtc::util::marshal::Marshal;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum BridgeFlow {
    /// Browser originated INVITE (WebRTC offer -> SIP answer).
    BrowserCaller,
    /// SIP phone originated INVITE (SIP offer -> browser WebRTC answer).
    SipCaller,
}

/// Per-call WebRTC session state.
struct WebRtcSession {
    pc: Arc<RTCPeerConnection>,
    flow: BridgeFlow,
    /// The SIP phone's RTP address (set from 200 OK SDP or learned from first packet).
    sip_peer: Arc<Mutex<Option<SocketAddr>>>,
    /// SDP to send back to the original SIP transaction caller in 200 OK.
    answer_sdp: String,
    created_at: Instant,
    /// Background task forwarding SIP phone RTP → local track → browser.
    sip_rx_task: JoinHandle<()>,
}

/// Shared WebRTC media gateway.
///
/// One `WebRtcSession` is created per browser-originated INVITE.
/// `create_session` performs the full WebRTC offer/answer negotiation and
/// returns the answer SDP + the port for the forwarded SIP INVITE.
#[derive(Clone)]
pub struct WebRtcGateway {
    sessions: Arc<Mutex<HashMap<String, WebRtcSession>>>,
    public_ip: String,
    /// Port range for SIP-side RTP sockets (shared with MediaRelay range).
    sip_port_min: u16,
    sip_port_max: u16,
    sip_port_counter: Arc<AtomicU32>,
}

impl WebRtcGateway {
    pub fn new(public_ip: String, sip_port_min: u16, sip_port_max: u16) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            public_ip,
            sip_port_min,
            sip_port_max,
            sip_port_counter: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Create a WebRTC session for a browser-originated INVITE.
    ///
    /// Returns `(webrtc_answer_sdp, sip_rtp_port)`:
    /// - `webrtc_answer_sdp`: the answer SDP to send back to the browser in 200 OK
    /// - `sip_rtp_port`: the port to include in the forwarded INVITE to the SIP phone
    pub async fn create_session(
        &self,
        call_id: String,
        browser_offer_sdp: &str,
    ) -> Result<(String, u16)> {
        let (pc, sip_peer, sip_rx_task, sip_rtp_port) = self.prepare_session(&call_id).await?;

        // --- SDP negotiation (browser offer -> gateway answer) ---
        let offer = RTCSessionDescription::offer(browser_offer_sdp.to_owned())?;
        pc.set_remote_description(offer).await?;

        let answer = pc.create_answer(None).await?;
        pc.set_local_description(answer).await?;

        // Give ICE host candidate gathering a brief moment to complete.
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        let local_desc = pc
            .local_description()
            .await
            .ok_or_else(|| anyhow!("No local description after ICE gathering"))?;
        let answer_sdp = local_desc.sdp.clone();

        let session = WebRtcSession {
            pc,
            flow: BridgeFlow::BrowserCaller,
            sip_peer,
            answer_sdp: answer_sdp.clone(),
            created_at: Instant::now(),
            sip_rx_task,
        };
        self.sessions.lock().await.insert(call_id.clone(), session);
        info!(
            "WebRTC session created for {}: sip_port={}",
            call_id, sip_rtp_port
        );
        Ok((answer_sdp, sip_rtp_port))
    }

    /// Create a WebRTC session for a SIP-phone-originated INVITE where the
    /// callee is a browser over WS/WSS.
    ///
    /// Returns `(webrtc_offer_sdp, sip_rtp_port)`:
    /// - `webrtc_offer_sdp`: offer to forward to browser callee
    /// - `sip_rtp_port`: gateway RTP port used in SIP-side 200 answer
    pub async fn create_session_for_sip_caller(&self, call_id: String) -> Result<(String, u16)> {
        let (pc, sip_peer, sip_rx_task, sip_rtp_port) = self.prepare_session(&call_id).await?;

        // --- SDP negotiation (gateway offer -> browser answer later) ---
        let offer = pc.create_offer(None).await?;
        pc.set_local_description(offer).await?;

        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let local_desc = pc
            .local_description()
            .await
            .ok_or_else(|| anyhow!("No local description after ICE gathering"))?;
        let webrtc_offer_sdp = local_desc.sdp.clone();

        // SIP caller should receive a plain RTP answer from gateway.
        let sip_answer_sdp = make_plain_rtp_sdp(&self.public_ip, sip_rtp_port);

        let session = WebRtcSession {
            pc,
            flow: BridgeFlow::SipCaller,
            sip_peer,
            answer_sdp: sip_answer_sdp.clone(),
            created_at: Instant::now(),
            sip_rx_task,
        };
        self.sessions.lock().await.insert(call_id.clone(), session);
        info!(
            "Reverse WebRTC session created for {}: sip_port={}",
            call_id, sip_rtp_port
        );
        Ok((webrtc_offer_sdp, sip_rtp_port))
    }

    async fn prepare_session(
        &self,
        call_id: &str,
    ) -> Result<(
        Arc<RTCPeerConnection>,
        Arc<Mutex<Option<SocketAddr>>>,
        JoinHandle<()>,
        u16,
    )> {
        // --- MediaEngine: PCMU + PCMA only (no transcoding needed) ---
        let mut m = MediaEngine::default();
        m.register_codec(
            RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_PCMU.to_owned(),
                    clock_rate: 8000,
                    channels: 1,
                    sdp_fmtp_line: "".to_owned(),
                    rtcp_feedback: vec![],
                },
                payload_type: 0,
                ..Default::default()
            },
            RTPCodecType::Audio,
        )?;
        m.register_codec(
            RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_PCMA.to_owned(),
                    clock_rate: 8000,
                    channels: 1,
                    sdp_fmtp_line: "".to_owned(),
                    rtcp_feedback: vec![],
                },
                payload_type: 8,
                ..Default::default()
            },
            RTPCodecType::Audio,
        )?;

        // --- SettingEngine: NAT 1:1 mapping ---
        let mut se = SettingEngine::default();
        se.set_nat_1to1_ips(vec![self.public_ip.clone()], RTCIceCandidateType::Host);

        // --- Build API + PeerConnection ---
        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut m)?;
        let api = APIBuilder::new()
            .with_media_engine(m)
            .with_interceptor_registry(registry)
            .with_setting_engine(se)
            .build();

        let pc = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await?);

        // --- Local audio track (SIP phone → browser direction) ---
        let local_track = Arc::new(TrackLocalStaticRTP::new(
            RTCRtpCodecCapability {
                mime_type: MIME_TYPE_PCMU.to_owned(),
                clock_rate: 8000,
                channels: 1,
                sdp_fmtp_line: "".to_owned(),
                rtcp_feedback: vec![],
            },
            "audio".to_owned(),
            format!("sip3gw-{}", &call_id[..call_id.len().min(8)]),
        ));
        let rtp_sender = pc
            .add_track(Arc::clone(&local_track) as Arc<dyn TrackLocal + Send + Sync>)
            .await?;

        // Drain RTCP to prevent sender backpressure.
        tokio::spawn(async move { while rtp_sender.read_rtcp().await.is_ok() {} });

        // --- SIP-side UDP socket (SIP phone sends RTP here) ---
        let sip_socket = Arc::new(self.bind_sip_port().await?);
        let sip_rtp_port = sip_socket.local_addr()?.port();
        let sip_peer: Arc<Mutex<Option<SocketAddr>>> = Arc::new(Mutex::new(None));

        // --- on_track: browser → SIP phone (register before negotiation) ---
        let sip_socket_rx = sip_socket.clone();
        let sip_peer_rx = sip_peer.clone();
        pc.on_track(Box::new(move |track, _receiver, _transceiver| {
            let socket = sip_socket_rx.clone();
            let peer = sip_peer_rx.clone();
            Box::pin(async move {
                let mut buf = vec![0u8; 4096];
                while let Ok((packet, _)) = track.read(&mut buf).await {
                    // Marshal the decoded RTP packet back to raw bytes.
                    if let Ok(raw) = packet.marshal()
                        && !raw.is_empty()
                        && let Some(addr) = *peer.lock().await
                    {
                        let _ = socket.send_to(&raw, addr).await;
                    }
                }
            })
        }));

        // --- Background task: SIP phone → browser via local_track ---
        let sip_socket_tx = sip_socket;
        let sip_peer_tx = sip_peer.clone();
        let local_track_tx = local_track;
        let sip_rx_task = tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            loop {
                match sip_socket_tx.recv_from(&mut buf).await {
                    Ok((n, src)) => {
                        // Learn SIP peer address from received packets (symmetric RTP).
                        // Update on change so stale SDP-derived addresses don't pin routing.
                        let mut peer = sip_peer_tx.lock().await;
                        if peer.map(|p| p != src).unwrap_or(true) {
                            *peer = Some(src);
                        }
                        drop(peer);
                        if let Err(e) = local_track_tx.write(&buf[..n]).await {
                            debug!("WebRTC gw: local track write error: {}", e);
                        }
                    }
                    Err(e) => {
                        warn!("WebRTC gw: SIP socket recv error: {}", e);
                        break;
                    }
                }
            }
        });

        Ok((pc, sip_peer, sip_rx_task, sip_rtp_port))
    }

    /// Set the SIP phone's RTP address for the call (from 200 OK SDP).
    pub async fn set_sip_peer(&self, call_id: &str, addr: SocketAddr) {
        let sip_peer = {
            let sessions = self.sessions.lock().await;
            sessions.get(call_id).map(|s| s.sip_peer.clone())
        };
        if let Some(peer) = sip_peer {
            *peer.lock().await = Some(addr);
            info!("WebRTC gw: SIP peer for {} set to {}", call_id, addr);
        }
    }

    pub async fn is_sip_caller_session(&self, call_id: &str) -> bool {
        let sessions = self.sessions.lock().await;
        sessions
            .get(call_id)
            .map(|s| s.flow == BridgeFlow::SipCaller)
            .unwrap_or(false)
    }

    /// For SIP-originated calls, apply browser's 200 OK SDP as remote answer.
    pub async fn apply_callee_answer(&self, call_id: &str, callee_answer_sdp: &str) -> Result<()> {
        let (pc, flow) = {
            let sessions = self.sessions.lock().await;
            match sessions.get(call_id) {
                Some(s) => (s.pc.clone(), s.flow),
                None => return Ok(()),
            }
        };
        if flow == BridgeFlow::SipCaller {
            let answer = RTCSessionDescription::answer(callee_answer_sdp.to_owned())?;
            pc.set_remote_description(answer).await?;
        }
        Ok(())
    }

    /// Return the stored WebRTC answer SDP, or `None` if no WebRTC session exists.
    pub async fn get_answer_sdp(&self, call_id: &str) -> Option<String> {
        let sessions = self.sessions.lock().await;
        sessions.get(call_id).map(|s| s.answer_sdp.clone())
    }

    /// Close the PeerConnection and remove the session.
    pub async fn remove_session(&self, call_id: &str) {
        let session = self.sessions.lock().await.remove(call_id);
        if let Some(s) = session {
            s.sip_rx_task.abort();
            let _ = s.pc.close().await;
            debug!("WebRTC gw: removed session {}", call_id);
        }
    }

    /// Close and remove sessions older than `max_age_secs`.
    pub async fn cleanup_stale_sessions(&self, max_age_secs: u64) {
        let stale: Vec<(String, WebRtcSession)> = {
            let mut sessions = self.sessions.lock().await;
            let now = Instant::now();
            let ids: Vec<String> = sessions
                .iter()
                .filter(|(_, s)| now.duration_since(s.created_at).as_secs() > max_age_secs)
                .map(|(k, _)| k.clone())
                .collect();
            ids.into_iter()
                .filter_map(|id| sessions.remove(&id).map(|v| (id, v)))
                .collect()
        };
        for (call_id, s) in stale {
            s.sip_rx_task.abort();
            let _ = s.pc.close().await;
            warn!("WebRTC gw: cleaned up stale session {}", call_id);
        }
    }

    /// Bind a single UDP socket for SIP phone communication.
    async fn bind_sip_port(&self) -> Result<UdpSocket> {
        let range = self.sip_port_max.saturating_sub(self.sip_port_min) as u32;
        if range == 0 {
            return Err(anyhow!("SIP RTP port range too small for WebRTC gateway"));
        }
        for _ in 0..range {
            let offset = self.sip_port_counter.fetch_add(1, Ordering::Relaxed) % range;
            let port = self.sip_port_min + offset as u16;
            if let Ok(sock) = UdpSocket::bind(format!("0.0.0.0:{}", port)).await {
                return Ok(sock);
            }
        }
        Err(anyhow!(
            "No available SIP RTP port in range {}-{}",
            self.sip_port_min,
            self.sip_port_max
        ))
    }
}
