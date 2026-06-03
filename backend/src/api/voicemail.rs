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
    VoicemailMessage, validate_box_limits, validate_enabled_flag, validate_voicemail_status,
};
use crate::sip::voicemail_mwi::VoicemailMwi;
use crate::storage::voicemail::LocalVoicemailStorage;
use tracing::{debug, warn};

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
            b.email,
            (CASE WHEN b.greeting_storage_key IS NOT NULL THEN 1 ELSE 0 END) AS has_greeting,
            COALESCE(SUM(CASE WHEN m.status = 'new' THEN 1 ELSE 0 END), 0) AS new_count,
            COALESCE(SUM(CASE WHEN m.status = 'saved' THEN 1 ELSE 0 END), 0) AS saved_count
         FROM sip_voicemail_boxes b
         LEFT JOIN sip_voicemail_messages m ON b.id = m.box_id
         GROUP BY b.id, b.username, b.domain, b.enabled,
                  b.no_answer_secs, b.max_message_secs, b.max_messages,
                  b.email, b.greeting_storage_key
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
        .unwrap_or(&state.config.load().server.sip_domain)
        .to_string();
    let enabled = body.enabled.unwrap_or(1);
    let no_answer_secs = body
        .no_answer_secs
        .unwrap_or(state.config.load().server.voicemail_no_answer_secs);
    let max_message_secs = body
        .max_message_secs
        .unwrap_or(state.config.load().server.voicemail_max_message_secs);
    let max_messages = body.max_messages.unwrap_or(100);

    validate_enabled_flag(body.enabled).map_err(|m| (StatusCode::BAD_REQUEST, m.to_string()))?;
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
    validate_enabled_flag(body.enabled).map_err(|m| (StatusCode::BAD_REQUEST, m.to_string()))?;
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
             greeting_storage_key = COALESCE(?, greeting_storage_key),
             email             = COALESCE(?, email)
         WHERE id = ?",
    )
    .bind(body.enabled)
    .bind(body.no_answer_secs)
    .bind(body.max_message_secs)
    .bind(body.max_messages)
    .bind(body.greeting_storage_key.as_deref())
    .bind(body.email.as_deref())
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
        state.config.load().server.voicemail_storage_dir.clone(),
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
    notify_message_owner(&state, id).await;

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
    notify_message_owner(&state, id).await;

    Ok(Json(json!({ "message": "Message deleted" })))
}

async fn notify_message_owner(state: &AppState, message_id: u64) {
    let row: Result<Option<(String, String)>, sqlx::Error> = sqlx::query_as(
        "SELECT b.username, b.domain
         FROM sip_voicemail_messages m
         JOIN sip_voicemail_boxes b ON m.box_id = b.id
         WHERE m.id = ?",
    )
    .bind(message_id)
    .fetch_optional(&state.pool)
    .await;

    let Some((username, domain)) = (match row {
        Ok(row) => row,
        Err(e) => {
            warn!(
                "Failed to look up voicemail message owner for MWI notification: {}",
                e
            );
            return;
        }
    }) else {
        return;
    };

    match VoicemailMwi::notify_mailbox_if_available(&username, &domain).await {
        Ok(true) => debug!("Sent voicemail MWI update for {}@{}", username, domain),
        Ok(false) => debug!(
            "Skipped voicemail MWI update for {}@{} because notifier is not initialized",
            username, domain
        ),
        Err(e) => warn!(
            "Failed to send voicemail MWI update for {}@{}: {}",
            username, domain, e
        ),
    }
}

// ------------------------------------------------------------------
// Greeting upload / download / delete
// ------------------------------------------------------------------

use axum::extract::Multipart;

