//! Webhook outbox + delivery worker.
//!
//! Business code calls [`WebhookDispatcher::enqueue`] which inserts a
//! pending row into `sip_webhook_deliveries`. A background tokio task
//! started by [`WebhookDispatcher::spawn_worker`] drains pending rows
//! every 10 seconds, POSTs them to the matching active webhook's URL
//! with an HMAC-SHA256 signature in `X-Sip3-Signature`, and updates
//! the row's status. Failed deliveries retry with exponential backoff
//! (2^attempts minutes) up to 10 attempts, then move to status=`dead`.

use anyhow::Result;
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde_json::Value;
use sha2::Sha256;
use sqlx::MySqlPool;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

const MAX_ATTEMPTS: u32 = 10;
const POLL_INTERVAL: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct WebhookDispatcher {
    pool: MySqlPool,
    http: Client,
}

impl WebhookDispatcher {
    pub fn new(pool: MySqlPool) -> Self {
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent("sip3-webhook/1.8")
            .build()
            .expect("reqwest client");
        Self { pool, http }
    }

    /// Enqueue an event for delivery to all active webhooks that
    /// subscribed to the given event type. The actual fan-out is done
    /// by the worker; this call only inserts one row per matching
    /// webhook.
    pub async fn enqueue(&self, event_type: &str, payload: Value) -> Result<()> {
        let payload_str = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());

        let webhooks: Vec<(u64, String, String, String)> = sqlx::query_as(
            "SELECT id, url, secret, events_json FROM sip_webhooks
             WHERE active = 1",
        )
        .fetch_all(&self.pool)
        .await?;

        for (id, _url, _secret, events_json) in webhooks {
            let subscribed: Vec<String> = serde_json::from_str(&events_json).unwrap_or_default();
            if !subscribed.iter().any(|e| e == event_type) {
                continue;
            }
            sqlx::query(
                "INSERT INTO sip_webhook_deliveries
                 (webhook_id, event_type, payload_json, status, attempts)
                 VALUES (?, ?, ?, 'pending', 0)",
            )
            .bind(id)
            .bind(event_type)
            .bind(&payload_str)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// Spawn a background tokio task that drains the outbox forever.
    pub fn spawn_worker(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(POLL_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await; // skip immediate
            loop {
                ticker.tick().await;
                if let Err(e) = self.drain_once().await {
                    warn!("webhook drain error: {}", e);
                }
            }
        });
    }

    /// Drain a batch of pending/failed deliveries whose next_retry_at
    /// has passed. Returns the number of rows attempted.
    pub async fn drain_once(&self) -> Result<usize> {
        let rows: Vec<(u64, String, String, String, u32)> = sqlx::query_as(
            "SELECT d.id, d.event_type, d.payload_json, w.url || '|' || w.secret, d.attempts
             FROM sip_webhook_deliveries d
             JOIN sip_webhooks w ON w.id = d.webhook_id
             WHERE d.status = 'pending'
                OR (d.status = 'failed' AND (d.next_retry_at IS NULL OR d.next_retry_at <= NOW()))
             ORDER BY d.id ASC
             LIMIT 50",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut attempted = 0usize;
        for (id, event_type, payload_str, url_secret, attempts) in rows {
            attempted += 1;
            let (url, secret) = match url_secret.split_once('|') {
                Some(pair) => pair,
                None => {
                    self.mark_failed(id, "invalid url|secret row").await?;
                    continue;
                }
            };
            let signature = sign(secret, &payload_str);
            let result = self
                .http
                .post(url)
                .header("Content-Type", "application/json")
                .header("X-Sip3-Event", &event_type)
                .header("X-Sip3-Signature", &signature)
                .header("X-Sip3-Delivery-Id", id.to_string())
                .body(payload_str.clone())
                .send()
                .await;

            match result {
                Ok(resp) if resp.status().is_success() => {
                    sqlx::query(
                        "UPDATE sip_webhook_deliveries
                         SET status = 'delivered', delivered_at = NOW(), last_error = NULL
                         WHERE id = ?",
                    )
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
                    crate::api::metrics::inc_webhook_delivery("delivered");
                    info!(target: "webhook", "delivered id={} event={}", id, event_type);
                }
                Ok(resp) => {
                    let status = resp.status();
                    let err = format!("http {}", status.as_u16());
                    self.maybe_retry(id, attempts, &err).await?;
                }
                Err(e) => {
                    let err = e.to_string();
                    self.maybe_retry(id, attempts, &err).await?;
                }
            }
        }
        Ok(attempted)
    }

    async fn maybe_retry(&self, id: u64, attempts: u32, err: &str) -> Result<()> {
        let next_attempt = attempts + 1;
        if next_attempt >= MAX_ATTEMPTS {
            sqlx::query(
                "UPDATE sip_webhook_deliveries
                 SET status = 'dead', attempts = ?, last_error = ?
                 WHERE id = ?",
            )
            .bind(next_attempt)
            .bind(err)
            .bind(id)
            .execute(&self.pool)
            .await?;
            crate::api::metrics::inc_webhook_delivery("dead");
            warn!(target: "webhook", "delivery {} died: {}", id, err);
        } else {
            // backoff: 2^attempts minutes, capped at 60 minutes
            let backoff_secs = (60u64).min(60 * (1u64 << next_attempt.min(6)));
            sqlx::query(
                "UPDATE sip_webhook_deliveries
                 SET status = 'failed', attempts = ?, last_error = ?,
                     next_retry_at = DATE_ADD(NOW(), INTERVAL ? SECOND)
                 WHERE id = ?",
            )
            .bind(next_attempt)
            .bind(err)
            .bind(backoff_secs as i64)
            .bind(id)
            .execute(&self.pool)
            .await?;
            crate::api::metrics::inc_webhook_delivery("failed");
            warn!(target: "webhook", "delivery {} failed (attempt {}): {}", id, next_attempt, err);
        }
        Ok(())
    }

    async fn mark_failed(&self, id: u64, err: &str) -> Result<()> {
        sqlx::query(
            "UPDATE sip_webhook_deliveries
             SET status = 'failed', last_error = ?
             WHERE id = ?",
        )
        .bind(err)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn sign(secret: &str, body: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(body.as_bytes());
    let bytes = mac.finalize().into_bytes();
    hex::encode(bytes)
}

/// Helper: compute the signature for a (secret, body) pair. Exposed for
/// integration tests and debugging (`webhook sign`).
pub fn sign_for_test(secret: &str, body: &str) -> String {
    sign(secret, body)
}
