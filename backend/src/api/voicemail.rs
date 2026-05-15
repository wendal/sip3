use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::Response,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

use super::AppState;
use crate::models::{
    CreateVoicemailBox, UpdateVoicemailBox, UpdateVoicemailMessage, VoicemailBoxSummary,
    VoicemailMessage, validate_box_limits, validate_voicemail_status,
};
use crate::storage::voicemail::LocalVoicemailStorage;

#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    pub box_id: Option<u64>,
    pub username: Option<String>,
    pub status: Option<String>,
}

pub async fn list_boxes(
    State(state): State<AppState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let boxes: Vec<VoicemailBoxSummary> = sqlx::query_as(
        "SELECT
            b.id, b.username, b.domain, b.enabled,
            b.no_answer_secs, b.max_message_secs, b.max_messages,
            COALESCE(SUM(CASE WHEN m.status = 'new' THEN 1 ELSE 0 END), 0) AS new_count,
            COALESCE(SUM(CASE WHEN m.status = 'saved' THEN 1 ELSE 0 END), 0) AS saved_count
         FROM sip_voicemail_boxes b
         LEFT JOIN sip_voicemail_messages m ON b.id = m.box_id
         GROUP BY b.id, b.username, b.domain, b.enabled,
                  b.no_answer_secs, b.max_message_secs, b.max_messages
         ORDER BY b.username, b.domain",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "data": boxes })))
}

pub async fn create_box(
    State(state): State<AppState>,
    Json(body): Json<CreateVoicemailBox>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if body.username.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "username is required".to_string()));
    }

    let domain = body
        .domain
        .as_deref()
        .unwrap_or(&state.config.server.sip_domain)
        .to_string();
    let enabled = body.enabled.unwrap_or(1);
    let no_answer_secs = body
        .no_answer_secs
        .unwrap_or(state.config.server.voicemail_no_answer_secs);
    let max_message_secs = body
        .max_message_secs
        .unwrap_or(state.config.server.voicemail_max_message_secs);
    let max_messages = body.max_messages.unwrap_or(100);

    validate_box_limits(no_answer_secs, max_message_secs, max_messages)
        .map_err(|m| (StatusCode::BAD_REQUEST, m.to_string()))?;

    // Verify the account exists and is enabled
    let account_exists: Option<(i8,)> =
        sqlx::query_as("SELECT enabled FROM sip_accounts WHERE username = ? AND domain = ?")
            .bind(&body.username)
            .bind(&domain)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    match account_exists {
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                format!("Account {}@{} not found", body.username, domain),
            ));
        }
        Some((0,)) => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Account {}@{} is disabled", body.username, domain),
            ));
        }
        _ => {}
    }

    let result = sqlx::query(
        "INSERT INTO sip_voicemail_boxes
         (username, domain, enabled, no_answer_secs, max_message_secs, max_messages)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&body.username)
    .bind(&domain)
    .bind(enabled)
    .bind(no_answer_secs)
    .bind(max_message_secs)
    .bind(max_messages)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(db_err) = &e
            && db_err.is_unique_violation()
        {
            return (
                StatusCode::CONFLICT,
                "Voicemail box already exists for this username and domain".to_string(),
            );
        }
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    Ok(Json(
        json!({ "id": result.last_insert_id(), "message": "Voicemail box created" }),
    ))
}

pub async fn update_box(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(body): Json<UpdateVoicemailBox>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Validate limits if provided
    if body.no_answer_secs.is_some()
        || body.max_message_secs.is_some()
        || body.max_messages.is_some()
    {
        // Fetch current values to validate against
        let current: Option<(u32, u32, u32)> = sqlx::query_as(
            "SELECT no_answer_secs, max_message_secs, max_messages
             FROM sip_voicemail_boxes WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let (curr_no_answer, curr_max_msg, curr_max_msgs) =
            current.ok_or((StatusCode::NOT_FOUND, "Voicemail box not found".to_string()))?;

        let final_no_answer = body.no_answer_secs.unwrap_or(curr_no_answer);
        let final_max_msg = body.max_message_secs.unwrap_or(curr_max_msg);
        let final_max_msgs = body.max_messages.unwrap_or(curr_max_msgs);

        validate_box_limits(final_no_answer, final_max_msg, final_max_msgs)
            .map_err(|m| (StatusCode::BAD_REQUEST, m.to_string()))?;
    }

    let result = sqlx::query(
        "UPDATE sip_voicemail_boxes SET
             enabled           = COALESCE(?, enabled),
             no_answer_secs    = COALESCE(?, no_answer_secs),
             max_message_secs  = COALESCE(?, max_message_secs),
             max_messages      = COALESCE(?, max_messages),
             greeting_storage_key = COALESCE(?, greeting_storage_key)
         WHERE id = ?",
    )
    .bind(body.enabled)
    .bind(body.no_answer_secs)
    .bind(body.max_message_secs)
    .bind(body.max_messages)
    .bind(body.greeting_storage_key.as_deref())
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Voicemail box not found".to_string()));
    }

    Ok(Json(json!({ "message": "Voicemail box updated" })))
}

