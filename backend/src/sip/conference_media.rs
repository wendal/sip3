//! Server-side RTP mixer for conference rooms.
//!
//! Each participant gets one UDP socket allocated from the conference RTP port
//! range. Inbound packets are decoded to 8 kHz 16-bit PCM and stored in a
//! latest-frame buffer per participant. A per-room ticker fires every 20 ms,
//! sums all other non-muted participants, encodes per receiver, and sends an
//! RTP packet back to each participant's learned peer address.
//!
//! Telephone-event (RFC 4733) inbound packets are parsed to detect the `*6`
//! mute/unmute sequence — see [`Participant::record_dtmf`].

use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::interval;
use tracing::{debug, warn};

use super::conference_sdp::ConferenceCodec;
use super::g711;

const SAMPLES_PER_FRAME: usize = 160; // 20 ms @ 8 kHz
const FRAME_INTERVAL: Duration = Duration::from_millis(20);
const RTP_BUF_SIZE: usize = 4096;
const RTP_HEADER_LEN: usize = 12;
const STAR6_TIMEOUT: Duration = Duration::from_secs(3);

/// Per-participant state. Owned by the room map via `Arc`.
struct Participant {
    call_id: String,
    relay_port: u16,
    codec: ConferenceCodec,
    audio_pt: u8,
    telephone_event_pt: Option<u8>,
    muted: AtomicBool,
    /// Learned RTP peer address (from first inbound packet, symmetric RTP).
    peer: Mutex<Option<SocketAddr>>,
    /// Latest decoded 20 ms PCM frame (160 samples). Empty means no audio yet.
    latest_frame: Mutex<Vec<i16>>,
    /// UDP socket — used by both the recv loop and mixer for sending back.
    socket: Arc<UdpSocket>,
    out_seq: AtomicU16,
    out_ts: AtomicU32,
    out_ssrc: u32,
    dtmf_state: Mutex<DtmfState>,
    recv_task: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Default)]
struct DtmfState {
    /// Last digit received with End marker, plus its timestamp.
    last_digit: Option<(u8, Instant)>,
    /// Last RTP timestamp from a telephone-event packet — used to dedupe
    /// repeated end-of-event packets that share the same RTP timestamp.
    last_event_ts: Option<u32>,
}

impl Participant {
    fn snapshot_frame(&self) -> Option<Vec<i16>> {
        let frame = self.latest_frame.try_lock().ok()?;
        if frame.is_empty() {
            None
        } else {
            Some(frame.clone())
        }
    }

    /// Update internal mute state on a parsed `*6` sequence.
    /// Returns `Some(new_muted)` when toggled, `None` otherwise.
    async fn record_dtmf(&self, event: u8) -> Option<bool> {
        let mut state = self.dtmf_state.lock().await;
        let now = Instant::now();
        let prev = state.last_digit.take();
        state.last_digit = Some((event, now));

        let prev_was_star_recent =
            matches!(prev, Some((10, t)) if now.duration_since(t) < STAR6_TIMEOUT);
        if prev_was_star_recent && event == 6 {
            state.last_digit = None;
            let now_muted = !self.muted.fetch_xor(true, Ordering::SeqCst);
            return Some(now_muted);
        }
        None
    }
}

struct Room {
    participants: HashMap<String, Arc<Participant>>,
    mixer_task: Option<JoinHandle<()>>,
}

/// Shared conference media manager. Cheap to clone (`Arc` inside).
#[derive(Clone)]
pub struct ConferenceMedia {
    rooms: Arc<Mutex<HashMap<u64, Room>>>,
    public_ip: Arc<String>,
    port_min: u16,
    port_max: u16,
    port_counter: Arc<AtomicU32>,
}

/// Returned from [`ConferenceMedia::join`].
#[derive(Debug, Clone)]
pub struct JoinedParticipant {
    pub relay_port: u16,
}

