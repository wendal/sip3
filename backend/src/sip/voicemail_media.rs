//! RTP media handling for voicemail recording and prompt playback.

use anyhow::{Result, anyhow};
use rand::Rng;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{interval, timeout};
use tracing::{debug, warn};

use super::g711::{decode_alaw, decode_ulaw, encode_alaw, encode_ulaw};
use super::voicemail_sdp::VoicemailCodec;
use crate::storage::voicemail::{LocalVoicemailStorage, pcm16_wav_bytes, read_pcm16_wav};

const SAMPLE_RATE: u32 = 8000;
const SAMPLES_PER_FRAME: usize = 160;
const RTP_HEADER_LEN: usize = 12;
const RTP_BUF_SIZE: usize = 4096;
const MAX_RECORD_SAMPLES: usize = 10_000_000;
const RECV_POLL: Duration = Duration::from_millis(250);
const FRAME_INTERVAL: Duration = Duration::from_millis(20);
const PLAY_PEER_WAIT: Duration = Duration::from_secs(5);

static MEDIA_REGISTRY: OnceLock<StdMutex<HashMap<String, MediaSession>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoicemailDtmf {
    One,
    Two,
    Seven,
    Nine,
    Pound,
    Star,
}

#[derive(Debug, Clone)]
pub struct VoicemailMedia {
    public_ip: String,
    min_port: u16,
    max_port: u16,
    owner: Arc<MediaOwner>,
    port_counter: Arc<AtomicU32>,
}

#[derive(Debug, Clone)]
struct MediaSession {
    owner_id: u64,
    socket: Arc<UdpSocket>,
    port: u16,
    codec: VoicemailCodec,
    audio_pt: u8,
    telephone_event_pt: Option<u8>,
    peer: Arc<Mutex<Option<SocketAddr>>>,
    active: Arc<AtomicBool>,
}

#[derive(Debug)]
struct MediaOwner {
    id: u64,
}

impl Drop for MediaOwner {
    fn drop(&mut self) {
        if let Some(registry) = MEDIA_REGISTRY.get()
            && let Ok(mut sessions) = registry.lock()
        {
            sessions.retain(|_, session| {
                if session.owner_id == self.id {
                    session.active.store(false, Ordering::SeqCst);
                    false
                } else {
                    true
                }
            });
        }
    }
}

