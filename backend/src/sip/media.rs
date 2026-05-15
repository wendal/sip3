use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

/// Maximum size of a single RTP datagram.
const RTP_BUF_SIZE: usize = 8192;

/// SDP media types that the RTP relay can transparently forward.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum SdpMediaKind {
    Audio,
    Video,
}

/// A supported SDP media section (`m=audio` or `m=video`) in offer/answer order.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct SdpMediaDescriptor {
    pub kind: SdpMediaKind,
    /// Zero-based index among all SDP `m=` sections, including unsupported media.
    pub media_index: usize,
    pub port: u16,
    pub active: bool,
}

/// Relay ports assigned for one SDP media section.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct MediaRelayPorts {
    pub kind: SdpMediaKind,
    pub media_index: usize,
    /// UDP port that the callee should send RTP to. Server relays -> caller.
    pub relay_port_a: u16,
    /// UDP port that the caller should send RTP to. Server relays -> callee.
    pub relay_port_b: u16,
}

/// RTP relay state for one SDP media section.
pub struct MediaStreamSession {
    pub kind: SdpMediaKind,
    pub media_index: usize,
    /// Actual RTP source address learned from the first packet arriving on relay_a
    /// (i.e. the callee's real public RTP address after NAT).
    pub callee_rtp: Arc<Mutex<Option<SocketAddr>>>,
    /// Actual RTP source address learned from the first packet arriving on relay_b
    /// (i.e. the caller's real public RTP address after NAT).
    pub caller_rtp: Arc<Mutex<Option<SocketAddr>>>,
    /// UDP port that the callee should send RTP to. Server relays -> caller.
    pub relay_port_a: u16,
    /// UDP port that the caller should send RTP to. Server relays -> callee.
    pub relay_port_b: u16,
    /// Background task forwarding callee->caller.
    task_a: JoinHandle<()>,
    /// Background task forwarding caller->callee.
    task_b: JoinHandle<()>,
}

impl MediaStreamSession {
    fn abort(&self) {
        self.task_a.abort();
        self.task_b.abort();
    }

    fn ports(&self) -> MediaRelayPorts {
        MediaRelayPorts {
            kind: self.kind,
            media_index: self.media_index,
            relay_port_a: self.relay_port_a,
            relay_port_b: self.relay_port_b,
        }
    }
}

/// Per-call RTP relay state.
pub struct MediaSession {
    pub streams: Vec<MediaStreamSession>,
    /// When this session was created; used for stale-session cleanup.
    pub created_at: std::time::Instant,
}

impl MediaSession {
    fn abort(&self) {
        for stream in &self.streams {
            stream.abort();
        }
    }

    pub fn first_ports(&self) -> Option<MediaRelayPorts> {
        self.streams.first().map(MediaStreamSession::ports)
    }

    pub fn callee_sdp_ports(&self) -> Vec<(usize, u16)> {
        self.streams
            .iter()
            .map(|stream| (stream.media_index, stream.relay_port_a))
            .collect()
    }

    pub fn caller_sdp_ports(&self) -> Vec<(usize, u16)> {
        self.streams
            .iter()
            .map(|stream| (stream.media_index, stream.relay_port_b))
            .collect()
    }
}

/// Shared RTP media relay manager.
///
/// For each call, two UDP sockets are allocated:
///  - relay_a: callee sends RTP here → server forwards to caller
///  - relay_b: caller sends RTP here → server forwards to callee
///
/// Endpoints are learned from the source address of the first packet received on
/// each socket (symmetric RTP), so private/NATted addresses in SDP are irrelevant.
#[derive(Clone)]
pub struct MediaRelay {
    pub sessions: Arc<Mutex<HashMap<String, MediaSession>>>,
    /// Public IP/hostname written into the `c=IN IP4` SDP line.
    pub public_ip: Arc<String>,
    port_min: u16,
    port_max: u16,
    /// Rolling counter for port allocation (wraps within [port_min, port_max)).
    port_counter: Arc<AtomicU32>,
}

