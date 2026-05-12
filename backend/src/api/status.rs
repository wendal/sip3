use axum::{extract::State, http::StatusCode, Json};
use serde_json::{json, Value};

use super::AppState;
use crate::models::{Call, Registration};

pub async fn list_registrations(
    State(state): State<AppState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let registrations: Vec<Registration> = sqlx::query_as(
        "SELECT id, username, domain, contact_uri, user_agent, expires_at, registered_at,
                source_ip, source_port
         FROM sip_registrations WHERE expires_at > NOW() ORDER BY registered_at DESC",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "data": registrations })))
}

pub async fn list_calls(
    State(state): State<AppState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let calls: Vec<Call> = sqlx::query_as(
        "SELECT id, call_id, caller, callee, status, started_at, answered_at, ended_at
         FROM sip_calls ORDER BY started_at DESC LIMIT 100",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "data": calls })))
}
