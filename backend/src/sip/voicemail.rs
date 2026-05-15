//! Local SIP voicemail endpoint.
//!
//! This module owns voicemail-specific local SIP responses and in-memory call
//! state.

use anyhow::Result;
use rand::Rng;
use sqlx::MySqlPool;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{Duration, sleep};
use tracing::{debug, info, warn};

use super::handler::{DialogStores, SipMessage, base_response, extract_uri, uri_username};
use super::media::MediaRelay;
use super::proxy::{CALLER_ACCOUNT_EXISTS_SQL, build_forwarded_cancel_for_target};
use super::transport::TransportRegistry;
use super::voicemail_media::{
    VoicemailDtmf, VoicemailMedia, parse_dtmf_relay, play_wav, record_to_storage,
};
use super::voicemail_mwi::{VoicemailMwi, registered_source_matches};
use super::voicemail_sdp::{build_answer, negotiate_offer};
use super::webrtc_gateway::WebRtcGateway;
use crate::config::Config;
use crate::storage::voicemail::LocalVoicemailStorage;

#[derive(Clone)]
pub struct Voicemail {
    pool: MySqlPool,
    cfg: Config,
    media: VoicemailMedia,
    mwi: VoicemailMwi,
    active: Arc<Mutex<HashMap<String, VoicemailCall>>>,
    no_answer: Arc<Mutex<HashMap<String, NoAnswerTimerEntry>>>,
    recording_state: Arc<Mutex<RecordingCancelState>>,
    access_pending: Arc<Mutex<HashSet<String>>>,
}