impl MediaRelay {
    pub fn new(public_ip: String, port_min: u16, port_max: u16) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            public_ip: Arc::new(public_ip),
            port_min,
            port_max,
            port_counter: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Allocate a single-audio relay session for `call_id`.
    ///
    /// Returns `(relay_port_a, relay_port_b)` to be written into the SDP offered to
    /// the callee (relay_a) and later to the caller (relay_b).
    pub async fn allocate_session(&self, call_id: String) -> Result<(u16, u16)> {
        let media = [SdpMediaDescriptor {
            kind: SdpMediaKind::Audio,
            media_index: 0,
            port: 0,
            active: true,
        }];
        let ports = self.allocate_session_for_media(call_id, &media).await?;
        let first = ports
            .first()
            .ok_or_else(|| anyhow!("No relay ports allocated for media session"))?;
        Ok((first.relay_port_a, first.relay_port_b))
    }

    /// Allocate relay streams for every active supported SDP media section.
    pub async fn allocate_session_for_media(
        &self,
        call_id: String,
        media: &[SdpMediaDescriptor],
    ) -> Result<Vec<MediaRelayPorts>> {
        let active_media: Vec<SdpMediaDescriptor> =
            media.iter().copied().filter(|m| m.active).collect();
        if active_media.is_empty() {
            return Err(anyhow!("No active RTP media sections to relay"));
        }

        let mut streams = Vec::with_capacity(active_media.len());
        for descriptor in active_media {
            match self.allocate_stream(descriptor).await {
                Ok(stream) => streams.push(stream),
                Err(e) => {
                    for stream in &streams {
                        stream.abort();
                    }
                    return Err(e);
                }
            }
        }

        let ports: Vec<MediaRelayPorts> = streams.iter().map(MediaStreamSession::ports).collect();
        let session = MediaSession {
            streams,
            created_at: std::time::Instant::now(),
        };

        self.sessions.lock().await.insert(call_id.clone(), session);
        debug!("Allocated media relay for {}: {:?}", call_id, ports);
        Ok(ports)
    }

    async fn allocate_stream(&self, descriptor: SdpMediaDescriptor) -> Result<MediaStreamSession> {
        let (sock_a, sock_b) = self.bind_port_pair().await?;
        let relay_port_a = sock_a.local_addr()?.port();
        let relay_port_b = sock_b.local_addr()?.port();

        let callee_rtp: Arc<Mutex<Option<SocketAddr>>> = Arc::new(Mutex::new(None));
        let caller_rtp: Arc<Mutex<Option<SocketAddr>>> = Arc::new(Mutex::new(None));

        let sock_a = Arc::new(sock_a);
        let sock_b = Arc::new(sock_b);

        // relay_a: callee sends here → forward to caller.
        //   • Learn callee's real address from packet source (symmetric RTP).
        //   • Forward to caller's learned address on relay_b.
        let callee_rtp_a = callee_rtp.clone();
        let caller_rtp_a = caller_rtp.clone();
        let recv_a = sock_a.clone();
        let send_b = sock_b.clone();
        let task_a = tokio::spawn(async move {
            run_relay_loop(recv_a, send_b, callee_rtp_a, caller_rtp_a).await;
        });

        // relay_b: caller sends here → forward to callee.
        //   • Learn caller's real address from packet source.
        //   • Forward to callee's learned address on relay_a.
        let callee_rtp_b = callee_rtp.clone();
        let caller_rtp_b = caller_rtp.clone();
        let recv_b = sock_b.clone();
        let send_a = sock_a.clone();
        let task_b = tokio::spawn(async move {
            run_relay_loop(recv_b, send_a, caller_rtp_b, callee_rtp_b).await;
        });

        Ok(MediaStreamSession {
            kind: descriptor.kind,
            media_index: descriptor.media_index,
            callee_rtp,
            caller_rtp,
            relay_port_a,
            relay_port_b,
            task_a,
            task_b,
        })
    }

    /// Remove the session and stop its relay tasks.
    pub async fn remove_session(&self, call_id: &str) {
        if let Some(session) = self.sessions.lock().await.remove(call_id) {
            session.abort();
            debug!("Removed media relay for {}", call_id);
        }
    }

