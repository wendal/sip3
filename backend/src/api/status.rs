use axum::{
    Json,
    extract::State,
    http::{StatusCode, header},
    response::Response,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;
use crate::models::Registration;
use crate::sip::call_cleanup::mark_stale_calls_ended;

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

/// Forcibly remove a registration (admin deregister).
pub async fn delete_registration(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<u64>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let result = sqlx::query("DELETE FROM sip_registrations WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Registration not found".to_string()));
    }

    Ok(Json(json!({ "message": "Registration removed" })))
}

#[derive(Debug, Deserialize)]
pub struct CallsQuery {
    pub status: Option<String>,
    pub caller: Option<String>,
    pub callee: Option<String>,
    pub since: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

pub async fn list_calls(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<CallsQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let limit = q.limit.unwrap_or(100).min(500);
    let offset = q.offset.unwrap_or(0);

    // Build a dynamic WHERE clause for optional filters.
    let mut conditions: Vec<&str> = Vec::new();
    if q.status.is_some() {
        conditions.push("status = ?");
    }
    if q.caller.is_some() {
        conditions.push("caller LIKE ?");
    }
    if q.callee.is_some() {
        conditions.push("callee LIKE ?");
    }
    if q.since.is_some() {
        conditions.push("started_at >= ?");
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    // duration_secs: seconds from started_at to ended_at (NULL while in progress).
    let sql = format!(
        "SELECT id, call_id, caller, callee, status, started_at, answered_at, ended_at,
                TIMESTAMPDIFF(SECOND, started_at, COALESCE(ended_at, NOW())) AS duration_secs
         FROM sip_calls
         {}
         ORDER BY started_at DESC
         LIMIT ? OFFSET ?",
        where_clause
    );

    // sqlx doesn't support fully dynamic binding, so we build the query manually
    // by binding each optional param in turn.
    let mut query = sqlx::query_as::<_, CallWithDuration>(&sql);

    if let Some(ref s) = q.status {
        query = query.bind(s);
    }
    if let Some(ref s) = q.caller {
        query = query.bind(format!("%{}%", s));
    }
    if let Some(ref s) = q.callee {
        query = query.bind(format!("%{}%", s));
    }
    if let Some(ref s) = q.since {
        query = query.bind(s);
    }
    query = query.bind(limit).bind(offset);

    let calls: Vec<CallWithDuration> = query
        .fetch_all(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(
        json!({ "data": calls, "limit": limit, "offset": offset }),
    ))
}

/// CDR export: GET /api/calls?format=csv — streams a UTF-8 BOM + CSV body.
/// Honors the same filters as list_calls (status, caller, callee, since).
/// Hard cap: 100,000 rows to prevent runaway exports.
pub async fn export_calls(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<CallsQuery>,
) -> Result<Response, (StatusCode, String)> {
    const MAX_EXPORT_ROWS: u32 = 100_000;

    let mut conditions: Vec<&str> = Vec::new();
    if q.status.is_some() {
        conditions.push("status = ?");
    }
    if q.caller.is_some() {
        conditions.push("caller LIKE ?");
    }
    if q.callee.is_some() {
        conditions.push("callee LIKE ?");
    }
    if q.since.is_some() {
        conditions.push("started_at >= ?");
    }
    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    // Same columns as list_calls + the new hangup/sip_response/recording
    // columns added by 018_cdr_extras.sql.
    let sql = format!(
        "SELECT id, call_id, caller, callee, status, hangup_cause, sip_response_code,
                started_at, answered_at, ended_at,
                TIMESTAMPDIFF(SECOND, started_at, COALESCE(ended_at, NOW())) AS duration_secs,
                recording_key
         FROM sip_calls
         {}
         ORDER BY started_at DESC
         LIMIT {}",
        where_clause, MAX_EXPORT_ROWS
    );

    let mut query = sqlx::query_as::<_, CallExportRow>(&sql);
    if let Some(ref s) = q.status {
        query = query.bind(s);
    }
    if let Some(ref s) = q.caller {
        query = query.bind(format!("%{}%", s));
    }
    if let Some(ref s) = q.callee {
        query = query.bind(format!("%{}%", s));
    }
    if let Some(ref s) = q.since {
        query = query.bind(s);
    }

    let rows: Vec<CallExportRow> = query
        .fetch_all(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut wtr = csv::Writer::from_writer(Vec::<u8>::new());
    wtr.write_record([
        "call_id",
        "caller",
        "callee",
        "status",
        "hangup_cause",
        "sip_response_code",
        "started_at",
        "answered_at",
        "ended_at",
        "duration_secs",
        "recording_key",
    ])
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    for r in &rows {
        wtr.write_record(&[
            r.call_id.clone(),
            r.caller.clone(),
            r.callee.clone(),
            r.status.clone(),
            r.hangup_cause.clone().unwrap_or_default(),
            r.sip_response_code
                .map(|c| c.to_string())
                .unwrap_or_default(),
            r.started_at.format("%Y-%m-%d %H:%M:%S").to_string(),
            r.answered_at
                .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_default(),
            r.ended_at
                .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_default(),
            r.duration_secs.map(|d| d.to_string()).unwrap_or_default(),
            r.recording_key.clone().unwrap_or_default(),
        ])
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    let mut bytes = Vec::with_capacity(3 + wtr.get_ref().len());
    bytes.extend_from_slice(b"\xEF\xBB\xBF"); // UTF-8 BOM for Excel
    let body = wtr
        .into_inner()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    bytes.extend_from_slice(&body);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/csv; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"sip3-cdr.csv\"",
        )
        .body(axum::body::Body::from(bytes))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
struct CallExportRow {
    pub id: u64,
    pub call_id: String,
    pub caller: String,
    pub callee: String,
    pub status: String,
    pub hangup_cause: Option<String>,
    pub sip_response_code: Option<i16>,
    pub started_at: chrono::NaiveDateTime,
    pub answered_at: Option<chrono::NaiveDateTime>,
    pub ended_at: Option<chrono::NaiveDateTime>,
    pub duration_secs: Option<i64>,
    pub recording_key: Option<String>,
}

#[derive(Debug, serde::Serialize, sqlx::FromRow)]
struct CallWithDuration {
    pub id: u64,
    pub call_id: String,
    pub caller: String,
    pub callee: String,
    pub status: String,
    pub started_at: chrono::NaiveDateTime,
    pub answered_at: Option<chrono::NaiveDateTime>,
    pub ended_at: Option<chrono::NaiveDateTime>,
    pub duration_secs: Option<i64>,
}

/// Query params for `POST /api/calls/cleanup`.
#[derive(Debug, Deserialize)]
pub struct CleanupQuery {
    /// Close calls whose `started_at` is older than this many hours.
    /// Defaults to 4. Pass `0` to close every still-open call (the backend
    /// uses this on startup automatically).
    pub older_than_hours: Option<i64>,
}

/// Admin endpoint to mark stale active calls as ended.
///
/// Useful when the in-memory dialog state is out of sync with the DB
/// (process restarts, missing BYE/CANCEL, test leftovers). Returns the
/// number of rows updated.
pub async fn cleanup_calls(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<CleanupQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let hours = q.older_than_hours.unwrap_or(4);
    let threshold = if hours <= 0 { None } else { Some(hours) };

    let cleaned = mark_stale_calls_ended(&state.pool, threshold)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "cleaned": cleaned,
        "older_than_hours": hours,
    })))
}