impl VoicemailMedia {
    pub fn new(public_ip: impl Into<String>, min_port: u16, max_port: u16) -> Self {
        Self {
            public_ip: public_ip.into(),
            min_port,
            max_port,
            owner: Arc::new(MediaOwner {
                id: rand::rng().random(),
            }),
            port_counter: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn public_ip(&self) -> &str {
        &self.public_ip
    }

    pub async fn allocate(
        &self,
        call_id: String,
        codec: VoicemailCodec,
        audio_pt: u8,
        telephone_event_pt: Option<u8>,
    ) -> Result<u16> {
        validate_payload_type(audio_pt)?;
        if let Some(pt) = telephone_event_pt {
            validate_payload_type(pt)?;
        }

        let socket = self.bind_socket().await?;
        let port = socket.local_addr()?.port();
        let session = MediaSession {
            owner_id: self.owner.id,
            socket: Arc::new(socket),
            port,
            codec,
            audio_pt,
            telephone_event_pt,
            peer: Arc::new(Mutex::new(None)),
            active: Arc::new(AtomicBool::new(true)),
        };

        let mut registry = registry()
            .lock()
            .map_err(|_| anyhow!("voicemail media registry lock poisoned"))?;
        if registry.contains_key(&call_id) {
            session.active.store(false, Ordering::SeqCst);
            return Err(anyhow!("voicemail media call id already exists: {call_id}"));
        }
        registry.insert(call_id.clone(), session);

        debug!("Allocated voicemail media for {} on port {}", call_id, port);
        Ok(port)
    }

    pub async fn remove(&self, call_id: &str) {
        match registry().lock() {
            Ok(mut registry) => {
                if let Some(session) = registry.remove(call_id) {
                    session.active.store(false, Ordering::SeqCst);
                    debug!(
                        "Removed voicemail media for {} from port {}",
                        call_id, session.port
                    );
                }
            }
            Err(e) => warn!("voicemail media registry lock poisoned: {}", e),
        }
    }

    async fn bind_socket(&self) -> Result<UdpSocket> {
        if self.min_port == 0 {
            return Err(anyhow!("voicemail RTP port range cannot start at 0"));
        }
        if self.min_port > self.max_port {
            return Err(anyhow!(
                "voicemail RTP port range is invalid: {}-{}",
                self.min_port,
                self.max_port
            ));
        }

        let range = u32::from(self.max_port) - u32::from(self.min_port) + 1;
        let start = self.port_counter.fetch_add(1, Ordering::Relaxed) % range;
        for attempt in 0..range {
            let offset = (start + attempt) % range;
            let port = u32::from(self.min_port) + offset;
            if let Ok(socket) = UdpSocket::bind(format!("0.0.0.0:{port}")).await {
                return Ok(socket);
            }
        }

        Err(anyhow!(
            "no available voicemail RTP port in range {}-{}",
            self.min_port,
            self.max_port
        ))
    }
}

pub async fn record_to_storage(
    call_id: &str,
    mailbox: &str,
    max_secs: u32,
    idle_secs: u32,
    storage: &LocalVoicemailStorage,
) -> Result<(String, u32)> {
    let session = get_session(call_id).await?;
    let max_samples = max_record_samples(max_secs)?;
    let mut samples = Vec::with_capacity(max_samples.min(SAMPLE_RATE as usize * 2));
    let mut buf = vec![0u8; RTP_BUF_SIZE];
    let started = Instant::now();
    let mut last_activity = Instant::now();
    let max_duration = Duration::from_secs(u64::from(max_secs));
    let idle_duration = (idle_secs > 0).then(|| Duration::from_secs(u64::from(idle_secs)));

    while session.active.load(Ordering::SeqCst) && samples.len() < max_samples {
        if max_secs > 0 && started.elapsed() >= max_duration {
            break;
        }
        if let Some(idle) = idle_duration
            && last_activity.elapsed() >= idle
        {
            break;
        }

        let (len, src) = match timeout(RECV_POLL, session.socket.recv_from(&mut buf)).await {
            Ok(Ok(packet)) => packet,
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => continue,
        };

        let packet = &buf[..len];
        let Some((pt, payload_offset)) = validated_rtp_packet(&session, packet) else {
            continue;
        };
        if !learn_or_verify_peer(&session, src).await {
            continue;
        }

        let payload = &packet[payload_offset..];
        if Some(pt) == session.telephone_event_pt {
            if rtp_dtmf_is_end(payload)
                && matches!(dtmf_from_rtp_event(payload[0]), Some(VoicemailDtmf::Pound))
            {
                break;
            }
            last_activity = Instant::now();
            continue;
        }
        if pt != session.audio_pt {
            continue;
        }

        let pcm = match session.codec {
            VoicemailCodec::Pcmu => decode_ulaw(payload),
            VoicemailCodec::Pcma => decode_alaw(payload),
        };
        if pcm.is_empty() {
            continue;
        }
        let remaining = max_samples.saturating_sub(samples.len());
        samples.extend(pcm.into_iter().take(remaining));
        last_activity = Instant::now();
    }

    let duration_secs = duration_secs(samples.len());
    let wav = pcm16_wav_bytes(&samples, SAMPLE_RATE);
    let key = storage.write_message(mailbox, call_id, &wav).await?;
    Ok((key, duration_secs))
}

pub async fn play_wav(call_id: &str, key: &str, storage: &LocalVoicemailStorage) -> Result<()> {
    let session = get_session(call_id).await?;
    let bytes = storage.read(key).await?;
    let wav = read_pcm16_wav(&bytes)?;
    if wav.sample_rate != SAMPLE_RATE {
        return Err(anyhow!(
            "voicemail WAV sample rate must be {} Hz, got {}",
            SAMPLE_RATE,
            wav.sample_rate
        ));
    }

    let mut peer = wait_for_peer(&session).await?;
    let mut seq: u16 = rand::rng().random();
    let mut ts: u32 = rand::rng().random();
    let ssrc: u32 = rand::rng().random();
    let mut ticker = interval(FRAME_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ticker.tick().await;
    let mut recv_buf = vec![0u8; RTP_BUF_SIZE];

    for chunk in wav.samples.chunks(SAMPLES_PER_FRAME) {
        if !session.active.load(Ordering::SeqCst) {
            break;
        }
        if let Some(updated_peer) = refresh_peer_nonblocking(&session, &mut recv_buf).await {
            peer = updated_peer;
        }

        let mut frame = chunk.to_vec();
        if frame.len() < SAMPLES_PER_FRAME {
            frame.resize(SAMPLES_PER_FRAME, 0);
        }
        let payload = match session.codec {
            VoicemailCodec::Pcmu => encode_ulaw(&frame),
            VoicemailCodec::Pcma => encode_alaw(&frame),
        };
        let packet = build_rtp_packet(session.audio_pt, seq, ts, ssrc, &payload);
        session.socket.send_to(&packet, peer).await?;
        seq = seq.wrapping_add(1);
        ts = ts.wrapping_add(SAMPLES_PER_FRAME as u32);
        ticker.tick().await;
    }

    Ok(())
}

pub fn parse_dtmf_relay(body: &str) -> Option<VoicemailDtmf> {
    body.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        if !name.trim().eq_ignore_ascii_case("Signal") {
            return None;
        }
        match value.trim() {
            "1" => Some(VoicemailDtmf::One),
            "2" => Some(VoicemailDtmf::Two),
            "7" => Some(VoicemailDtmf::Seven),
            "9" => Some(VoicemailDtmf::Nine),
            "#" => Some(VoicemailDtmf::Pound),
            "*" => Some(VoicemailDtmf::Star),
            _ => None,
        }
    })
}

pub fn build_rtp_packet(pt: u8, seq: u16, ts: u32, ssrc: u32, payload: &[u8]) -> Vec<u8> {
    debug_assert!(pt <= 127, "RTP payload type must be in 0..=127");
    let mut buf = Vec::with_capacity(RTP_HEADER_LEN + payload.len());
    buf.push(0x80);
    buf.push(pt & 0x7F);
    buf.extend_from_slice(&seq.to_be_bytes());
    buf.extend_from_slice(&ts.to_be_bytes());
    buf.extend_from_slice(&ssrc.to_be_bytes());
    buf.extend_from_slice(payload);
    buf
}

pub fn rtp_payload_offset(packet: &[u8]) -> Option<usize> {
    if packet.len() < RTP_HEADER_LEN || (packet[0] >> 6) != 2 {
        return None;
    }

    let cc = (packet[0] & 0x0F) as usize;
    let mut offset = RTP_HEADER_LEN.checked_add(4usize.checked_mul(cc)?)?;
    if packet.len() < offset {
        return None;
    }

    if (packet[0] & 0x10) != 0 {
        let header_end = offset.checked_add(4)?;
        if packet.len() < header_end {
            return None;
        }
        let ext_words = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]) as usize;
        let ext_bytes = 4usize.checked_mul(ext_words)?;
        offset = header_end.checked_add(ext_bytes)?;
        if packet.len() < offset {
            return None;
        }
    }

    Some(offset)
}

