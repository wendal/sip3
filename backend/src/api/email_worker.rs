//! Background worker that drains the `sip_email_outbox` table and sends
//! each pending entry via SMTP using `lettre`. Triggered by the voicemail
//! storage path on a successful message save (when the mailbox has a
//! configured email address).
//!
//! Configuration lives in `Config::email` (already wired into Config
//! but unused before C5). When `email.smtp_host` is empty the worker
//! is a no-op; rows accumulate in the outbox for an operator to
//! replay once SMTP is configured.

use anyhow::Result;
use lettre::AsyncTransport;
use lettre::message::{Attachment, Body, Mailbox, Message, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, Tokio1Executor};
use sqlx::MySqlPool;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::config::Config;
use crate::storage::voicemail::LocalVoicemailStorage;
use std::path::PathBuf;

const POLL_INTERVAL: Duration = Duration::from_secs(15);
const MAX_ATTEMPTS: u32 = 5;

pub struct EmailWorker {
    pool: MySqlPool,
    config: Arc<arc_swap::ArcSwap<Config>>,
}

impl EmailWorker {
    pub fn new(pool: MySqlPool, config: Arc<arc_swap::ArcSwap<Config>>) -> Self {
        Self { pool, config }
    }

    /// Spawn a tokio task that drains the outbox forever.
    pub fn spawn_worker(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(POLL_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                if let Err(e) = self.drain_once().await {
                    warn!("email worker drain error: {}", e);
                }
            }
        });
    }

    pub async fn drain_once(&self) -> Result<usize> {
        // Refresh storage dir in case config was reloaded.
        let storage_dir = PathBuf::from(self.config.load().server.voicemail_storage_dir.clone());
        let email_cfg = &self.config.load().email;
        if email_cfg.smtp_host.is_empty() {
            // Operator hasn't configured SMTP yet; skip silently.
            return Ok(0);
        }

        let rows: Vec<(u64, String, String, String, Option<String>, u32)> = sqlx::query_as(
            "SELECT id, to_addr, subject, body, attachment_key, attempts
             FROM sip_email_outbox
             WHERE status = 'pending'
                OR (status = 'failed' AND scheduled_at <= NOW())
             ORDER BY id ASC
             LIMIT 25",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut sent = 0usize;
        let storage = LocalVoicemailStorage::new(storage_dir);
        for (id, to_addr, subject, body, attachment_key, attempts) in rows {
            let result = build_and_send(
                email_cfg,
                &to_addr,
                &subject,
                &body,
                attachment_key.as_deref(),
                &storage,
            )
            .await;

            match result {
                Ok(()) => {
                    sqlx::query(
                        "UPDATE sip_email_outbox
                         SET status = 'sent', sent_at = NOW(), last_error = NULL
                         WHERE id = ?",
                    )
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
                    crate::api::metrics::inc_email_outbox_sent();
                    info!(target: "email", "outbox id={} sent to {}", id, to_addr);
                    sent += 1;
                }
                Err(e) => {
                    let err = e.to_string();
                    let next_attempt = attempts + 1;
                    if next_attempt >= MAX_ATTEMPTS {
                        sqlx::query(
                            "UPDATE sip_email_outbox
                             SET status = 'dead', attempts = ?, last_error = ?
                             WHERE id = ?",
                        )
                        .bind(next_attempt)
                        .bind(&err)
                        .bind(id)
                        .execute(&self.pool)
                        .await?;
                    } else {
                        // 5 minute backoff between retries.
                        sqlx::query(
                            "UPDATE sip_email_outbox
                             SET status = 'failed', attempts = ?, last_error = ?,
                                 scheduled_at = DATE_ADD(NOW(), INTERVAL 5 MINUTE)
                             WHERE id = ?",
                        )
                        .bind(next_attempt)
                        .bind(&err)
                        .bind(id)
                        .execute(&self.pool)
                        .await?;
                    }
                    warn!(target: "email", "outbox id={} send failed: {}", id, err);
                }
            }
        }
        let pending: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sip_email_outbox WHERE status IN ('pending','failed')",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap_or((0,));
        crate::api::metrics::set_email_outbox_pending(pending.0);
        Ok(sent)
    }
}

async fn build_and_send(
    email_cfg: &crate::config::EmailConfig,
    to_addr: &str,
    subject: &str,
    body: &str,
    attachment_key: Option<&str>,
    storage: &LocalVoicemailStorage,
) -> Result<()> {
    let from: Mailbox = email_cfg
        .from_address
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid from_address: {e}"))?;
    let to: Mailbox = to_addr
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid to_addr: {e}"))?;

    let message_builder = Message::builder()
        .from(from)
        .to(to)
        .subject(subject.to_string());

    let text_part = SinglePart::plain(body.to_string());
    let message = if let Some(key) = attachment_key {
        let bytes = storage
            .read(key)
            .await
            .map_err(|e| anyhow::anyhow!("read attachment: {e}"))?;
        let filename = key.rsplit('/').next().unwrap_or("voicemail.wav");
        let attachment = Attachment::new(filename.to_string())
            .body(Body::new(bytes), "audio/wav".parse().unwrap());
        message_builder.multipart(
            MultiPart::mixed()
                .singlepart(text_part)
                .singlepart(attachment),
        )?
    } else {
        message_builder.singlepart(text_part)?
    };

    let creds = Credentials::new(
        email_cfg.smtp_username.clone(),
        email_cfg.smtp_password.clone(),
    );
    let transport = if email_cfg.use_tls {
        AsyncSmtpTransport::<Tokio1Executor>::relay(&email_cfg.smtp_host)?
            .port(email_cfg.smtp_port)
            .credentials(creds)
            .build()
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&email_cfg.smtp_host)
            .port(email_cfg.smtp_port)
            .credentials(creds)
            .build()
    };

    transport.send(message).await?;
    Ok(())
}

/// Insert a row into sip_email_outbox. Called by the voicemail storage
/// path when a new message is recorded for a box that has a configured
/// email.
pub async fn enqueue_new_message(
    pool: &MySqlPool,
    mailbox_id: u64,
    to_addr: &str,
    caller: &str,
    duration_secs: u32,
    attachment_key: &str,
) -> Result<()> {
    let subject = format!("New voicemail from {}", caller);
    let body = format!(
        "You have a new voicemail.\n\nCaller: {}\nDuration: {} seconds\n\nListen via the admin UI or SIP *97 access.",
        caller, duration_secs
    );
    sqlx::query(
        "INSERT INTO sip_email_outbox
           (mailbox_id, to_addr, subject, body, attachment_key, status)
         VALUES (?, ?, ?, ?, ?, 'pending')",
    )
    .bind(mailbox_id)
    .bind(to_addr)
    .bind(&subject)
    .bind(&body)
    .bind(attachment_key)
    .execute(pool)
    .await?;
    Ok(())
}
