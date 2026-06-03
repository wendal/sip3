//! Webhook subscription admin CRUD + delivery history.
//!
//! Routes (all require JWT or X-Api-Key, mounted in `api/mod.rs`):
//! - `GET    /api/webhooks`              — list subscriptions
//! - `POST   /api/webhooks`              — create subscription
//! - `PUT    /api/webhooks/:id`          — update subscription
//! - `DELETE /api/webhooks/:id`          — delete subscription (cascades deliveries)
//! - `GET    /api/webhooks/deliveries`   — recent delivery history
//! - `POST   /api/webhooks/deliveries/:id/retry` — force-retry a failed/dead delivery

use axum::{Json, extract::State, http::StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::MySqlPool;

use super::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateWebhook {
    pub name: String,
    pub url: String,
    pub secret: String,
    pub events: Vec<String>,
    #[serde(default = "default_active")]
    pub active: i8,
}

fn default_active() -> i8 {
    1
}

#[derive(Debug, Deserialize)]
pub struct UpdateWebhook {
    pub name: Option<String>,
    pub url: Option<String>,
    pub secret: Option<String>,
    pub events: Option<Vec<String>>,
    pub active: Option<i8>,
}

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, (StatusCode, String)> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        u64,
        String,
        String,
        String,
        i8,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT id, name, url, events_json, active, created_at, updated_at
         FROM sip_webhooks
         ORDER BY id ASC",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let webhooks: Vec<Value> = rows
        .into_iter()
        .map(
            |(id, name, url, events_json, active, created_at, updated_at)| {
                let events: Vec<String> = serde_json::from_str(&events_json).unwrap_or_default();
                json!({
                    "id": id,
                    "name": name,
                    "url": url,
                    "events": events,
                    "active": active != 0,
                    "created_at": created_at,
                    "updated_at": updated_at,
                })
            },
        )
        .collect();

    Ok(Json(json!({ "data": webhooks })))
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateWebhook>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if body.name.trim().is_empty() || body.url.trim().is_empty() || body.secret.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "name, url, and secret are required".to_string(),
        ));
    }
    if body.events.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "events must contain at least one event type".to_string(),
        ));
    }
    let events_json = serde_json::to_string(&body.events).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize events: {e}"),
        )
    })?;

    let result = sqlx::query(
        "INSERT INTO sip_webhooks (name, url, secret, events_json, active)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&body.name)
    .bind(&body.url)
    .bind(&body.secret)
    .bind(&events_json)
    .bind(body.active)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(db_err) = &e
            && db_err.is_unique_violation()
        {
            return (
                StatusCode::CONFLICT,
                format!("webhook name '{}' already exists", body.name),
            );
        }
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    Ok(Json(json!({
        "id": result.last_insert_id(),
        "name": body.name,
        "events": body.events,
        "active": body.active != 0,
    })))
}

pub async fn update(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<u64>,
    Json(body): Json<UpdateWebhook>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("begin tx: {e}")))?;

    if let Some(name) = &body.name {
        sqlx::query("UPDATE sip_webhooks SET name = ? WHERE id = ?")
            .bind(name)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    if let Some(url) = &body.url {
        sqlx::query("UPDATE sip_webhooks SET url = ? WHERE id = ?")
            .bind(url)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    if let Some(secret) = &body.secret {
        sqlx::query("UPDATE sip_webhooks SET secret = ? WHERE id = ?")
            .bind(secret)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    if let Some(events) = &body.events {
        let events_json = serde_json::to_string(events).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("serialize events: {e}"),
            )
        })?;
        sqlx::query("UPDATE sip_webhooks SET events_json = ? WHERE id = ?")
            .bind(&events_json)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    if let Some(active) = body.active {
        sqlx::query("UPDATE sip_webhooks SET active = ? WHERE id = ?")
            .bind(active)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    tx.commit()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("commit tx: {e}")))?;

    Ok(Json(json!({ "id": id, "message": "Webhook updated" })))
}

pub async fn delete(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<u64>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let result = sqlx::query("DELETE FROM sip_webhooks WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Webhook not found".to_string()));
    }
    Ok(Json(json!({ "id": id, "message": "Webhook deleted" })))
}

#[derive(Debug, Deserialize)]
pub struct DeliveryQuery {
    #[serde(default)]
    pub webhook_id: Option<u64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_limit() -> u32 {
    100
}

pub async fn list_deliveries(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<DeliveryQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let limit = q.limit.min(500);
    let offset = q.offset;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        u64,
        u64,
        String,
        String,
        String,
        u32,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
        chrono::DateTime<chrono::Utc>,
        Option<chrono::DateTime<chrono::Utc>>,
    )> = sqlx::query_as(
        "SELECT id, webhook_id, event_type, payload_json, status, attempts,
                last_error, next_retry_at, created_at, delivered_at
         FROM sip_webhook_deliveries
         WHERE (? IS NULL OR webhook_id = ?)
           AND (? IS NULL OR status = ?)
         ORDER BY id DESC
         LIMIT ? OFFSET ?",
    )
    .bind(q.webhook_id)
    .bind(q.webhook_id)
    .bind(&q.status)
    .bind(&q.status)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let deliveries: Vec<Value> = rows
        .into_iter()
        .map(
            |(
                id,
                webhook_id,
                event_type,
                payload_json,
                status,
                attempts,
                last_error,
                next_retry_at,
                created_at,
                delivered_at,
            )| {
                let payload: Value = serde_json::from_str(&payload_json).unwrap_or(Value::Null);
                json!({
                    "id": id,
                    "webhook_id": webhook_id,
                    "event_type": event_type,
                    "payload": payload,
                    "status": status,
                    "attempts": attempts,
                    "last_error": last_error,
                    "next_retry_at": next_retry_at,
                    "created_at": created_at,
                    "delivered_at": delivered_at,
                })
            },
        )
        .collect();

    Ok(Json(json!({
        "data": deliveries,
        "limit": limit,
        "offset": offset,
    })))
}

pub async fn retry_delivery(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<u64>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let result = sqlx::query(
        "UPDATE sip_webhook_deliveries
         SET status = 'pending', next_retry_at = NULL
         WHERE id = ? AND status IN ('failed', 'dead')",
    )
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            "Delivery not found or not in a retryable state".to_string(),
        ));
    }
    Ok(Json(
        json!({ "id": id, "message": "Delivery queued for retry" }),
    ))
}

/// Helper used by other modules: enqueue a delivery if any active webhook
/// is subscribed to the given event type.
pub async fn enqueue(pool: &MySqlPool, event_type: &str, payload: Value) -> anyhow::Result<()> {
    let dispatcher = super::webhook_dispatcher::WebhookDispatcher::new(pool.clone());
    dispatcher.enqueue(event_type, payload).await
}