impl ConferenceMedia {
    pub fn new(public_ip: String, port_min: u16, port_max: u16) -> Self {
        Self {
            rooms: Arc::new(Mutex::new(HashMap::new())),
            public_ip: Arc::new(public_ip),
            port_min,
            port_max,
            port_counter: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn public_ip(&self) -> &str {
        &self.public_ip
    }

    /// Allocate a UDP socket and register a participant. Spawns a per-room
    /// mixer ticker on the first participant.
    pub async fn join(
        &self,
        room_id: u64,
        call_id: String,
        codec: ConferenceCodec,
        audio_pt: u8,
        telephone_event_pt: Option<u8>,
    ) -> Result<JoinedParticipant> {
        let socket = self.bind_socket().await?;
        let relay_port = socket.local_addr()?.port();
        let socket = Arc::new(socket);

        let participant = Arc::new(Participant {
            call_id: call_id.clone(),
            relay_port,
            codec,
            audio_pt,
            telephone_event_pt,
            muted: AtomicBool::new(false),
            peer: Mutex::new(None),
            latest_frame: Mutex::new(Vec::new()),
            socket: socket.clone(),
            out_seq: AtomicU16::new(rand_u16()),
            out_ts: AtomicU32::new(rand_u32()),
            out_ssrc: rand_u32(),
            dtmf_state: Mutex::new(DtmfState::default()),
            recv_task: Mutex::new(None),
        });

        let recv_handle = {
            let participant = participant.clone();
            tokio::spawn(async move {
                run_recv_loop(participant).await;
            })
        };
        *participant.recv_task.lock().await = Some(recv_handle);

        let mut rooms = self.rooms.lock().await;
        let room = rooms.entry(room_id).or_insert_with(|| Room {
            participants: HashMap::new(),
            mixer_task: None,
        });
        room.participants.insert(call_id.clone(), participant);

        if room.mixer_task.is_none() {
            let rooms_arc = self.rooms.clone();
            let task = tokio::spawn(async move {
                run_mixer_loop(rooms_arc, room_id).await;
            });
            room.mixer_task = Some(task);
        }

        debug!(
            "Conference room {} added participant {} on port {}",
            room_id, call_id, relay_port
        );
        Ok(JoinedParticipant { relay_port })
    }

    /// Remove a participant. If the room becomes empty, stop the mixer task.
    pub async fn leave(&self, room_id: u64, call_id: &str) {
        let mut rooms = self.rooms.lock().await;
        let Some(room) = rooms.get_mut(&room_id) else {
            return;
        };
        if let Some(participant) = room.participants.remove(call_id)
            && let Some(handle) = participant.recv_task.lock().await.take()
        {
            handle.abort();
        }
        if room.participants.is_empty() {
            if let Some(handle) = room.mixer_task.take() {
                handle.abort();
            }
            rooms.remove(&room_id);
            debug!("Conference room {} closed (empty)", room_id);
        }
    }

    /// Force-set mute via admin API, ignoring DTMF.
    #[allow(dead_code)]
    pub async fn set_muted(&self, room_id: u64, call_id: &str, muted: bool) -> bool {
        let rooms = self.rooms.lock().await;
        if let Some(room) = rooms.get(&room_id)
            && let Some(p) = room.participants.get(call_id)
        {
            p.muted.store(muted, Ordering::SeqCst);
            return true;
        }
        false
    }

    /// Feed a SIP-INFO-derived DTMF event through the same `*6` state machine
    /// used by inbound RFC 2833 packets. Returns `Some(now_muted)` when the
    /// `*` -> `6` sequence completes.
    pub async fn record_dtmf_for(&self, room_id: u64, call_id: &str, event: u8) -> Option<bool> {
        let participant = {
            let rooms = self.rooms.lock().await;
            rooms.get(&room_id)?.participants.get(call_id)?.clone()
        };
        participant.record_dtmf(event).await
    }

    pub async fn participant_count(&self, room_id: u64) -> usize {
        self.rooms
            .lock()
            .await
            .get(&room_id)
            .map(|r| r.participants.len())
            .unwrap_or(0)
    }

    /// Drop all rooms (used by cleanup tasks on startup).
    pub async fn shutdown_all(&self) {
        let mut rooms = self.rooms.lock().await;
        for (_, room) in rooms.drain() {
            if let Some(handle) = room.mixer_task {
                handle.abort();
            }
            for (_, p) in room.participants {
                if let Some(handle) = p.recv_task.lock().await.take() {
                    handle.abort();
                }
            }
        }
    }

    async fn bind_socket(&self) -> Result<UdpSocket> {
        let range = self.port_max.saturating_sub(self.port_min) as u32;
        if range == 0 {
            return Err(anyhow!("conference RTP port range is empty"));
        }
        for _ in 0..range {
            let offset = self.port_counter.fetch_add(1, Ordering::Relaxed) % range;
            let port = self.port_min + offset as u16;
            if let Ok(sock) = UdpSocket::bind(format!("0.0.0.0:{}", port)).await {
                return Ok(sock);
            }
        }
        Err(anyhow!(
            "no available conference RTP port in range {}-{}",
            self.port_min,
            self.port_max
        ))
    }
}

async fn run_recv_loop(participant: Arc<Participant>) {
    let mut buf = vec![0u8; RTP_BUF_SIZE];
    let socket = participant.socket.clone();
    loop {
        let (len, src) = match socket.recv_from(&mut buf).await {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    "conference RTP recv error on port {}: {}",
                    participant.relay_port, e
                );
                break;
            }
        };
        let packet = &buf[..len];
        if packet.len() < RTP_HEADER_LEN {
            continue;
        }
        *participant.peer.lock().await = Some(src);

        let pt = packet[1] & 0x7F;
        let payload_offset = rtp_payload_offset(packet);
        if payload_offset >= packet.len() {
            continue;
        }
        let payload = &packet[payload_offset..];

        if Some(pt) == participant.telephone_event_pt {
            handle_dtmf_packet(&participant, packet, payload).await;
            continue;
        }

        if pt != participant.audio_pt {
            continue;
        }
        let pcm = match participant.codec {
            ConferenceCodec::Pcmu => g711::decode_ulaw(payload),
            ConferenceCodec::Pcma => g711::decode_alaw(payload),
        };
        if pcm.is_empty() {
            continue;
        }
        let mut frame = pcm;
        if frame.len() > SAMPLES_PER_FRAME {
            frame.truncate(SAMPLES_PER_FRAME);
        } else if frame.len() < SAMPLES_PER_FRAME {
            frame.resize(SAMPLES_PER_FRAME, 0);
        }
        *participant.latest_frame.lock().await = frame;
    }
}

