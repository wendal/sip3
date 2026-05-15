use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};

use super::AppState;
use crate::models::{
    ConferenceParticipant, ConferenceRoom, CreateConferenceRoom, UpdateConferenceRoom,
    validate_conference_extension,
};

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, (StatusCode, String)> {
    let rooms: Vec<ConferenceRoom> = sqlx::query_as(
        "SELECT id, extension, domain, name, enabled, max_participants, created_at, updated_at
         FROM sip_conference_rooms ORDER BY extension",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "data": rooms })))
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateConferenceRoom>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if body.name.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name is required".to_string()));
    }
    validate_conference_extension(&body.extension)
        .map_err(|m| (StatusCode::BAD_REQUEST, m.to_string()))?;

    let realm = &state.config.auth.realm;
    let domain = body.domain.as_deref().unwrap_or(realm.as_str()).to_string();
    let max_participants = body.max_participants.unwrap_or(20);
    let enabled = body.enabled.unwrap_or(1);

    if max_participants == 0 || max_participants > 200 {
        return Err((
            StatusCode::BAD_REQUEST,
            "max_participants must be between 1 and 200".to_string(),
        ));
    }

    let result = sqlx::query(
        "INSERT INTO sip_conference_rooms (extension, domain, name, enabled, max_participants)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&body.extension)
    .bind(&domain)
    .bind(body.name.trim())
    .bind(enabled)
    .bind(max_participants)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(db_err) = &e
            && db_err.is_unique_violation()
        {
            return (
                StatusCode::CONFLICT,
                "Conference room already exists for this extension and domain".to_string(),
            );
        }
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    Ok(Json(
        json!({ "id": result.last_insert_id(), "message": "Conference room created" }),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(body): Json<UpdateConferenceRoom>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if let Some(mp) = body.max_participants
        && (mp == 0 || mp > 200)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "max_participants must be between 1 and 200".to_string(),
        ));
    }

    let result = sqlx::query(
        "UPDATE sip_conference_rooms SET
             name             = COALESCE(?, name),
             domain           = COALESCE(?, domain),
             max_participants = COALESCE(?, max_participants),
             enabled          = COALESCE(?, enabled)
         WHERE id = ?",
    )
    .bind(body.name.as_deref().map(str::trim))
    .bind(body.domain.as_deref())
    .bind(body.max_participants)
    .bind(body.enabled)
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            "Conference room not found".to_string(),
        ));
    }

    Ok(Json(json!({ "message": "Conference room updated" })))
}

pub async fn delete_room(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let result = sqlx::query("DELETE FROM sip_conference_rooms WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            "Conference room not found".to_string(),
        ));
    }

    Ok(Json(json!({ "message": "Conference room deleted" })))
}

pub async fn list_participants(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let participants: Vec<ConferenceParticipant> = sqlx::query_as(
        "SELECT id, room_id, call_id, account, source_ip, source_port,
                rtp_ip, rtp_port, relay_port, codec, muted, joined_at, left_at
         FROM sip_conference_participants
         WHERE room_id = ? AND left_at IS NULL
         ORDER BY joined_at",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "data": participants })))
}