pub async fn list_messages(
    State(state): State<AppState>,
    Query(params): Query<ListMessagesQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Validate status if provided
    if let Some(ref status) = params.status {
        validate_voicemail_status(status).map_err(|m| (StatusCode::BAD_REQUEST, m.to_string()))?;
    }

    // Build query dynamically based on filters
    let mut query_str = String::from(
        "SELECT m.id, m.box_id, m.caller, m.callee, m.call_id,
                m.duration_secs, m.storage_key, m.content_type, m.status,
                m.created_at, m.heard_at
         FROM sip_voicemail_messages m",
    );

    let mut conditions = Vec::new();
    if params.box_id.is_some() {
        conditions.push("m.box_id = ?");
    }
    if params.username.is_some() {
        query_str.push_str(" JOIN sip_voicemail_boxes b ON m.box_id = b.id");
        conditions.push("b.username = ?");
    }
    if params.status.is_some() {
        conditions.push("m.status = ?");
    }

    if !conditions.is_empty() {
        query_str.push_str(" WHERE ");
        query_str.push_str(&conditions.join(" AND "));
    }

    query_str.push_str(" ORDER BY m.created_at DESC");

    let mut query = sqlx::query_as::<_, VoicemailMessage>(&query_str);

    if let Some(box_id) = params.box_id {
        query = query.bind(box_id);
    }
    if let Some(ref username) = params.username {
        query = query.bind(username);
    }
    if let Some(ref status) = params.status {
        query = query.bind(status);
    }

    let messages = query
        .fetch_all(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "data": messages })))
}

pub async fn download_message(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Response, (StatusCode, String)> {
    // Fetch storage_key from DB
    let message: Option<(String,)> =
        sqlx::query_as("SELECT storage_key FROM sip_voicemail_messages WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (storage_key,) = message.ok_or((StatusCode::NOT_FOUND, "Message not found".to_string()))?;

    // Read audio file from storage
    let storage = LocalVoicemailStorage::new(PathBuf::from(
        state.config.server.voicemail_storage_dir.clone(),
    ));
    let bytes = storage.read(&storage_key).await.map_err(|e| {
        if e.to_string().contains("No such file or directory")
            || e.to_string().contains("cannot find the path")
            || e.to_string().contains("系统找不到指定的路径")
        {
            return (StatusCode::NOT_FOUND, "Audio file not found".to_string());
        }
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    let filename = format!("voicemail-{}.wav", id);
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "audio/wav")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", filename),
        )
        .body(Body::from(bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(response)
}

pub async fn update_message(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(body): Json<UpdateVoicemailMessage>,
) -> Result<Json<Value>, (StatusCode, String)> {
    validate_voicemail_status(&body.status)
        .map_err(|m| (StatusCode::BAD_REQUEST, m.to_string()))?;

    // Set heard_at = NOW() when status becomes 'saved' or 'deleted'
    // Set heard_at = NULL for 'new' to maintain unread status
    let heard_at_expr = match body.status.as_str() {
        "saved" | "deleted" => "COALESCE(heard_at, NOW())",
        "new" => "NULL",
        _ => "heard_at", // should not reach due to validation
    };

    let query = format!(
        "UPDATE sip_voicemail_messages
         SET status = ?, heard_at = {}
         WHERE id = ?",
        heard_at_expr
    );

    let result = sqlx::query(&query)
        .bind(&body.status)
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Message not found".to_string()));
    }

    Ok(Json(json!({ "message": "Message updated" })))
}

pub async fn delete_message(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Soft delete: set status to 'deleted', preserve heard_at or set to NOW()
    let result = sqlx::query(
        "UPDATE sip_voicemail_messages
         SET status = 'deleted', heard_at = COALESCE(heard_at, NOW())
         WHERE id = ?",
    )
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Message not found".to_string()));
    }

    Ok(Json(json!({ "message": "Message deleted" })))
}