#[derive(Debug)]
enum NoAnswerTimerEntry {
    Ringing(tokio::task::JoinHandle<()>),
    Firing,
    Canceled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoAnswerTimerCancel {
    Canceled,
    AlreadyFired,
    NotFound,
}

#[derive(Debug, Default)]
struct RecordingCancelState {
    recording: HashSet<String>,
    canceled: HashSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordingPersistenceDecision {
    Insert,
    DiscardCanceled,
    DiscardEmpty,
}

#[derive(Debug, Clone)]
enum VoicemailMode {
    Recording { box_id: u64, mailbox: String },
    Playback { mailbox: String },
}

#[derive(Debug, Clone)]
struct VoicemailCall {
    mode: VoicemailMode,
    caller: String,
    callee: String,
}

#[derive(Debug, Clone)]
struct RecordingStart {
    call_id: String,
    box_id: u64,
    mailbox: String,
    domain: String,
    caller: String,
    callee: String,
    max_message_secs: u32,
}

#[derive(Debug, Clone)]
pub struct MailboxSettings {
    pub id: u64,
    pub username: String,
    pub domain: String,
    pub no_answer_secs: u32,
    pub max_message_secs: u32,
    pub max_messages: u32,
    pub greeting_storage_key: Option<String>,
}

impl Voicemail {
    pub fn new(pool: MySqlPool, cfg: Config, media: VoicemailMedia, mwi: VoicemailMwi) -> Self {
        Self {
            pool,
            cfg,
            media,
            mwi,
            active: Arc::new(Mutex::new(HashMap::new())),
            no_answer: Arc::new(Mutex::new(HashMap::new())),
            recording_state: Arc::new(Mutex::new(RecordingCancelState::default())),
            access_pending: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn mwi(&self) -> &VoicemailMwi {
        &self.mwi
    }

    pub async fn is_voicemail_call(&self, call_id: &str) -> bool {
        self.active.lock().await.contains_key(call_id)
    }

    pub fn is_access_invite(&self, msg: &SipMessage) -> bool {
        if !matches!(msg.method.as_deref(), Some("INVITE")) {
            return false;
        }
        msg.request_uri
            .as_deref()
            .and_then(uri_username)
            .is_some_and(|target| {
                is_voicemail_access_target(&target, &self.cfg.server.voicemail_access_extension)
            })
    }

    pub async fn lookup_enabled_box(
        &self,
        username: &str,
        domain: &str,
    ) -> Option<MailboxSettings> {
        let row = sqlx::query_as::<_, (u64, String, String, u32, u32, u32, Option<String>)>(
            "SELECT id, username, domain, no_answer_secs, max_message_secs, max_messages, greeting_storage_key
             FROM sip_voicemail_boxes
             WHERE username = ? AND domain = ? AND enabled = 1",
        )
        .bind(username)
        .bind(domain)
        .fetch_optional(&self.pool)
        .await;

        match row {
            Ok(Some((
                id,
                username,
                domain,
                no_answer_secs,
                max_message_secs,
                max_messages,
                greeting_storage_key,
            ))) => Some(MailboxSettings {
                id,
                username,
                domain,
                no_answer_secs,
                max_message_secs,
                max_messages,
                greeting_storage_key,
            }),
            Ok(None) => None,
            Err(e) => {
                warn!(
                    "Failed to look up voicemail mailbox {}@{}: {}",
                    username, domain, e
                );
                None
            }
        }
    }

    pub async fn handle_access_invite(&self, msg: &SipMessage, src: SocketAddr) -> Result<String> {
        if !self.is_access_invite(msg) {
            return Ok(base_response(msg, 404, "Not Found").build());
        }

        let call_id = msg.call_id().unwrap_or("").to_string();
        if call_id.is_empty() {
            return Ok(base_response(msg, 400, "Bad Request").build());
        }
        let tracked_access =
            track_access_candidate(&call_id, &self.active, &self.access_pending).await;
        if let Some((status, reason)) = duplicate_access_candidate_response(tracked_access) {
            warn!("Duplicate voicemail access INVITE rejected for call_id={call_id}");
            return Ok(base_response(msg, status, reason).build());
        }

        let caller = msg
            .from_header()
            .and_then(extract_uri)
            .and_then(|u| uri_username(&u))
            .unwrap_or_default();
        if caller.is_empty() {
            discard_access_candidate(&call_id, &self.access_pending, tracked_access).await;
            return Ok(base_response(msg, 400, "Bad Request").build());
        }

        let domain = self.cfg.server.sip_domain.clone();
        let Some(settings) = self.lookup_enabled_box(&caller, &domain).await else {
            warn!(
                "Voicemail access rejected for unknown mailbox {}@{}",
                caller, domain
            );
            discard_access_candidate(&call_id, &self.access_pending, tracked_access).await;
            return Ok(base_response(msg, 404, "Not Found").build());
        };
        if !self
            .caller_source_is_registered(&caller, &domain, src)
            .await?
        {
            warn!(
                "Voicemail access rejected from unregistered source {} for {}@{}",
                src, caller, domain
            );
            discard_access_candidate(&call_id, &self.access_pending, tracked_access).await;
            return Ok(base_response(msg, 403, "Forbidden").build());
        }

        let negotiation = match negotiate_offer(&msg.body) {
            Ok(n) => n,
            Err(e) => {
                warn!("Voicemail access SDP rejected for {}: {}", caller, e);
                discard_access_candidate(&call_id, &self.access_pending, tracked_access).await;
                return Ok(base_response(msg, 488, "Not Acceptable Here").build());
            }
        };

        let relay_port = match self
            .media
            .allocate(
                call_id.clone(),
                negotiation.codec,
                negotiation.audio_pt,
                negotiation.telephone_event_pt,
            )
            .await
        {
            Ok(port) => port,
            Err(e) => {
                warn!("Voicemail access media allocation failed: {}", e);
                discard_access_candidate(&call_id, &self.access_pending, tracked_access).await;
                return Ok(base_response(msg, 500, "Internal Server Error").build());
            }
        };

        let response = self.build_local_ok(
            msg,
            &call_id,
            &self.cfg.server.voicemail_access_extension,
            relay_port,
            &negotiation,
        );

        self.active.lock().await.insert(
            call_id.clone(),
            VoicemailCall {
                mode: VoicemailMode::Playback {
                    mailbox: caller.clone(),
                },
                caller: caller.clone(),
                callee: settings.username.clone(),
            },
        );
        discard_access_candidate(&call_id, &self.access_pending, tracked_access).await;

        if let Some(greeting_key) = settings.greeting_storage_key.clone() {
            let storage = LocalVoicemailStorage::new(PathBuf::from(
                self.cfg.server.voicemail_storage_dir.clone(),
            ));
            let play_call_id = call_id.clone();
            tokio::spawn(async move {
                if let Err(e) = play_wav(&play_call_id, &greeting_key, &storage).await {
                    debug!("Voicemail access prompt playback skipped/failed: {}", e);
                }
            });
        } else {
            debug!(
                "Voicemail access call {} established without playback prompt; IVR is future work",
                call_id
            );
        }

        info!(
            "Voicemail access started for {} (call_id={})",
            caller, call_id
        );
        Ok(response)
    }

    pub async fn handle_delivery_invite(
        &self,
        msg: &SipMessage,
        _src: SocketAddr,
        callee: &str,
    ) -> Result<String> {
        let call_id = msg.call_id().unwrap_or("").to_string();
        if call_id.is_empty() || callee.is_empty() {
            return Ok(base_response(msg, 400, "Bad Request").build());
        }
        let tracked_recording = track_recording_candidate(&call_id, &self.recording_state).await;
        if let Some((status, reason)) = duplicate_recording_candidate_response(tracked_recording) {
            warn!("Duplicate voicemail delivery INVITE rejected for call_id={call_id}");
            return Ok(base_response(msg, status, reason).build());
        }

        let domain = self.cfg.server.sip_domain.clone();
        let Some(caller) = delivery_caller(msg.from_header()) else {
            discard_recording_candidate(&call_id, &self.recording_state, tracked_recording).await;
            warn!("Voicemail delivery rejected missing/unparseable From header");
            return Ok(base_response(msg, 400, "Bad Request").build());
        };
        let caller_ok: Option<(i32,)> = match sqlx::query_as(CALLER_ACCOUNT_EXISTS_SQL)
            .bind(&caller)
            .bind(&domain)
            .fetch_optional(&self.pool)
            .await
        {
            Ok(row) => row,
            Err(e) => {
                discard_recording_candidate(&call_id, &self.recording_state, tracked_recording)
                    .await;
                return Err(e.into());
            }
        };
        if caller_ok.is_none() {
            discard_recording_candidate(&call_id, &self.recording_state, tracked_recording).await;
            warn!(
                "Voicemail delivery from unknown caller {}@{} to {}",
                caller, domain, callee
            );
            return Ok(base_response(msg, 403, "Forbidden").build());
        }

        let Some(settings) = self.lookup_enabled_box(callee, &domain).await else {
            discard_recording_candidate(&call_id, &self.recording_state, tracked_recording).await;
            warn!("Voicemail delivery requested for disabled/unknown mailbox {callee}@{domain}");
            return Ok(base_response(msg, 404, "Not Found").build());
        };

        let mailbox_is_full = match self
            .mailbox_is_full(settings.id, settings.max_messages)
            .await
        {
            Ok(is_full) => is_full,
            Err(e) => {
                discard_recording_candidate(&call_id, &self.recording_state, tracked_recording)
                    .await;
                return Err(e);
            }
        };
        if mailbox_is_full {
            discard_recording_candidate(&call_id, &self.recording_state, tracked_recording).await;
            warn!("Voicemail mailbox {callee}@{domain} is full");
            return Ok(base_response(msg, 486, "Busy Here").build());
        }

        let negotiation = match negotiate_offer(&msg.body) {
            Ok(n) => n,
            Err(e) => {
                discard_recording_candidate(&call_id, &self.recording_state, tracked_recording)
                    .await;
                warn!("Voicemail delivery SDP rejected for {}: {}", callee, e);
                return Ok(base_response(msg, 488, "Not Acceptable Here").build());
            }
        };

        let relay_port = match self
            .media
            .allocate(
                call_id.clone(),
                negotiation.codec,
                negotiation.audio_pt,
                negotiation.telephone_event_pt,
            )
            .await
        {
            Ok(port) => port,
            Err(e) => {
                discard_recording_candidate(&call_id, &self.recording_state, tracked_recording)
                    .await;
                warn!("Voicemail delivery media allocation failed: {}", e);
                return Ok(base_response(msg, 500, "Internal Server Error").build());
            }
        };

        let response = self.build_local_ok(msg, &call_id, callee, relay_port, &negotiation);
        self.active.lock().await.insert(
            call_id.clone(),
            VoicemailCall {
                mode: VoicemailMode::Recording {
                    box_id: settings.id,
                    mailbox: settings.username.clone(),
                },
                caller: caller.clone(),
                callee: callee.to_string(),
            },
        );

        self.spawn_recording_task(RecordingStart {
            call_id: call_id.clone(),
            box_id: settings.id,
            mailbox: settings.username.clone(),
            domain: domain.clone(),
            caller: caller.clone(),
            callee: callee.to_string(),
            max_message_secs: settings.max_message_secs,
        });

        info!(
            "Voicemail delivery started for {} from {} (call_id={}, relay_port={})",
            callee, caller, call_id, relay_port
        );
        Ok(response)
    }

    pub async fn handle_ack(&self, msg: &SipMessage) {
        let call_id = msg.call_id().unwrap_or("");
        self.cancel_no_answer_timer(call_id).await;
        debug!(
            "Voicemail ACK received for {}; recording tasks start immediately after 200 OK in this MVP",
            call_id
        );
    }

    pub async fn handle_bye(&self, msg: &SipMessage) -> Result<String> {
        let call_id = msg.call_id().unwrap_or("").to_string();
        let removed = self.active.lock().await.remove(&call_id);
        self.cancel_no_answer_timer(&call_id).await;
        self.media.remove(&call_id).await;
        if let Some(call) = removed {
            info!(
                "Voicemail call ended: {} -> {} (call_id={})",
                call.caller, call.callee, call_id
            );
        }
        Ok(base_response(msg, 200, "OK").build())
    }

    pub async fn handle_cancel(&self, msg: &SipMessage) -> Result<String> {
        let call_id = msg.call_id().unwrap_or("").to_string();
        if !call_id.is_empty() && mark_recording_canceled(&call_id, &self.recording_state).await {
            debug!("Voicemail recording marked canceled for {}", call_id);
        }
        self.active.lock().await.remove(&call_id);
        self.cancel_no_answer_timer(&call_id).await;
        self.media.remove(&call_id).await;
        // This endpoint can only acknowledge the CANCEL here; routing-layer
        // integration must send the matching 487 for the INVITE transaction.
        Ok(base_response(msg, 200, "OK").build())
    }

    pub async fn handle_info(&self, msg: &SipMessage) -> Result<String> {
        let call_id = msg.call_id().unwrap_or("").to_string();
        let call = { self.active.lock().await.get(&call_id).cloned() };
        let Some(call) = call else {
            return Ok(base_response(msg, 481, "Call/Transaction Does Not Exist").build());
        };

        if let Some(dtmf) = parse_dtmf_relay(&msg.body) {
            match (&call.mode, dtmf) {
                (VoicemailMode::Recording { box_id, mailbox }, VoicemailDtmf::Pound) => {
                    info!(
                        "Voicemail recording for {} (box {}) got # stop signal",
                        mailbox, box_id
                    );
                    self.media.remove(&call_id).await;
                }
                (VoicemailMode::Playback { mailbox }, VoicemailDtmf::One) => {
                    debug!(
                        "Voicemail playback for {} got 1; navigation is future work",
                        mailbox
                    );
                }
                (VoicemailMode::Playback { mailbox }, VoicemailDtmf::Seven) => {
                    debug!(
                        "Voicemail playback for {} got 7; delete is not implemented yet",
                        mailbox
                    );
                }
                (VoicemailMode::Playback { mailbox }, VoicemailDtmf::Nine) => {
                    debug!(
                        "Voicemail playback for {} got 9; save is not implemented yet",
                        mailbox
                    );
                }
                (VoicemailMode::Playback { mailbox }, other) => {
                    debug!("Voicemail playback for {} got {:?}", mailbox, other);
                }
                (VoicemailMode::Recording { box_id, mailbox }, other) => {
                    debug!(
                        "Voicemail recording for {} (box {}) got {:?}",
                        mailbox, box_id, other
                    );
                }
            }
        }
        Ok(base_response(msg, 200, "OK").build())
    }

    pub async fn cancel_no_answer_timer(&self, call_id: &str) -> NoAnswerTimerCancel {
        let mut no_answer = self.no_answer.lock().await;
        match no_answer.remove(call_id) {
            Some(NoAnswerTimerEntry::Ringing(handle)) => {
                handle.abort();
                debug!("Cancelled voicemail no-answer timer for {}", call_id);
                NoAnswerTimerCancel::Canceled
            }
            Some(NoAnswerTimerEntry::Firing) => {
                no_answer.insert(call_id.to_string(), NoAnswerTimerEntry::Firing);
                debug!("Voicemail no-answer timer already fired for {}", call_id);
                NoAnswerTimerCancel::AlreadyFired
            }
            Some(NoAnswerTimerEntry::Canceled) => {
                no_answer.insert(call_id.to_string(), NoAnswerTimerEntry::Canceled);
                NoAnswerTimerCancel::Canceled
            }
            None => NoAnswerTimerCancel::NotFound,
        }
    }

    pub async fn cancel_no_answer_timer_for_caller_cancel(
        &self,
        call_id: &str,
    ) -> NoAnswerTimerCancel {
        let mut no_answer = self.no_answer.lock().await;
        if mark_fired_no_answer_timer_canceled(&mut no_answer, call_id) {
            debug!(
                "Marked fired voicemail no-answer timer canceled for {}",
                call_id
            );
            return NoAnswerTimerCancel::Canceled;
        }

        match no_answer.remove(call_id) {
            Some(NoAnswerTimerEntry::Ringing(handle)) => {
                handle.abort();
                debug!("Cancelled voicemail no-answer timer for {}", call_id);
                NoAnswerTimerCancel::Canceled
            }
            Some(NoAnswerTimerEntry::Canceled) => {
                no_answer.insert(call_id.to_string(), NoAnswerTimerEntry::Canceled);
                NoAnswerTimerCancel::Canceled
            }
            None => NoAnswerTimerCancel::NotFound,
            Some(NoAnswerTimerEntry::Firing) => unreachable!("firing state handled above"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn start_no_answer_timer(
        &self,
        msg: SipMessage,
        src: SocketAddr,
        target_addr: SocketAddr,
        target_uri: String,
        callee: String,
        no_answer_secs: u32,
        transport_registry: TransportRegistry,
        socket: Arc<UdpSocket>,
        dialog_stores: DialogStores,
        media_relay: MediaRelay,
        webrtc_gateway: Arc<WebRtcGateway>,
    ) {
        let call_id = msg.call_id().unwrap_or("").to_string();
        if call_id.is_empty() || callee.is_empty() {
            warn!("Skipping voicemail no-answer timer with missing call-id or callee");
            return;
        }

        let voicemail = self.clone();
        let timers = self.no_answer.clone();
        let domain = self.cfg.server.sip_domain.clone();
        let max_fwd = msg
            .header("max-forwards")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(70)
            .saturating_sub(1);
        let delay = Duration::from_secs(u64::from(no_answer_secs.max(1)));
        let timer_call_id = call_id.clone();

        let handle = tokio::spawn(async move {
            sleep(delay).await;

            let should_fire = {
                let mut timers = timers.lock().await;
                claim_no_answer_timer_to_fire(&mut timers, &timer_call_id)
            };
            if !should_fire {
                debug!(
                    "Voicemail no-answer timer lost cancellation race for {}",
                    timer_call_id
                );
                return;
            }

            info!(
                "Voicemail no-answer timer fired for {}; cancelling ringing leg at {}",
                timer_call_id, target_addr
            );
            let cancel = build_forwarded_cancel_for_target(&msg, &target_uri, max_fwd, &domain);
            if !transport_registry.send(target_addr, cancel.clone())
                && let Err(e) = socket.send_to(cancel.as_bytes(), target_addr).await
            {
                warn!(
                    "Failed to send no-answer CANCEL for {} to {}: {}",
                    timer_call_id, target_addr, e
                );
            }

            dialog_stores.pending.lock().await.remove(&timer_call_id);
            dialog_stores.active.lock().await.remove(&timer_call_id);
            media_relay.remove_session(&timer_call_id).await;
            webrtc_gateway.remove_session(&timer_call_id).await;

            if !voicemail
                .no_answer_timer_should_continue(&timer_call_id)
                .await
            {
                debug!(
                    "Suppressing no-answer voicemail setup for canceled call {}",
                    timer_call_id
                );
                voicemail.finish_no_answer_timer(&timer_call_id).await;
                return;
            }

            let response = match voicemail.handle_delivery_invite(&msg, src, &callee).await {
                Ok(response) => response,
                Err(e) => {
                    warn!(
                        "Failed to build no-answer voicemail response for {}: {}",
                        timer_call_id, e
                    );
                    base_response(&msg, 500, "Internal Server Error").build()
                }
            };

            if !voicemail
                .no_answer_timer_should_continue(&timer_call_id)
                .await
            {
                debug!(
                    "Suppressing no-answer voicemail response for canceled call {}",
                    timer_call_id
                );
                voicemail
                    .cleanup_suppressed_no_answer_delivery(&timer_call_id)
                    .await;
                voicemail.finish_no_answer_timer(&timer_call_id).await;
                return;
            }

            if !transport_registry.send(src, response.clone())
                && let Err(e) = socket.send_to(response.as_bytes(), src).await
            {
                warn!(
                    "Failed to send no-answer voicemail response for {} to {}: {}",
                    timer_call_id, src, e
                );
            }

            voicemail.finish_no_answer_timer(&timer_call_id).await;
        });

        if let Some(old) = self
            .no_answer
            .lock()
            .await
            .insert(call_id.clone(), NoAnswerTimerEntry::Ringing(handle))
        {
            if let NoAnswerTimerEntry::Ringing(old) = old {
                old.abort();
            }
            debug!(
                "Replaced existing voicemail no-answer timer for {}",
                call_id
            );
        } else {
            debug!("Started voicemail no-answer timer for {}", call_id);
        }
    }

    async fn no_answer_timer_should_continue(&self, call_id: &str) -> bool {
        let no_answer = self.no_answer.lock().await;
        no_answer_timer_should_continue(&no_answer, call_id)
    }

    async fn finish_no_answer_timer(&self, call_id: &str) {
        self.no_answer.lock().await.remove(call_id);
    }

    async fn cleanup_suppressed_no_answer_delivery(&self, call_id: &str) {
        if mark_recording_canceled(call_id, &self.recording_state).await {
            debug!(
                "Voicemail recording marked canceled after suppressed no-answer delivery for {}",
                call_id
            );
        }
        self.active.lock().await.remove(call_id);
        self.media.remove(call_id).await;
    }

    pub async fn reconcile_on_startup(&self) -> Result<()> {
        let deleted =
            sqlx::query("DELETE FROM sip_voicemail_mwi_subscriptions WHERE expires_at <= NOW()")
                .execute(&self.pool)
                .await?
                .rows_affected();
        if deleted > 0 {
            info!("Removed {} expired voicemail MWI subscriptions", deleted);
        }

        let storage_dir = PathBuf::from(self.cfg.server.voicemail_storage_dir.clone());
        let cleaned =
            tokio::task::spawn_blocking(move || cleanup_temp_files(&storage_dir)).await??;
        if cleaned > 0 {
            info!("Removed {} temporary voicemail storage files", cleaned);
        }
        Ok(())
    }

    async fn mailbox_is_full(&self, box_id: u64, max_messages: u32) -> Result<bool> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sip_voicemail_messages
             WHERE box_id = ? AND status <> 'deleted'",
        )
        .bind(box_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0 >= i64::from(max_messages))
    }

    async fn caller_source_is_registered(
        &self,
        username: &str,
        domain: &str,
        src: SocketAddr,
    ) -> Result<bool> {
        let rows: Vec<(String, u16)> = sqlx::query_as(
            "SELECT source_ip, source_port FROM sip_registrations
             WHERE username = ? AND domain = ? AND expires_at > NOW()",
        )
        .bind(username)
        .bind(domain)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .any(|(ip, port)| registered_source_matches(&ip, port, src)))
    }

    fn build_local_ok(
        &self,
        msg: &SipMessage,
        call_id: &str,
        local_user: &str,
        relay_port: u16,
        negotiation: &super::voicemail_sdp::VoicemailNegotiation,
    ) -> String {
        let session_id = epoch_id();
        let answer = build_answer(self.media.public_ip(), relay_port, negotiation, session_id);
        let to_tag = format!("vm-{:x}", session_id);
        let to_with_tag = with_to_tag(msg.to_header().unwrap_or(""), &to_tag);
        let contact = format!(
            "<sip:{}@{}:{}>",
            local_user, self.cfg.server.public_ip, self.cfg.server.sip_port
        );

        debug!(
            "Building voicemail 200 OK for {} with relay port {}",
            call_id, relay_port
        );
        base_response_with_to(msg, 200, "OK", &to_with_tag)
            .header("Contact", &contact)
            .header("Content-Type", "application/sdp")
            .header("Allow", "INVITE, ACK, CANCEL, BYE, INFO")
            .body(&answer)
            .build()
    }

    fn spawn_recording_task(&self, recording: RecordingStart) {
        let storage = LocalVoicemailStorage::new(PathBuf::from(
            self.cfg.server.voicemail_storage_dir.clone(),
        ));
        let idle_timeout_secs = self.cfg.server.voicemail_idle_timeout_secs;
        let pool = self.pool.clone();
        let media = self.media.clone();
        let active = self.active.clone();
        let recording_state = self.recording_state.clone();
        let mwi = self.mwi.clone();

        tokio::spawn(async move {
            let result = record_to_storage(
                &recording.call_id,
                &recording.mailbox,
                recording.max_message_secs,
                idle_timeout_secs,
                &storage,
            )
            .await;

            active.lock().await.remove(&recording.call_id);
            media.remove(&recording.call_id).await;

            let should_persist =
                should_persist_recording(&recording.call_id, &recording_state).await;

            let (storage_key, duration_secs) = match result {
                Ok(recorded) => recorded,
                Err(e) => {
                    warn!(
                        "Voicemail recording failed for {}: {}",
                        recording.call_id, e
                    );
                    return;
                }
            };

            match recording_persistence_decision(should_persist, duration_secs) {
                RecordingPersistenceDecision::Insert => {}
                RecordingPersistenceDecision::DiscardCanceled => {
                    if let Err(e) = storage.delete(&storage_key).await {
                        warn!(
                            "Failed to remove canceled voicemail audio {} for {}: {}",
                            storage_key, recording.call_id, e
                        );
                    }
                    debug!(
                        "Discarded canceled voicemail recording for {}",
                        recording.call_id
                    );
                    return;
                }
                RecordingPersistenceDecision::DiscardEmpty => {
                    if let Err(e) = storage.delete(&storage_key).await {
                        warn!(
                            "Failed to remove empty voicemail audio {} for {}: {}",
                            storage_key, recording.call_id, e
                        );
                    }
                    debug!(
                        "Discarded zero-duration voicemail recording for {}",
                        recording.call_id
                    );
                    return;
                }
            }

            let insert_result = sqlx::query(
                "INSERT INTO sip_voicemail_messages
                   (box_id, caller, callee, call_id, duration_secs, storage_key, content_type, status)
                 VALUES (?, ?, ?, ?, ?, ?, 'audio/wav', 'new')",
            )
            .bind(recording.box_id)
            .bind(&recording.caller)
            .bind(&recording.callee)
            .bind(&recording.call_id)
            .bind(duration_secs)
            .bind(&storage_key)
            .execute(&pool)
            .await;

            if let Err(e) = insert_result {
                warn!(
                    "Failed to insert voicemail message metadata for {}: {}",
                    recording.call_id, e
                );
                if let Err(e) = storage.delete(&storage_key).await {
                    warn!(
                        "Failed to remove orphan voicemail audio {} after DB error: {}",
                        storage_key, e
                    );
                }
                return;
            }

            if let Err(e) = mwi
                .notify_mailbox(&recording.callee, &recording.domain)
                .await
            {
                warn!(
                    "Failed to notify voicemail MWI for {}@{}: {}",
                    recording.callee, recording.domain, e
                );
            }
            info!(
                "Stored voicemail message for {} from {} (call_id={}, duration={}s)",
                recording.callee, recording.caller, recording.call_id, duration_secs
            );
        });
    }
}

pub fn is_voicemail_access_target(target: &str, access_extension: &str) -> bool {
    target == access_extension
}

pub fn is_message_summary_event(event: Option<&str>) -> bool {
    event
        .and_then(|value| value.split(';').next())
        .is_some_and(|token| token.trim().eq_ignore_ascii_case("message-summary"))
}

fn claim_no_answer_timer_to_fire(
    timers: &mut HashMap<String, NoAnswerTimerEntry>,
    call_id: &str,
) -> bool {
    if matches!(timers.get(call_id), Some(NoAnswerTimerEntry::Ringing(_))) {
        timers.insert(call_id.to_string(), NoAnswerTimerEntry::Firing);
        true
    } else {
        false
    }
}

fn mark_fired_no_answer_timer_canceled(
    timers: &mut HashMap<String, NoAnswerTimerEntry>,
    call_id: &str,
) -> bool {
    if matches!(timers.get(call_id), Some(NoAnswerTimerEntry::Firing)) {
        timers.insert(call_id.to_string(), NoAnswerTimerEntry::Canceled);
        true
    } else {
        false
    }
}

fn no_answer_timer_should_continue(
    timers: &HashMap<String, NoAnswerTimerEntry>,
    call_id: &str,
) -> bool {
    matches!(timers.get(call_id), Some(NoAnswerTimerEntry::Firing))
}

fn with_to_tag(to: &str, tag: &str) -> String {
    if has_outer_tag_param(to) {
        to.to_string()
    } else {
        format!("{};tag={}", to, tag)
    }
}

fn has_outer_tag_param(to: &str) -> bool {
    let mut in_angle_uri = false;
    let mut in_quote = false;
    let mut escaped = false;
    let mut outside_address = String::with_capacity(to.len());

    for ch in to.chars() {
        if in_quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_quote = false;
            }
            continue;
        }

        match ch {
            '"' if !in_angle_uri => in_quote = true,
            '<' if !in_angle_uri => in_angle_uri = true,
            '>' if in_angle_uri => in_angle_uri = false,
            _ if !in_angle_uri => outside_address.push(ch),
            _ => {}
        }
    }
    outside_address.to_ascii_lowercase().contains(";tag=")
}

fn delivery_caller(from: Option<&str>) -> Option<String> {
    let caller = from.and_then(extract_uri).and_then(|u| uri_username(&u))?;
    (caller != "unknown").then_some(caller)
}

fn duplicate_recording_candidate_response(tracked_recording: bool) -> Option<(u16, &'static str)> {
    (!tracked_recording).then_some((491, "Request Pending"))
}

fn duplicate_access_candidate_response(tracked_access: bool) -> Option<(u16, &'static str)> {
    (!tracked_access).then_some((491, "Request Pending"))
}

fn recording_persistence_decision(
    should_persist: bool,
    duration_secs: u32,
) -> RecordingPersistenceDecision {
    if !should_persist {
        RecordingPersistenceDecision::DiscardCanceled
    } else if duration_secs == 0 {
        RecordingPersistenceDecision::DiscardEmpty
    } else {
        RecordingPersistenceDecision::Insert
    }
}

async fn track_access_candidate(
    call_id: &str,
    active: &Arc<Mutex<HashMap<String, VoicemailCall>>>,
    pending: &Arc<Mutex<HashSet<String>>>,
) -> bool {
    if active.lock().await.contains_key(call_id) {
        return false;
    }
    pending.lock().await.insert(call_id.to_string())
}

async fn discard_access_candidate(
    call_id: &str,
    pending: &Arc<Mutex<HashSet<String>>>,
    tracked_access: bool,
) {
    if tracked_access {
        pending.lock().await.remove(call_id);
    }
}

async fn track_recording_candidate(
    call_id: &str,
    recording_state: &Arc<Mutex<RecordingCancelState>>,
) -> bool {
    recording_state
        .lock()
        .await
        .recording
        .insert(call_id.to_string())
}

async fn discard_recording_candidate(
    call_id: &str,
    recording_state: &Arc<Mutex<RecordingCancelState>>,
    tracked_recording: bool,
) {
    if !tracked_recording {
        return;
    }
    let mut state = recording_state.lock().await;
    state.recording.remove(call_id);
    state.canceled.remove(call_id);
}

async fn mark_recording_canceled(
    call_id: &str,
    recording_state: &Arc<Mutex<RecordingCancelState>>,
) -> bool {
    let mut state = recording_state.lock().await;
    if state.recording.contains(call_id) {
        state.canceled.insert(call_id.to_string());
        true
    } else {
        false
    }
}

async fn should_persist_recording(
    call_id: &str,
    recording_state: &Arc<Mutex<RecordingCancelState>>,
) -> bool {
    let mut state = recording_state.lock().await;
    state.recording.remove(call_id);
    !state.canceled.remove(call_id)
}

fn epoch_id() -> u64 {
    rand::rng().random()
}

fn base_response_with_to(
    req: &SipMessage,
    status_code: u16,
    reason: &str,
    to_value: &str,
) -> super::handler::SipResponseBuilder {
    let mut builder = super::handler::SipResponseBuilder::new(status_code, reason);
    for via in req.via_headers() {
        builder = builder.header("Via", via);
    }
    if let Some(from) = req.from_header() {
        builder = builder.header("From", from);
    }
    builder = builder.header("To", to_value);
    if let Some(call_id) = req.call_id() {
        builder = builder.header("Call-ID", call_id);
    }
    if let Some(cseq) = req.cseq() {
        builder = builder.header("CSeq", cseq);
    }
    builder.header("Server", "SIP3/0.1.0")
}

fn cleanup_temp_files(root: &Path) -> Result<usize> {
    if !root.exists() {
        return Ok(0);
    }

    let mut removed = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                warn!(
                    "Could not scan voicemail storage directory {:?}: {}",
                    dir, e
                );
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    warn!("Could not inspect voicemail storage entry: {}", e);
                    continue;
                }
            };
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(e) => {
                    warn!("Could not inspect voicemail storage path {:?}: {}", path, e);
                    continue;
                }
            };
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() && is_identifiable_temp_file(&path) {
                match std::fs::remove_file(&path) {
                    Ok(()) => removed += 1,
                    Err(e) => warn!("Could not remove temp voicemail file {:?}: {}", path, e),
                }
            }
        }
    }
    Ok(removed)
}