fn registry() -> &'static StdMutex<HashMap<String, MediaSession>> {
    MEDIA_REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()))
}

async fn get_session(call_id: &str) -> Result<MediaSession> {
    registry()
        .lock()
        .map_err(|_| anyhow!("voicemail media registry lock poisoned"))?
        .get(call_id)
        .cloned()
        .ok_or_else(|| anyhow!("voicemail media session not found for {call_id}"))
}

fn validate_payload_type(pt: u8) -> Result<()> {
    if pt > 127 {
        return Err(anyhow!("RTP payload type must be in 0..=127, got {pt}"));
    }
    Ok(())
}

fn max_record_samples(max_secs: u32) -> Result<usize> {
    if max_secs == 0 {
        return Err(anyhow!("voicemail max recording duration must be positive"));
    }
    let samples = u64::from(max_secs)
        .checked_mul(u64::from(SAMPLE_RATE))
        .ok_or_else(|| anyhow!("voicemail max recording duration overflows"))?;
    let samples = usize::try_from(samples)
        .map_err(|_| anyhow!("voicemail max recording duration too large"))?;
    if samples > MAX_RECORD_SAMPLES {
        return Err(anyhow!(
            "voicemail max recording duration exceeds {} samples",
            MAX_RECORD_SAMPLES
        ));
    }
    Ok(samples)
}