async fn handle_dtmf_packet(participant: &Arc<Participant>, packet: &[u8], payload: &[u8]) {
    if payload.len() < 4 {
        return;
    }
    let event = payload[0] & 0x0F;
    let end_marker = (payload[1] & 0x80) != 0;
    let rtp_ts = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);

    let already_handled = {
        let state = participant.dtmf_state.lock().await;
        state.last_event_ts == Some(rtp_ts)
    };
    if !end_marker || already_handled {
        return;
    }
    {
        let mut s = participant.dtmf_state.lock().await;
        s.last_event_ts = Some(rtp_ts);
    }
    if let Some(now_muted) = participant.record_dtmf(event).await {
        debug!(
            "Conference participant {} toggled mute via DTMF *6 -> {}",
            participant.call_id, now_muted
        );
    }
}

async fn run_mixer_loop(rooms: Arc<Mutex<HashMap<u64, Room>>>, room_id: u64) {
    let mut ticker = interval(FRAME_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;

        let snapshot: Vec<Arc<Participant>> = {
            let rooms = rooms.lock().await;
            match rooms.get(&room_id) {
                Some(room) => room.participants.values().cloned().collect(),
                None => return,
            }
        };
        if snapshot.is_empty() {
            return;
        }

        let mut frames: Vec<(Arc<Participant>, Option<Vec<i16>>)> =
            Vec::with_capacity(snapshot.len());
        for p in &snapshot {
            let frame = if p.muted.load(Ordering::SeqCst) {
                None
            } else {
                p.snapshot_frame()
            };
            frames.push((p.clone(), frame));
        }

        for (receiver, _) in &frames {
            let peer = match *receiver.peer.lock().await {
                Some(addr) => addr,
                None => continue,
            };
            let mixed = mix_for_receiver(receiver, &frames);
            let payload = match receiver.codec {
                ConferenceCodec::Pcmu => g711::encode_ulaw(&mixed),
                ConferenceCodec::Pcma => g711::encode_alaw(&mixed),
            };
            let seq = receiver.out_seq.fetch_add(1, Ordering::SeqCst);
            let ts = receiver
                .out_ts
                .fetch_add(SAMPLES_PER_FRAME as u32, Ordering::SeqCst);
            let packet = build_rtp_packet(receiver.audio_pt, seq, ts, receiver.out_ssrc, &payload);
            let _ = receiver.socket.send_to(&packet, peer).await;
        }
    }
}

fn mix_for_receiver(
    receiver: &Arc<Participant>,
    frames: &[(Arc<Participant>, Option<Vec<i16>>)],
) -> Vec<i16> {
    let mut acc = vec![0i32; SAMPLES_PER_FRAME];
    for (other, frame) in frames {
        if Arc::ptr_eq(other, receiver) {
            continue;
        }
        if let Some(f) = frame {
            for (a, s) in acc.iter_mut().zip(f.iter()) {
                *a += *s as i32;
            }
        }
    }
    acc.iter()
        .map(|v| (*v).clamp(i16::MIN as i32, i16::MAX as i32) as i16)
        .collect()
}

fn rtp_payload_offset(packet: &[u8]) -> usize {
    let cc = (packet[0] & 0x0F) as usize;
    let mut offset = RTP_HEADER_LEN + 4 * cc;
    let extension = (packet[0] & 0x10) != 0;
    if extension && packet.len() >= offset + 4 {
        let ext_len = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]) as usize;
        offset += 4 + 4 * ext_len;
    }
    offset
}

fn build_rtp_packet(pt: u8, seq: u16, ts: u32, ssrc: u32, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(RTP_HEADER_LEN + payload.len());
    buf.push(0x80); // V=2, P=0, X=0, CC=0
    buf.push(pt & 0x7F);
    buf.extend_from_slice(&seq.to_be_bytes());
    buf.extend_from_slice(&ts.to_be_bytes());
    buf.extend_from_slice(&ssrc.to_be_bytes());
    buf.extend_from_slice(payload);
    buf
}

