use axum::{extract::Query, extract::State, http::StatusCode, Json};
use serde::Deserialize;
use serde_json::{json, Value};

use super::AppState;
use crate::models::{AutoBlockEntry, SecurityEvent, UnblockRequest};
use crate::security_guard::{persist_security_event, AuthSurface, SecurityEventType};

#[derive(Debug, Deserialize)]
pub struct SecurityEventsQuery {
    pub source_ip: Option<String>,
    pub event_type: Option<String>,
    pub surface: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

pub async fn list_events(
    State(state): State<AppState>,
    Query(q): Query<SecurityEventsQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let limit = q.limit.unwrap_or(100).min(500);
    let offset = q.offset.unwrap_or(0);

    let mut conditions: Vec<&str> = Vec::new();
    if q.source_ip.is_some() {
        conditions.push("source_ip = ?");
    }
    if q.event_type.is_some() {
        conditions.push("event_type = ?");
    }
    if q.surface.is_some() {
        conditions.push("surface = ?");
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT id, surface, event_type, source_ip, username, detail, created_at
         FROM sip_security_events
         {}
         ORDER BY created_at DESC
         LIMIT ? OFFSET ?",
        where_clause
    );

    let mut query = sqlx::query_as::<_, SecurityEvent>(&sql);
    if let Some(ip) = &q.source_ip {
        query = query.bind(ip);
    }
    if let Some(event_type) = &q.event_type {
        query = query.bind(event_type);
    }
    if let Some(surface) = &q.surface {
        query = query.bind(surface);
    }
    query = query.bind(limit).bind(offset);

    let data = query
        .fetch_all(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "data": data,
        "limit": limit,
        "offset": offset
    })))
}

pub async fn list_blocks(
    State(state): State<AppState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let rows: Vec<AutoBlockEntry> = sqlx::query_as(
        "SELECT id, cidr, description, priority, enabled, created_at
         FROM sip_acl
         WHERE action = 'deny' AND enabled = 1 AND description LIKE 'auto-ban:%'
         ORDER BY created_at DESC",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "data": rows })))
}

pub async fn unblock(
    State(state): State<AppState>,
    Json(body): Json<UnblockRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if body.cidr.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "cidr is required".to_string()));
    }

    let result = sqlx::query(
        "UPDATE sip_acl
         SET enabled = 0
         WHERE action = 'deny' AND cidr = ? AND enabled = 1 AND description LIKE 'auto-ban:%'",
    )
    .bind(body.cidr.trim())
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "block not found".to_string()));
    }

    let source_ip = body.cidr.split('/').next().unwrap_or("0.0.0.0").to_string();
    if let Err(e) = persist_security_event(
        &state.pool,
        AuthSurface::SipRegister,
        SecurityEventType::IpUnblocked,
        &source_ip,
        None,
        "manual unblock from security api",
    )
    .await
    {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
    }

    Ok(Json(json!({ "message": "block disabled" })))
}

pub async fn summary(State(state): State<AppState>) -> Result<Json<Value>, (StatusCode, String)> {
    let failed_24h: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sip_security_events
         WHERE event_type = 'auth_failed' AND created_at >= DATE_SUB(NOW(), INTERVAL 24 HOUR)",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let blocked_24h: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sip_security_events
         WHERE event_type = 'ip_blocked' AND created_at >= DATE_SUB(NOW(), INTERVAL 24 HOUR)",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let active_auto_blocks: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sip_acl
         WHERE action = 'deny' AND enabled = 1 AND description LIKE 'auto-ban:%'",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "auth_failed_24h": failed_24h,
        "blocked_24h": blocked_24h,
        "active_auto_blocks": active_auto_blocks
    })))
}

pub async fn runtime_snapshot(
    State(state): State<AppState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let active_registrations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sip_registrations WHERE expires_at > NOW()")
            .fetch_one(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let active_calls: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sip_calls WHERE status IN ('trying', 'answered') AND ended_at IS NULL",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let recent_failed_auth_5m: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sip_security_events
         WHERE event_type = 'auth_failed' AND created_at >= DATE_SUB(NOW(), INTERVAL 5 MINUTE)",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let active_auto_blocks: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sip_acl
         WHERE action = 'deny' AND enabled = 1 AND description LIKE 'auto-ban:%'",
    )
    .fetch_one(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "active_registrations": active_registrations,
        "active_calls": active_calls,
        "recent_failed_auth_5m": recent_failed_auth_5m,
        "active_auto_blocks": active_auto_blocks
    })))
}