fn duration_secs(samples: usize) -> u32 {
    let sample_rate = SAMPLE_RATE as usize;
    samples
        .saturating_add(sample_rate - 1)
        .checked_div(sample_rate)
        .and_then(|secs| u32::try_from(secs).ok())
        .unwrap_or(u32::MAX)
}

fn rtp_dtmf_is_end(payload: &[u8]) -> bool {
    payload.len() >= 4 && (payload[1] & 0x80) != 0
}

fn dtmf_from_rtp_event(event: u8) -> Option<VoicemailDtmf> {
    match event {
        1 => Some(VoicemailDtmf::One),
        2 => Some(VoicemailDtmf::Two),
        7 => Some(VoicemailDtmf::Seven),
        9 => Some(VoicemailDtmf::Nine),
        10 => Some(VoicemailDtmf::Star),
        11 => Some(VoicemailDtmf::Pound),
        _ => None,
    }
}

async fn wait_for_peer(session: &MediaSession) -> Result<SocketAddr> {
    if let Some(peer) = *session.peer.lock().await {
        return Ok(peer);
    }

    let deadline = Instant::now() + PLAY_PEER_WAIT;
    let mut buf = vec![0u8; RTP_BUF_SIZE];
    loop {
        if !session.active.load(Ordering::SeqCst) {
            return Err(anyhow!("voicemail media session was removed"));
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(anyhow!("timed out waiting for voicemail RTP peer"));
        }
        let wait = RECV_POLL.min(deadline.saturating_duration_since(now));
        match timeout(wait, session.socket.recv_from(&mut buf)).await {
            Ok(Ok((len, src))) => {
                let packet = &buf[..len];
                if validated_rtp_packet(session, packet).is_some()
                    && learn_or_verify_peer(session, src).await
                {
                    return Ok(src);
                }
            }
            Ok(Err(e)) => return Err(e.into()),
            Err(_) => {}
        }
    }
}

