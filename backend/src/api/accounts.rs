use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use bcrypt::{hash, DEFAULT_COST};
use serde_json::{json, Value};

use super::AppState;
use crate::models::{Account, CreateAccount, UpdateAccount};

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, (StatusCode, String)> {
    let accounts: Vec<Account> = sqlx::query_as(
        "SELECT id, username, password_hash, ha1_hash, display_name, domain, enabled,
                created_at, updated_at
         FROM sip_accounts ORDER BY created_at DESC",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({ "data": accounts })))
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateAccount>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if body.username.is_empty() || body.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "username and password are required".to_string(),
        ));
    }

    let password_hash = hash(&body.password, DEFAULT_COST)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let realm = &state.config.auth.realm;
    let domain = body.domain.as_deref().unwrap_or(realm.as_str()).to_string();

    // Pre-compute HA1 for SIP Digest auth: MD5(username:realm:password)
    let ha1 = format!(
        "{:x}",
        md5::compute(format!("{}:{}:{}", body.username, realm, body.password).as_bytes())
    );

    let result = sqlx::query(
        "INSERT INTO sip_accounts (username, password_hash, ha1_hash, display_name, domain)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&body.username)
    .bind(&password_hash)
    .bind(&ha1)
    .bind(&body.display_name)
    .bind(&domain)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        let msg = e.to_string();
        if msg.contains("Duplicate") || msg.contains("1062") {
            (StatusCode::CONFLICT, "Username already exists".to_string())
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, msg)
        }
    })?;

    Ok(Json(
        json!({ "id": result.last_insert_id(), "message": "Account created" }),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Json(body): Json<UpdateAccount>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let row: Option<(String,)> = sqlx::query_as("SELECT username FROM sip_accounts WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (username,) = match row {
        Some(r) => r,
        None => return Err((StatusCode::NOT_FOUND, "Account not found".to_string())),
    };

    let realm = &state.config.auth.realm;

    if let Some(password) = &body.password {
        let new_hash = bcrypt::hash(password, DEFAULT_COST)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let ha1 = format!(
            "{:x}",
            md5::compute(format!("{}:{}:{}", username, realm, password).as_bytes())
        );
        sqlx::query("UPDATE sip_accounts SET password_hash = ?, ha1_hash = ? WHERE id = ?")
            .bind(&new_hash)
            .bind(&ha1)
            .bind(id)
            .execute(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    if let Some(display_name) = &body.display_name {
        sqlx::query("UPDATE sip_accounts SET display_name = ? WHERE id = ?")
            .bind(display_name)
            .bind(id)
            .execute(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    if let Some(domain) = &body.domain {
        sqlx::query("UPDATE sip_accounts SET domain = ? WHERE id = ?")
            .bind(domain)
            .bind(id)
            .execute(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    if let Some(enabled) = body.enabled {
        sqlx::query("UPDATE sip_accounts SET enabled = ? WHERE id = ?")
            .bind(enabled)
            .bind(id)
            .execute(&state.pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    Ok(Json(json!({ "message": "Account updated" })))
}

pub async fn delete_account(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let result = sqlx::query("DELETE FROM sip_accounts WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "Account not found".to_string()));
    }

    Ok(Json(json!({ "message": "Account deleted" })))
}