    /// Abort and remove any sessions older than `max_age_secs`.
    /// Call this from a periodic background task to prevent port leaks
    /// when BYE is never received (network failure, client crash, etc.).
    pub async fn cleanup_stale_sessions(&self, max_age_secs: u64) {
        let mut sessions = self.sessions.lock().await;
        let now = std::time::Instant::now();
        let stale: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| now.duration_since(s.created_at).as_secs() > max_age_secs)
            .map(|(k, _)| k.clone())
            .collect();
        for call_id in stale {
            if let Some(session) = sessions.remove(&call_id) {
                session.abort();
                warn!("Cleaned up stale media session for call_id: {}", call_id);
            }
        }
    }

    /// Try to bind two consecutive even/odd UDP ports.
    async fn bind_port_pair(&self) -> Result<(UdpSocket, UdpSocket)> {
        let range = (self.port_max.saturating_sub(self.port_min)) as u32;
        if range < 2 {
            return Err(anyhow!("RTP port range too small"));
        }

        // Try up to range/2 candidates before giving up.
        for _ in 0..(range / 2) {
            let offset = self.port_counter.fetch_add(2, Ordering::Relaxed) % range;
            // Align to even port so RTP/RTCP pairs start cleanly.
            let base = self.port_min + (offset as u16 & !1u16);
            let port_a = base;
            let port_b = base + 1;

            let (sa, sb) = tokio::join!(
                UdpSocket::bind(format!("0.0.0.0:{}", port_a)),
                UdpSocket::bind(format!("0.0.0.0:{}", port_b)),
            );
            match (sa, sb) {
                (Ok(a), Ok(b)) => return Ok((a, b)),
                _ => continue,
            }
        }
        Err(anyhow!(
            "No available RTP port pair in range {}-{}",
            self.port_min,
            self.port_max
        ))
    }
}

/// Core relay loop: receive packets on `recv_socket`, learn the source as `own_addr`,
/// and forward to whatever address is stored in `peer_addr`.
async fn run_relay_loop(
    recv_socket: Arc<UdpSocket>,
    send_socket: Arc<UdpSocket>,
    own_addr: Arc<Mutex<Option<SocketAddr>>>,
    peer_addr: Arc<Mutex<Option<SocketAddr>>>,
) {
    let mut buf = vec![0u8; RTP_BUF_SIZE];
    loop {
        let (len, src) = match recv_socket.recv_from(&mut buf).await {
            Ok(r) => r,
            Err(e) => {
                warn!("RTP relay socket error: {}", e);
                break;
            }
        };

        // Update the learned address for this side.
        {
            let mut own = own_addr.lock().await;
            *own = Some(src);
        }

        // Forward to the peer if its address is known.
        if let Some(dst) = *peer_addr.lock().await
            && let Err(e) = send_socket.send_to(&buf[..len], dst).await
        {
            warn!("RTP relay forward error to {}: {}", dst, e);
        }
    }
}

// ─── SDP helpers ────────────────────────────────────────────────────────────

/// Return supported SDP media sections (`m=audio`, `m=video`) in media order.
pub fn parse_sdp_media_sections(sdp: &str) -> Vec<SdpMediaDescriptor> {
    let mut media = Vec::new();
    let mut media_index = 0usize;

    for line in sdp.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("m=") {
            continue;
        }

        let mut parts = trimmed.split_whitespace();
        let Some(media_token) = parts.next() else {
            media_index += 1;
            continue;
        };
        let Some(port_token) = parts.next() else {
            media_index += 1;
            continue;
        };

        let kind = match media_token.strip_prefix("m=") {
            Some("audio") => Some(SdpMediaKind::Audio),
            Some("video") => Some(SdpMediaKind::Video),
            _ => None,
        };

        if let Some(kind) = kind
            && let Some(port) = sdp_media_port(port_token)
        {
            media.push(SdpMediaDescriptor {
                kind,
                media_index,
                port,
                active: port != 0,
            });
        }

        media_index += 1;
    }

    media
}

fn sdp_media_port(port_token: &str) -> Option<u16> {
    port_token.split('/').next()?.parse().ok()
}