fn rand_u16() -> u16 {
    use rand::Rng;
    rand::rng().random()
}

fn rand_u32() -> u32 {
    use rand::Rng;
    rand::rng().random()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn dummy_participant(codec: ConferenceCodec) -> Arc<Participant> {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        Arc::new(Participant {
            call_id: "test".into(),
            relay_port: 0,
            codec,
            audio_pt: codec.static_pt(),
            telephone_event_pt: Some(101),
            muted: AtomicBool::new(false),
            peer: Mutex::new(None),
            latest_frame: Mutex::new(Vec::new()),
            socket,
            out_seq: AtomicU16::new(0),
            out_ts: AtomicU32::new(0),
            out_ssrc: 0,
            dtmf_state: Mutex::new(DtmfState::default()),
            recv_task: Mutex::new(None),
        })
    }

    #[tokio::test]
    async fn dtmf_star_then_six_toggles_mute() {
        let p = dummy_participant(ConferenceCodec::Pcmu).await;
        assert!(!p.muted.load(Ordering::SeqCst));
        assert!(p.record_dtmf(10).await.is_none());
        let toggled = p.record_dtmf(6).await;
        assert_eq!(toggled, Some(true));
        assert!(p.muted.load(Ordering::SeqCst));

        assert!(p.record_dtmf(10).await.is_none());
        let toggled = p.record_dtmf(6).await;
        assert_eq!(toggled, Some(false));
        assert!(!p.muted.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn dtmf_stray_six_does_not_toggle() {
        let p = dummy_participant(ConferenceCodec::Pcmu).await;
        assert!(p.record_dtmf(6).await.is_none());
        assert!(!p.muted.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn dtmf_other_digit_breaks_sequence() {
        let p = dummy_participant(ConferenceCodec::Pcmu).await;
        assert!(p.record_dtmf(10).await.is_none());
        assert!(p.record_dtmf(3).await.is_none());
        assert!(p.record_dtmf(6).await.is_none());
        assert!(!p.muted.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn mixer_excludes_self_and_muted() {
        let a = dummy_participant(ConferenceCodec::Pcmu).await;
        let b = dummy_participant(ConferenceCodec::Pcmu).await;
        let c = dummy_participant(ConferenceCodec::Pcmu).await;
        c.muted.store(true, Ordering::SeqCst);

        let frames = vec![
            (a.clone(), Some(vec![1000i16; SAMPLES_PER_FRAME])),
            (b.clone(), Some(vec![2000i16; SAMPLES_PER_FRAME])),
            (c.clone(), None),
        ];

        let mixed_for_a = mix_for_receiver(&a, &frames);
        assert_eq!(mixed_for_a[0], 2000);

        let mixed_for_b = mix_for_receiver(&b, &frames);
        assert_eq!(mixed_for_b[0], 1000);
    }

    #[tokio::test]
    async fn mixer_clips_to_i16() {
        let a = dummy_participant(ConferenceCodec::Pcmu).await;
        let b = dummy_participant(ConferenceCodec::Pcmu).await;
        let c = dummy_participant(ConferenceCodec::Pcmu).await;
        let frames = vec![
            (a.clone(), Some(vec![20000i16; SAMPLES_PER_FRAME])),
            (b.clone(), Some(vec![20000i16; SAMPLES_PER_FRAME])),
            (c.clone(), None),
        ];
        let mixed_for_c = mix_for_receiver(&c, &frames);
        assert_eq!(mixed_for_c[0], i16::MAX);
    }

    #[test]
    fn rtp_packet_has_correct_header() {
        let payload = vec![0xFFu8; 160];
        let pkt = build_rtp_packet(0, 0x1234, 0xCAFEBABE, 0xDEADBEEF, &payload);
        assert_eq!(pkt.len(), 12 + 160);
        assert_eq!(pkt[0], 0x80);
        assert_eq!(pkt[1], 0);
        assert_eq!(&pkt[2..4], &0x1234u16.to_be_bytes());
        assert_eq!(&pkt[4..8], &0xCAFEBABEu32.to_be_bytes());
        assert_eq!(&pkt[8..12], &0xDEADBEEFu32.to_be_bytes());
    }

    #[test]
    fn rtp_payload_offset_skips_csrcs_and_extension() {
        let mut pkt = vec![0x92u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]; // V=2, X=1, CC=2
        pkt.extend_from_slice(&[0u8; 8]); // 2 CSRC entries
        pkt.extend_from_slice(&[0, 0, 0, 1]); // extension header: 1 word follows
        pkt.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]); // ext data
        pkt.push(0x42); // payload
        let off = rtp_payload_offset(&pkt);
        assert_eq!(pkt[off], 0x42);
    }
}