async fn refresh_peer_nonblocking(session: &MediaSession, buf: &mut [u8]) -> Option<SocketAddr> {
    let mut latest = None;
    loop {
        match session.socket.try_recv_from(buf) {
            Ok((len, src)) => {
                let packet = &buf[..len];
                if validated_rtp_packet(session, packet).is_some()
                    && learn_or_verify_peer(session, src).await
                {
                    latest = Some(src);
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(e) => {
                warn!("voicemail RTP peer refresh error: {}", e);
                break;
            }
        }
    }
    latest
}

fn validated_rtp_packet(session: &MediaSession, packet: &[u8]) -> Option<(u8, usize)> {
    let payload_offset = rtp_payload_offset(packet)?;
    if payload_offset >= packet.len() {
        return None;
    }
    let pt = packet[1] & 0x7F;
    if pt != session.audio_pt && Some(pt) != session.telephone_event_pt {
        return None;
    }
    Some((pt, payload_offset))
}

async fn learn_or_verify_peer(session: &MediaSession, src: SocketAddr) -> bool {
    let mut peer = session.peer.lock().await;
    match *peer {
        Some(existing) if existing != src => false,
        Some(_) => true,
        None => {
            *peer = Some(src);
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_dtmf_relay_signal_to_control() {
        assert_eq!(
            parse_dtmf_relay("Signal=#\r\nDuration=160\r\n"),
            Some(VoicemailDtmf::Pound)
        );
        assert_eq!(
            parse_dtmf_relay("Signal=7\r\nDuration=160\r\n"),
            Some(VoicemailDtmf::Seven)
        );
        assert_eq!(
            parse_dtmf_relay("Signal=*\r\nDuration=160\r\n"),
            Some(VoicemailDtmf::Star)
        );
    }

    #[test]
    fn builds_rtp_packet_with_header() {
        let payload = [0xff, 0xfe, 0xfd];
        let packet = build_rtp_packet(0, 7, 160, 0x11223344, &payload);
        assert_eq!(packet.len(), 15);
        assert_eq!(packet[0], 0x80);
        assert_eq!(packet[1], 0);
        assert_eq!(&packet[2..4], &7u16.to_be_bytes());
        assert_eq!(&packet[4..8], &160u32.to_be_bytes());
        assert_eq!(&packet[8..12], &0x11223344u32.to_be_bytes());
        assert_eq!(&packet[12..], &payload);
    }

    #[test]
    fn rtp_payload_offset_handles_csrc_and_extension() {
        let mut packet = vec![0x92, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3];
        packet.extend_from_slice(&[0, 0, 0, 4]); // CSRC 1
        packet.extend_from_slice(&[0, 0, 0, 5]); // CSRC 2
        packet.extend_from_slice(&[0xbe, 0xde, 0, 1]); // extension header, 1 word
        packet.extend_from_slice(&[1, 2, 3, 4]); // extension payload
        packet.extend_from_slice(&[0xaa, 0xbb]); // RTP payload

        assert_eq!(rtp_payload_offset(&packet), Some(28));
    }

    #[test]
    fn rtp_payload_offset_rejects_truncated_packets() {
        assert_eq!(rtp_payload_offset(&[0x80; 11]), None);
        assert_eq!(rtp_payload_offset(&[0x81; 12]), None);

        let mut truncated_extension = vec![0x90, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3];
        truncated_extension.extend_from_slice(&[0xbe, 0xde, 0, 2]);
        truncated_extension.extend_from_slice(&[1, 2, 3, 4]);
        assert_eq!(rtp_payload_offset(&truncated_extension), None);
    }

    #[test]
    fn parse_dtmf_relay_trims_and_rejects_unknown_signals() {
        assert_eq!(
            parse_dtmf_relay("Duration=160\r\nSignal = 9\r\n"),
            Some(VoicemailDtmf::Nine)
        );
        assert_eq!(parse_dtmf_relay("Signal=3\r\nDuration=160\r\n"), None);
        assert_eq!(parse_dtmf_relay("Duration=160\r\n"), None);
    }

    #[test]
    fn rtp_dtmf_rejects_invalid_event_values() {
        assert_eq!(dtmf_from_rtp_event(11), Some(VoicemailDtmf::Pound));
        assert_eq!(dtmf_from_rtp_event(27), None);
    }

    #[tokio::test]
    async fn allocate_rejects_invalid_port_range() {
        let media = VoicemailMedia::new("203.0.113.10".to_string(), 20000, 19999);
        let result = media
            .allocate("call-a".to_string(), VoicemailCodec::Pcmu, 0, Some(101))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn allocate_rejects_invalid_payload_type() {
        let media = VoicemailMedia::new("203.0.113.10".to_string(), 20000, 20000);
        let result = media
            .allocate("call-a".to_string(), VoicemailCodec::Pcmu, 128, None)
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn record_to_storage_requires_allocated_session() {
        let storage =
            LocalVoicemailStorage::new(std::path::PathBuf::from("target\\voicemail-media-tests"));
        let result = record_to_storage("missing-call", "1001", 1, 1, &storage).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn record_to_storage_rejects_zero_max_secs() {
        let call_id = format!("zero-call-{}", rand::rng().random::<u64>());
        let (media, _port) = allocate_test_session(&call_id, VoicemailCodec::Pcmu).await;
        let storage =
            LocalVoicemailStorage::new(std::path::PathBuf::from("target\\voicemail-media-tests"));
        let result = record_to_storage(&call_id, "1001", 0, 1, &storage).await;

        assert!(result.is_err());
        media.remove(&call_id).await;
    }

    #[tokio::test]
    async fn record_to_storage_decodes_audio_and_stops_on_pound() {
        let call_id = format!("record-call-{}", rand::rng().random::<u64>());
        let (media, port) = allocate_test_session(&call_id, VoicemailCodec::Pcmu).await;
        let root = std::path::PathBuf::from(format!(
            "target\\voicemail-media-tests\\{}",
            rand::rng().random::<u64>()
        ));
        let storage = LocalVoicemailStorage::new(root.clone());
        let client = UdpSocket::bind("127.0.0.1:0").await.expect("client bind");
        let dst = format!("127.0.0.1:{port}");

        let recorder = async {
            record_to_storage(&call_id, "1001", 1, 1, &storage)
                .await
                .expect("record")
        };
        let sender = async {
            tokio::time::sleep(Duration::from_millis(25)).await;
            let audio = build_rtp_packet(0, 1, 160, 0x01020304, &[0xff; SAMPLES_PER_FRAME]);
            client.send_to(&audio, &dst).await.expect("send audio");
            let pound = build_rtp_packet(101, 2, 320, 0x01020304, &[11, 0x80, 0, 160]);
            client.send_to(&pound, &dst).await.expect("send pound");
        };

        let ((key, duration_secs), _) = tokio::join!(recorder, sender);
        let decoded = read_pcm16_wav(&storage.read(&key).await.expect("read wav")).expect("wav");

        assert_eq!(duration_secs, 1);
        assert_eq!(decoded.sample_rate, SAMPLE_RATE);
        assert_eq!(decoded.samples.len(), SAMPLES_PER_FRAME);

        media.remove(&call_id).await;
        storage.delete(&key).await.expect("delete");
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn play_wav_sends_rtp_to_learned_peer() {
        let call_id = format!("play-call-{}", rand::rng().random::<u64>());
        let (media, port) = allocate_test_session(&call_id, VoicemailCodec::Pcma).await;
        let root = std::path::PathBuf::from(format!(
            "target\\voicemail-media-tests\\{}",
            rand::rng().random::<u64>()
        ));
        let storage = LocalVoicemailStorage::new(root.clone());
        let key = storage
            .write_message(
                "1001",
                &call_id,
                &pcm16_wav_bytes(&vec![0i16; SAMPLES_PER_FRAME], SAMPLE_RATE),
            )
            .await
            .expect("write prompt");
        let client = UdpSocket::bind("127.0.0.1:0").await.expect("client bind");
        let dst = format!("127.0.0.1:{port}");

        let player = async { play_wav(&call_id, &key, &storage).await.expect("play") };
        let receiver = async {
            tokio::time::sleep(Duration::from_millis(25)).await;
            let inbound = build_rtp_packet(8, 1, 160, 0x01020304, &[0xd5; SAMPLES_PER_FRAME]);
            client.send_to(&inbound, &dst).await.expect("seed peer");
            let mut buf = [0u8; 512];
            let (len, _) = timeout(Duration::from_secs(1), client.recv_from(&mut buf))
                .await
                .expect("receive timeout")
                .expect("receive packet");
            buf[..len].to_vec()
        };

        let (_, packet) = tokio::join!(player, receiver);

        assert_eq!(packet[0], 0x80);
        assert_eq!(packet[1], 8);
        assert_eq!(rtp_payload_offset(&packet), Some(12));
        assert_eq!(packet.len(), RTP_HEADER_LEN + SAMPLES_PER_FRAME);

        media.remove(&call_id).await;
        storage.delete(&key).await.expect("delete");
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn record_ignores_malformed_packets_when_learning_peer() {
        let call_id = format!("malformed-peer-call-{}", rand::rng().random::<u64>());
        let (media, port) = allocate_test_session(&call_id, VoicemailCodec::Pcmu).await;
        let session = get_session(&call_id).await.expect("session");
        let root = std::path::PathBuf::from(format!(
            "target\\voicemail-media-tests\\{}",
            rand::rng().random::<u64>()
        ));
        let storage = LocalVoicemailStorage::new(root.clone());
        let sender = UdpSocket::bind("127.0.0.1:0").await.expect("sender bind");
        let dst = format!("127.0.0.1:{port}");

        let recorder = async {
            record_to_storage(&call_id, "1001", 2, 1, &storage)
                .await
                .expect("record")
        };
        let poison = async {
            tokio::time::sleep(Duration::from_millis(25)).await;
            sender
                .send_to(&[0x80, 0x00, 0x00], &dst)
                .await
                .expect("send malformed");
        };

        let ((key, _duration), _) = tokio::join!(recorder, poison);

        assert_eq!(*session.peer.lock().await, None);

        media.remove(&call_id).await;
        storage.delete(&key).await.expect("delete");
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn wait_for_peer_skips_mismatched_payload_type() {
        let call_id = format!("wait-peer-call-{}", rand::rng().random::<u64>());
        let (media, port) = allocate_test_session(&call_id, VoicemailCodec::Pcmu).await;
        let session = get_session(&call_id).await.expect("session");
        let wrong_sender = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("wrong sender bind");
        let valid_sender = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("valid sender bind");
        let dst = format!("127.0.0.1:{port}");
        let valid_addr = valid_sender.local_addr().expect("valid addr");

        let waiter = async { wait_for_peer(&session).await.expect("peer") };
        let sender = async {
            tokio::time::sleep(Duration::from_millis(25)).await;
            let wrong = build_rtp_packet(96, 1, 160, 0x01020304, &[0xaa; SAMPLES_PER_FRAME]);
            wrong_sender
                .send_to(&wrong, &dst)
                .await
                .expect("send wrong pt");
            let valid = build_rtp_packet(0, 2, 320, 0x01020304, &[0xff; SAMPLES_PER_FRAME]);
            valid_sender
                .send_to(&valid, &dst)
                .await
                .expect("send valid pt");
        };

        let (peer, _) = tokio::join!(waiter, sender);

        assert_eq!(peer, valid_addr);

        media.remove(&call_id).await;
    }

    #[tokio::test]
    async fn refresh_peer_ignores_different_source_once_peer_known() {
        let call_id = format!("refresh-peer-call-{}", rand::rng().random::<u64>());
        let (media, port) = allocate_test_session(&call_id, VoicemailCodec::Pcmu).await;
        let session = get_session(&call_id).await.expect("session");
        let trusted_peer = UdpSocket::bind("127.0.0.1:0").await.expect("trusted bind");
        let attacker = UdpSocket::bind("127.0.0.1:0").await.expect("attacker bind");
        let trusted_addr = trusted_peer.local_addr().expect("trusted addr");
        let dst = format!("127.0.0.1:{port}");
        *session.peer.lock().await = Some(trusted_addr);

        let attack = build_rtp_packet(0, 1, 160, 0x01020304, &[0xff; SAMPLES_PER_FRAME]);
        attacker
            .send_to(&attack, &dst)
            .await
            .expect("send attacker packet");
        tokio::time::sleep(Duration::from_millis(25)).await;
        let mut buf = [0u8; RTP_BUF_SIZE];

        assert_eq!(refresh_peer_nonblocking(&session, &mut buf).await, None);
        assert_eq!(*session.peer.lock().await, Some(trusted_addr));

        media.remove(&call_id).await;
    }

    async fn allocate_test_session(call_id: &str, codec: VoicemailCodec) -> (VoicemailMedia, u16) {
        for _ in 0..25 {
            let probe = UdpSocket::bind("127.0.0.1:0").await.expect("probe bind");
            let port = probe.local_addr().expect("probe addr").port();
            drop(probe);

            let media = VoicemailMedia::new("127.0.0.1".to_string(), port, port);
            match media
                .allocate(call_id.to_string(), codec, codec.static_pt(), Some(101))
                .await
            {
                Ok(allocated) => {
                    assert_eq!(allocated, port);
                    return (media, allocated);
                }
                Err(_) => continue,
            }
        }
        panic!("failed to allocate test voicemail media port");
    }
}