/// Rewrite a SDP body using relay ports keyed by zero-based SDP media index.
pub fn rewrite_sdp_media(sdp: &str, new_ip: &str, media_ports: &[(usize, u16)]) -> String {
    let port_by_media: HashMap<usize, u16> = media_ports.iter().copied().collect();
    let has_unsupported_media = sdp.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("m=")
            && !trimmed.starts_with("m=audio ")
            && !trimmed.starts_with("m=audio\t")
            && !trimmed.starts_with("m=video ")
            && !trimmed.starts_with("m=video\t")
    });
    let media_sections_with_c = sdp_media_sections_with_connection(sdp);

    let mut out = Vec::new();
    let mut current_media_index: Option<usize> = None;
    let mut next_media_index = 0usize;

    for line in sdp.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("m=") {
            let media_index = next_media_index;
            next_media_index += 1;
            current_media_index = Some(media_index);

            if let Some(new_port) = port_by_media.get(&media_index)
                && sdp_m_line_port(line).is_some_and(|port| port != 0)
            {
                out.push(rewrite_m_port(line, *new_port));
                if has_unsupported_media && !media_sections_with_c.contains_key(&media_index) {
                    out.push(format!("c=IN IP4 {}", new_ip));
                }
                continue;
            }
        } else if trimmed.starts_with("c=IN IP4 ") {
            if !has_unsupported_media {
                out.push(format!("c=IN IP4 {}", new_ip));
                continue;
            }
            if current_media_index.is_some_and(|idx| port_by_media.contains_key(&idx)) {
                out.push(format!("c=IN IP4 {}", new_ip));
                continue;
            }
        }

        out.push(line.to_string());
    }

    out.join("\r\n")
}

fn sdp_media_sections_with_connection(sdp: &str) -> HashMap<usize, ()> {
    let mut media_with_c = HashMap::new();
    let mut current_media_index: Option<usize> = None;
    let mut next_media_index = 0usize;

    for line in sdp.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("m=") {
            current_media_index = Some(next_media_index);
            next_media_index += 1;
        } else if trimmed.starts_with("c=IN IP4 ")
            && let Some(media_index) = current_media_index
        {
            media_with_c.insert(media_index, ());
        }
    }

    media_with_c
}

fn sdp_m_line_port(line: &str) -> Option<u16> {
    let port_token = line.split_whitespace().nth(1)?;
    sdp_media_port(port_token)
}

/// Rewrite a SDP body, replacing the connection address and the port of the
/// first active audio media stream with the supplied server values.
///
/// Handles:
///  - session-level `c=IN IP4 <addr>` (replaced unconditionally)
///  - media-level `c=IN IP4 <addr>` inside the first `m=audio` section
///  - the port field in `m=audio <port> ...`
///
/// Multiple media sections: only the first `m=audio` (or `m=audio`-compatible)
/// stream is touched; subsequent ones are passed through unchanged.
pub fn rewrite_sdp(sdp: &str, new_ip: &str, new_port: u16) -> String {
    let media_index = parse_sdp_media_sections(sdp)
        .into_iter()
        .find(|m| m.kind == SdpMediaKind::Audio && m.active)
        .map(|m| m.media_index);
    match media_index {
        Some(media_index) => rewrite_sdp_media(sdp, new_ip, &[(media_index, new_port)]),
        None => rewrite_sdp_media(sdp, new_ip, &[]),
    }
}

/// Replace the port field in a SDP `m=` line.
/// Input: `m=audio 49170 RTP/AVP 0 8`
/// Output: `m=audio 10001 RTP/AVP 0 8`
fn rewrite_m_port(line: &str, new_port: u16) -> String {
    // m=<media> <port>[/<count>] <proto> <fmt>...
    let mut parts = line.splitn(4, ' ');
    let media = parts.next().unwrap_or("");
    let port_field = parts.next().unwrap_or("0");
    let rest = parts.next().unwrap_or("");
    let fmt = parts.next().unwrap_or("");

    // Preserve /count suffix if present (e.g. "49170/2")
    let count_suffix = if let Some(pos) = port_field.find('/') {
        &port_field[pos..]
    } else {
        ""
    };

    if fmt.is_empty() {
        format!("{} {}{} {}", media, new_port, count_suffix, rest)
    } else {
        format!("{} {}{} {} {}", media, new_port, count_suffix, rest, fmt)
    }
}