fn is_identifiable_temp_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            let lower = name.to_ascii_lowercase();
            lower.ends_with(".tmp") || lower.ends_with(".part") || lower.ends_with(".partial")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_access_extension() {
        assert!(is_voicemail_access_target("*97", "*97"));
        assert!(!is_voicemail_access_target("1001", "*97"));
    }

    #[test]
    fn access_target_requires_exact_match() {
        assert!(!is_voicemail_access_target("*970", "*97"));
        assert!(!is_voicemail_access_target(" *97", "*97"));
    }

    #[test]
    fn message_summary_event_matches_event_token_with_parameters() {
        assert!(is_message_summary_event(Some("message-summary")));
        assert!(is_message_summary_event(Some("message-summary;id=1001")));
        assert!(is_message_summary_event(Some(" MESSAGE-SUMMARY ; id=1001")));
    }

    #[test]
    fn message_summary_event_rejects_other_events_and_empty_headers() {
        assert!(!is_message_summary_event(None));
        assert!(!is_message_summary_event(Some("presence")));
        assert!(!is_message_summary_event(Some("dialog")));
        assert!(!is_message_summary_event(Some("message-summary-extra")));
    }

    #[tokio::test]
    async fn no_answer_timer_claim_records_fired_state_until_cleanup() {
        let mut timers = HashMap::from([(
            "call-123".to_string(),
            NoAnswerTimerEntry::Ringing(tokio::spawn(async {})),
        )]);

        assert!(claim_no_answer_timer_to_fire(&mut timers, "call-123"));
        assert!(matches!(
            timers.get("call-123"),
            Some(NoAnswerTimerEntry::Firing)
        ));
        assert!(!claim_no_answer_timer_to_fire(&mut timers, "call-123"));
    }

    #[tokio::test]
    async fn caller_cancel_after_no_answer_fire_blocks_delivery() {
        let mut timers = HashMap::from([(
            "call-123".to_string(),
            NoAnswerTimerEntry::Ringing(tokio::spawn(async {})),
        )]);

        assert!(claim_no_answer_timer_to_fire(&mut timers, "call-123"));
        assert!(no_answer_timer_should_continue(&timers, "call-123"));

        assert!(mark_fired_no_answer_timer_canceled(&mut timers, "call-123"));

        assert!(!no_answer_timer_should_continue(&timers, "call-123"));
    }

    #[test]
    fn no_answer_cancel_rewrites_invite_cseq_method_to_cancel() {
        let raw = "INVITE sip:1003@sip.air32.cn SIP/2.0\r\n\
                   Via: SIP/2.0/UDP 192.168.1.2:56473;branch=z9hG4bK.NMNgTadYq;rport\r\n\
                   Max-Forwards: 70\r\n\
                   From: sip:1001@sip.air32.cn;tag=fromtag\r\n\
                   To: sip:1003@sip.air32.cn\r\n\
                   Call-ID: NO4pEKSYw-\r\n\
                   CSeq: 20 INVITE\r\n\
                   Content-Length: 0\r\n\
                   \r\n";

        let msg = SipMessage::parse(raw).expect("parse invite");
        let cancel = build_forwarded_cancel_for_target(
            &msg,
            "sip:1003@192.168.1.2:43453;transport=udp",
            69,
            "sip.air32.cn",
        );

        assert!(cancel.contains("CSeq: 20 CANCEL\r\n"));
        assert!(!cancel.contains("CSeq: 20 INVITE\r\n"));
    }

    #[test]
    fn adds_to_tag_when_missing() {
        let out = with_to_tag("<sip:1001@sip.air32.cn>", "vm-abc");
        assert!(out.contains("tag=vm-abc"));
    }

    #[test]
    fn preserves_existing_to_tag() {
        let out = with_to_tag("<sip:1001@sip.air32.cn>;tag=existing", "vm-abc");
        assert_eq!(out, "<sip:1001@sip.air32.cn>;tag=existing");
    }

    #[test]
    fn adds_outer_to_tag_when_tag_exists_only_inside_uri() {
        let out = with_to_tag("<sip:1001@sip.air32.cn;tag=uri-param>", "vm-abc");
        assert_eq!(out, "<sip:1001@sip.air32.cn;tag=uri-param>;tag=vm-abc");
    }

    #[test]
    fn adds_outer_to_tag_when_tag_exists_only_inside_quoted_display_name() {
        let out = with_to_tag("\"Bob;tag=not-param\" <sip:1001@sip.air32.cn>", "vm-abc");
        assert_eq!(
            out,
            "\"Bob;tag=not-param\" <sip:1001@sip.air32.cn>;tag=vm-abc"
        );
    }

    #[test]
    fn quoted_display_name_tag_scanner_honors_backslash_escaped_quote() {
        let out = with_to_tag(
            "\"Bob \\\";tag=not-param\" <sip:1001@sip.air32.cn>",
            "vm-abc",
        );
        assert_eq!(
            out,
            "\"Bob \\\";tag=not-param\" <sip:1001@sip.air32.cn>;tag=vm-abc"
        );
    }

    #[tokio::test]
    async fn cancelled_recording_completion_is_consumed_and_not_persisted() {
        let state = Arc::new(Mutex::new(RecordingCancelState::default()));

        track_recording_candidate("call-123", &state).await;
        assert!(mark_recording_canceled("call-123", &state).await);

        assert!(!should_persist_recording("call-123", &state).await);
        assert!(should_persist_recording("call-123", &state).await);
    }

    #[tokio::test]
    async fn cancel_for_untracked_call_does_not_leave_cancellation_marker() {
        let state = Arc::new(Mutex::new(RecordingCancelState::default()));

        assert!(!mark_recording_canceled("call-123", &state).await);

        let guard = state.lock().await;
        assert!(guard.recording.is_empty());
        assert!(guard.canceled.is_empty());
    }

    #[tokio::test]
    async fn discarded_recording_candidate_removes_cancellation_marker() {
        let state = Arc::new(Mutex::new(RecordingCancelState::default()));

        assert!(track_recording_candidate("call-123", &state).await);
        assert!(mark_recording_canceled("call-123", &state).await);
        discard_recording_candidate("call-123", &state, true).await;

        let guard = state.lock().await;
        assert!(guard.recording.is_empty());
        assert!(guard.canceled.is_empty());
    }

    #[tokio::test]
    async fn duplicate_recording_candidate_cleanup_preserves_original_state() {
        let state = Arc::new(Mutex::new(RecordingCancelState::default()));

        assert!(track_recording_candidate("call-123", &state).await);
        assert!(!track_recording_candidate("call-123", &state).await);
        assert!(mark_recording_canceled("call-123", &state).await);
        discard_recording_candidate("call-123", &state, false).await;

        assert!(!should_persist_recording("call-123", &state).await);
    }

    #[test]
    fn duplicate_recording_candidate_gets_request_pending_response() {
        assert_eq!(
            duplicate_recording_candidate_response(false),
            Some((491, "Request Pending"))
        );
        assert_eq!(duplicate_recording_candidate_response(true), None);
    }

    #[test]
    fn duplicate_access_candidate_gets_request_pending_response() {
        assert_eq!(
            duplicate_access_candidate_response(false),
            Some((491, "Request Pending"))
        );
        assert_eq!(duplicate_access_candidate_response(true), None);
    }

    #[tokio::test]
    async fn duplicate_access_candidate_is_detected_before_active_insert() {
        let active = Arc::new(Mutex::new(HashMap::new()));
        let pending = Arc::new(Mutex::new(HashSet::new()));

        assert!(track_access_candidate("call-123", &active, &pending).await);
        assert!(!track_access_candidate("call-123", &active, &pending).await);
    }

    #[test]
    fn empty_recording_completion_is_not_persisted() {
        assert_eq!(
            recording_persistence_decision(true, 0),
            RecordingPersistenceDecision::DiscardEmpty
        );
        assert_eq!(
            recording_persistence_decision(true, 1),
            RecordingPersistenceDecision::Insert
        );
    }

    #[test]
    fn canceled_recording_completion_is_not_persisted() {
        assert_eq!(
            recording_persistence_decision(false, 1),
            RecordingPersistenceDecision::DiscardCanceled
        );
    }

    #[test]
    fn delivery_caller_rejects_missing_or_unparseable_from() {
        assert_eq!(delivery_caller(None), None);
        assert_eq!(delivery_caller(Some("not a sip address")), None);
        assert_eq!(delivery_caller(Some("<tel:1001>")), None);
    }

    #[test]
    fn delivery_caller_rejects_literal_unknown_sentinel() {
        assert_eq!(delivery_caller(Some("<sip:unknown@sip.air32.cn>")), None);
    }
}