/// `POST /api/voicemail/boxes/:id/greeting` — multipart upload, field `file`.
/// Accepts a 8kHz mono PCM16 WAV up to 60s. Stores it on disk via
/// `LocalVoicemailStorage` and updates `sip_voicemail_boxes.greeting_storage_key`.
pub async fn upload_greeting(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    mut multipart: Multipart,
) -> Result<Json<Value>, (StatusCode, String)> {
    // 1. Find the box and get its (username, domain) for storage keying.
    let box_row: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT username, domain, greeting_storage_key FROM sip_voicemail_boxes WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let (username, domain, prev_key) =
        box_row.ok_or_else(|| (StatusCode::NOT_FOUND, "Voicemail box not found".to_string()))?;

    // 2. Read the file field.
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut original_filename: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("multipart: {e}")))?
    {
        if field.name() == Some("file") {
            original_filename = field.file_name().map(|s| s.to_string());
            file_bytes = Some(
                field
                    .bytes()
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, format!("read field: {e}")))?
                    .to_vec(),
            );
        }
    }
    let bytes =
        file_bytes.ok_or_else(|| (StatusCode::BAD_REQUEST, "missing 'file' field".to_string()))?;

    // 3. Validate: must be a decodable mono PCM-16 WAV with duration <= 60s.
    let decoded = crate::storage::voicemail::read_pcm16_wav(&bytes).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("not a valid PCM16 WAV: {e}"),
        )
    })?;
    let sample_rate = decoded.sample_rate as u32;
    if sample_rate != 8000 {
        return Err((
            StatusCode::BAD_REQUEST,
            "greeting must be 8kHz mono PCM16 WAV".to_string(),
        ));
    }
    let duration_secs = decoded.samples.len() as u32 / sample_rate;
    if duration_secs == 0 || duration_secs > 60 {
        return Err((
            StatusCode::BAD_REQUEST,
            "greeting duration must be between 1 and 60 seconds".to_string(),
        ));
    }

    // 4. Persist on disk. Reuse LocalVoicemailStorage::write_message by
    //    passing a synthetic call_id of "greeting" so the file lands at
    //    `<root>/<sanitized-username>/greeting-<uuid>.wav`.
    let storage_dir = state.config.load().server.voicemail_storage_dir.clone();
    let storage = LocalVoicemailStorage::new(PathBuf::from(storage_dir));
    let storage_key = storage
        .write_message(&username, "greeting", &bytes)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("write: {e}")))?;

    // 5. Remove the previous file (best-effort).
    if let Some(prev) = prev_key.as_deref() {
        let _ = storage.delete(prev).await;
    }

    // 6. Update DB row + insert history entry.
    sqlx::query("UPDATE sip_voicemail_boxes SET greeting_storage_key = ? WHERE id = ?")
        .bind(&storage_key)
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    sqlx::query(
        "INSERT INTO sip_voicemail_greetings
           (box_id, storage_key, original_filename, duration_secs)
         VALUES (?, ?, ?, ?)",
    )
    .bind(id)
    .bind(&storage_key)
    .bind(original_filename.unwrap_or_else(|| "greeting.wav".to_string()))
    .bind(duration_secs)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "id": id,
        "username": username,
        "domain": domain,
        "storage_key": storage_key,
        "duration_secs": duration_secs,
    })))
}

/// `GET /api/voicemail/boxes/:id/greeting` — stream the WAV blob.
pub async fn download_greeting(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Response, (StatusCode, String)> {
    let row: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT username, domain, greeting_storage_key FROM sip_voicemail_boxes WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let (_username, _domain, key) =
        row.ok_or_else(|| (StatusCode::NOT_FOUND, "Voicemail box not found".to_string()))?;
    let key = key.ok_or_else(|| (StatusCode::NOT_FOUND, "No greeting uploaded".to_string()))?;

    let storage_dir = state.config.load().server.voicemail_storage_dir.clone();
    let storage = LocalVoicemailStorage::new(PathBuf::from(storage_dir));
    let bytes = storage
        .read(&key)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, format!("greeting missing: {e}")))?;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "audio/wav")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"greeting-{}.wav\"", id),
        )
        .body(Body::from(bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// `DELETE /api/voicemail/boxes/:id/greeting` — clear the box's greeting.
pub async fn delete_greeting(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT greeting_storage_key FROM sip_voicemail_boxes WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let key = row
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Voicemail box not found".to_string()))?
        .0;

    if let Some(key) = key {
        let storage_dir = state.config.load().server.voicemail_storage_dir.clone();
        let storage = LocalVoicemailStorage::new(PathBuf::from(storage_dir));
        let _ = storage.delete(&key).await;
    }
    sqlx::query("UPDATE sip_voicemail_boxes SET greeting_storage_key = NULL WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "id": id, "message": "Greeting cleared" })))
}