/// Rewrite the `Content-Length` header of a raw SIP message whose body has
/// been replaced with `new_body`.  Returns the updated raw message string.
pub fn rewrite_content_length(raw: &str, new_body: &str) -> String {
    // SIP messages use CRLF but we also handle bare LF just in case.
    let sep = if raw.contains("\r\n\r\n") {
        "\r\n\r\n"
    } else {
        "\n\n"
    };

    let header_sep = if sep == "\r\n\r\n" { "\r\n" } else { "\n" };

    if let Some(pos) = raw.find(sep) {
        let headers_raw = &raw[..pos];
        let new_cl = new_body.len();

        let updated_headers: Vec<&str> = headers_raw
            .split(header_sep)
            .map(|line| {
                if line.to_lowercase().starts_with("content-length:") {
                    // We'll replace with the computed value below.
                    ""
                } else {
                    line
                }
            })
            .filter(|l| !l.is_empty())
            .collect();

        format!(
            "{}{}Content-Length: {}{}{}",
            updated_headers.join(header_sep),
            header_sep,
            new_cl,
            sep,
            new_body
        )
    } else {
        raw.to_string()
    }
}

/// Return true if the SDP body contains at least one `a=crypto:` attribute,
/// indicating that SRTP (SDES key exchange) is being offered.
pub fn sdp_has_crypto(sdp: &str) -> bool {
    sdp.lines()
        .any(|l| l.to_lowercase().starts_with("a=crypto:"))
}

/// Return true if the SIP response's `CSeq` header ends with `INVITE`
/// and a non-empty SDP body is present.
pub fn is_invite_200_ok_with_sdp(msg: &super::handler::SipMessage) -> bool {
    if msg.status_code != Some(200) {
        return false;
    }
    let cseq_is_invite = msg
        .cseq()
        .map(|s| s.to_uppercase().ends_with("INVITE"))
        .unwrap_or(false);
    let has_sdp = !msg.body.is_empty()
        && msg
            .header("content-type")
            .map(|ct| ct.to_lowercase().contains("application/sdp"))
            .unwrap_or(false);
    cseq_is_invite && has_sdp
}

/// Return true if `sdp` looks like a WebRTC SDP (ICE credentials / DTLS fingerprint).
pub fn is_webrtc_sdp(sdp: &str) -> bool {
    sdp.lines().any(|l| {
        let lower = l.trim().to_lowercase();
        lower.starts_with("a=ice-ufrag:") || lower.starts_with("a=fingerprint:")
    })
}

/// Build a minimal plain RTP SDP offer to send to a legacy SIP phone.
/// The phone returns RTP to `server_ip:sip_rtp_port`.
pub fn make_plain_rtp_sdp(server_ip: &str, sip_rtp_port: u16) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!(
        "v=0\r\no=- {ts} {ts} IN IP4 {ip}\r\ns=-\r\nc=IN IP4 {ip}\r\nt=0 0\r\n\
         m=audio {port} RTP/AVP 0 8\r\n\
         a=rtpmap:0 PCMU/8000\r\n\
         a=rtpmap:8 PCMA/8000\r\n\
         a=sendrecv\r\n",
        ts = ts,
        ip = server_ip,
        port = sip_rtp_port,
    )
}

/// Extract the connection IP from the first session-level `c=IN IP4 <addr>` line.
pub fn sdp_connection_ip(sdp: &str) -> Option<String> {
    for line in sdp.lines() {
        if let Some(addr) = line.strip_prefix("c=IN IP4 ") {
            return Some(addr.trim().to_owned());
        }
    }
    None
}

/// Parse the SIP phone's audio RTP `SocketAddr` from a SDP body.
/// Returns `None` if connection IP or audio port cannot be found.
pub fn sdp_rtp_addr(sdp: &str) -> Option<std::net::SocketAddr> {
    let ip = sdp_connection_ip(sdp)?;
    let port = sdp_audio_port(sdp)?;
    format!("{}:{}", ip, port).parse().ok()
}

/// Extract the first audio RTP port from a SDP body's `m=audio <port>` line.
pub fn sdp_audio_port(sdp: &str) -> Option<u16> {
    for line in sdp.lines() {
        if line.starts_with("m=audio ") || line.starts_with("m=audio\t") {
            let port_str = line.split_whitespace().nth(1)?;
            let port_only = port_str.split('/').next()?; // strip /count
            return port_only.parse().ok();
        }
    }
    None
}
