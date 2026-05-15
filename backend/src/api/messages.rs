use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;
use crate::models::SipMessageRecord;

#[derive(Debug, Deserialize)]
pub struct MessageHistoryRequest {
    pub username: String,
    pub password: String,
    pub domain: Option<String>,
    pub peer: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct MessagesQuery {
    pub sender: Option<String>,
    pub receiver: Option<String>,
    pub status: Option<String>,
    pub since: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

async fn verify_sip_credentials(
    state: &AppState,
    username: &str,
    password: &str,
    domain: &str,
) -> Result<(), (StatusCode, String)> {
    let realm = &state.config.auth.realm;
    let ha1_computed = format!(
        "{:x}",
        md5::compute(format!("{}:{}:{}", username, realm, password))
    );

    let stored: Option<String> = sqlx::query_scalar(
        "SELECT ha1_hash FROM sip_accounts WHERE username = ? AND domain = ? AND enabled = 1",
    )
    .bind(username)
    .bind(domain)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let stored_ha1 =
        stored.ok_or_else(|| (StatusCode::UNAUTHORIZED, "Invalid credentials".to_string()))?;
    if stored_ha1 != ha1_computed {
        return Err((StatusCode::UNAUTHORIZED, "Invalid credentials".to_string()));
    }
    Ok(())
}

pub async fn history(
    State(state): State<AppState>,
    Json(body): Json<MessageHistoryRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if body.username.is_empty() || body.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "username and password are required".to_string(),
        ));
    }

    let limit = body.limit.unwrap_or(50).min(200);
    let offset = body.offset.unwrap_or(0);
    let account_domain = body
        .domain
        .unwrap_or_else(|| state.config.server.sip_domain.clone());
    verify_sip_credentials(&state, &body.username, &body.password, &account_domain).await?;

    let message_domain = state.config.server.sip_domain.clone();
    let self_aor = format!("{}@{}", body.username, message_domain);

    let rows: Vec<SipMessageRecord> = if let Some(peer) = body.peer.as_deref().map(str::trim) {
        if peer.is_empty() {
            return Err((StatusCode::BAD_REQUEST, "peer cannot be empty".to_string()));
        }
        let peer_aor = format!("{}@{}", peer, state.config.server.sip_domain);
        sqlx::query_as(
            "SELECT id, message_id, call_id, sender, receiver, content_type, body, status, source_ip, created_at, delivered_at
             FROM sip_messages
             WHERE (sender = ? AND receiver = ?)
                OR (sender = ? AND receiver = ?)
             ORDER BY created_at DESC
             LIMIT ? OFFSET ?",
        )
        .bind(&self_aor)
        .bind(&peer_aor)
        .bind(&peer_aor)
        .bind(&self_aor)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        sqlx::query_as(
            "SELECT id, message_id, call_id, sender, receiver, content_type, body, status, source_ip, created_at, delivered_at
             FROM sip_messages
             WHERE sender = ? OR receiver = ?
             ORDER BY created_at DESC
             LIMIT ? OFFSET ?",
        )
        .bind(&self_aor)
        .bind(&self_aor)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    Ok(Json(json!({
        "data": rows,
        "limit": limit,
        "offset": offset,
    })))
}

pub async fn list_messages(
    State(state): State<AppState>,
    Query(q): Query<MessagesQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let limit = q.limit.unwrap_or(100).min(500);
    let offset = q.offset.unwrap_or(0);

    let mut conditions: Vec<&str> = Vec::new();
    if q.sender.is_some() {
        conditions.push("sender LIKE ?");
    }
    if q.receiver.is_some() {
        conditions.push("receiver LIKE ?");
    }
    if q.status.is_some() {
        conditions.push("status = ?");
    }
    if q.since.is_some() {
        conditions.push("created_at >= ?");
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT id, message_id, call_id, sender, receiver, content_type, body, status, source_ip, created_at, delivered_at
         FROM sip_messages
         {}
         ORDER BY created_at DESC
         LIMIT ? OFFSET ?",
        where_clause
    );

    let mut query = sqlx::query_as::<_, SipMessageRecord>(&sql);
    if let Some(ref s) = q.sender {
        query = query.bind(format!("%{}%", s));
    }
    if let Some(ref s) = q.receiver {
        query = query.bind(format!("%{}%", s));
    }
    if let Some(ref s) = q.status {
        query = query.bind(s);
    }
    if let Some(ref s) = q.since {
        query = query.bind(s);
    }
    query = query.bind(limit).bind(offset);

    let rows = query
        .fetch_all(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "data": rows,
        "limit": limit,
        "offset": offset,
    })))
}
